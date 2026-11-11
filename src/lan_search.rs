//! 局域网搜索会话：10s 循环 UDP discover，挂 Host AsyncTask。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::host_ipc::HostGateway;
use crate::lan::{discover_devices, DiscoveredLanDevice};

pub const SEARCH_WALL_SECS: u64 = 10;
pub const DISCOVER_TIMEOUT_SECS: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPhase {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SearchPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct SearchSnapshot {
    pub task_id: String,
    pub phase: SearchPhase,
    pub message: String,
    pub progress_percent: Option<u32>,
    pub devices: Vec<DiscoveredLanDevice>,
}

struct SessionInner {
    write_token: String,
    phase: SearchPhase,
    message: String,
    progress_percent: Option<u32>,
    devices: BTreeMap<String, DiscoveredLanDevice>,
    cancel: Arc<AtomicBool>,
}

/// 按 pid 合并发现结果（同 pid 用最新覆盖）。
pub fn merge_into(map: &mut BTreeMap<String, DiscoveredLanDevice>, d: DiscoveredLanDevice) {
    map.insert(d.pid.clone(), d);
}

pub struct LanSearchRegistry {
    host: HostGateway,
    sessions: Mutex<BTreeMap<String, SessionInner>>,
}

impl LanSearchRegistry {
    pub fn new(host: HostGateway) -> Self {
        Self {
            host,
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn start(self: &Arc<Self>) -> Result<String, String> {
        let (task_id, write_token) = self
            .host
            .create_task(
                "lan_search",
                "局域网发现",
                1,
                Some(300),
                Some(30),
            )
            .await
            .map_err(|e| e.to_string())?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut guard = self.sessions.lock().await;
            guard.insert(
                task_id.clone(),
                SessionInner {
                    write_token: write_token.clone(),
                    phase: SearchPhase::Pending,
                    message: "准备搜索".into(),
                    progress_percent: Some(0),
                    devices: BTreeMap::new(),
                    cancel: cancel.clone(),
                },
            );
        }
        let reg = Arc::clone(self);
        let tid = task_id.clone();
        tokio::spawn(async move {
            reg.run_loop(tid, write_token, cancel).await;
        });
        Ok(task_id)
    }

    pub async fn snapshot(&self, task_id: &str) -> Option<SearchSnapshot> {
        let guard = self.sessions.lock().await;
        let s = guard.get(task_id)?;
        Some(SearchSnapshot {
            task_id: task_id.to_string(),
            phase: s.phase,
            message: s.message.clone(),
            progress_percent: s.progress_percent,
            devices: s.devices.values().cloned().collect(),
        })
    }

    pub async fn cancel(&self, task_id: &str) -> bool {
        let (write_token, cancel_flag) = {
            let mut guard = self.sessions.lock().await;
            let Some(s) = guard.get_mut(task_id) else {
                return false;
            };
            if s.phase.is_terminal() {
                return false;
            }
            s.cancel.store(true, Ordering::SeqCst);
            s.phase = SearchPhase::Cancelled;
            s.message = "已取消".into();
            (s.write_token.clone(), true)
        };
        if cancel_flag {
            let _ = self
                .host
                .patch_task(
                    task_id,
                    &write_token,
                    json!({
                        "phase": "cancelled",
                        "message": "已取消",
                    }),
                )
                .await;
        }
        true
    }

    async fn run_loop(self: Arc<Self>, task_id: String, write_token: String, cancel: Arc<AtomicBool>) {
        let wall = Duration::from_secs(SEARCH_WALL_SECS);
        let started = Instant::now();
        {
            let mut guard = self.sessions.lock().await;
            if let Some(s) = guard.get_mut(&task_id) {
                s.phase = SearchPhase::Running;
                s.message = "搜索中…".into();
            }
        }
        let _ = self
            .host
            .patch_task(
                &task_id,
                &write_token,
                json!({
                    "phase": "running",
                    "message": "搜索中…",
                    "progress_percent": 0,
                }),
            )
            .await;

        while started.elapsed() < wall {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            let discover_timeout = Duration::from_secs(DISCOVER_TIMEOUT_SECS);
            let found = tokio::task::spawn_blocking(move || discover_devices(discover_timeout))
                .await;
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            match found {
                Ok(Ok(list)) => {
                    let mut guard = self.sessions.lock().await;
                    if let Some(s) = guard.get_mut(&task_id) {
                        if s.phase == SearchPhase::Cancelled {
                            return;
                        }
                        for d in list {
                            merge_into(&mut s.devices, d);
                        }
                        let pct = ((started.elapsed().as_secs_f32() / wall.as_secs_f32()) * 100.0)
                            .clamp(0.0, 99.0) as u32;
                        s.progress_percent = Some(pct);
                        s.message = format!("已发现 {} 台", s.devices.len());
                        s.phase = SearchPhase::Running;
                        let msg = s.message.clone();
                        drop(guard);
                        let _ = self
                            .host
                            .patch_task(
                                &task_id,
                                &write_token,
                                json!({
                                    "phase": "running",
                                    "message": msg,
                                    "progress_percent": pct,
                                }),
                            )
                            .await;
                    }
                }
                Ok(Err(e)) => {
                    let mut guard = self.sessions.lock().await;
                    if let Some(s) = guard.get_mut(&task_id) {
                        if s.phase.is_terminal() {
                            return;
                        }
                        s.message = format!("发现异常: {e}");
                    }
                }
                Err(e) => {
                    let mut guard = self.sessions.lock().await;
                    if let Some(s) = guard.get_mut(&task_id) {
                        if s.phase.is_terminal() {
                            return;
                        }
                        s.message = format!("discover join: {e}");
                    }
                }
            }
        }

        if cancel.load(Ordering::SeqCst) {
            return;
        }

        let (devices_json, count) = {
            let mut guard = self.sessions.lock().await;
            let Some(s) = guard.get_mut(&task_id) else {
                return;
            };
            if s.phase == SearchPhase::Cancelled {
                return;
            }
            s.phase = SearchPhase::Completed;
            s.progress_percent = Some(100);
            s.message = format!("搜索结束 · {} 台", s.devices.len());
            let devices: Vec<Value> = s
                .devices
                .values()
                .map(|d| {
                    json!({
                        "pid": d.pid,
                        "name": d.name,
                        "host": d.host,
                        "product_name": d.product_name,
                        "device": d.device,
                        "support_light": d.support_light,
                    })
                })
                .collect();
            let count = devices.len();
            (devices, count)
        };

        let _ = self
            .host
            .patch_task(
                &task_id,
                &write_token,
                json!({
                    "phase": "completed",
                    "message": format!("搜索结束 · {count} 台"),
                    "progress_percent": 100,
                    "current": 1,
                    "result": { "devices": devices_json },
                }),
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: &str, host: &str) -> DiscoveredLanDevice {
        DiscoveredLanDevice {
            pid: pid.into(),
            name: pid.into(),
            host: host.into(),
            product_name: None,
            device: None,
            support_light: true,
        }
    }

    #[test]
    fn merge_devices_by_pid_overwrites_host() {
        let mut map = BTreeMap::new();
        merge_into(&mut map, sample("A", "1.1.1.1"));
        merge_into(&mut map, sample("A", "1.1.1.2"));
        assert_eq!(map.get("A").unwrap().host, "1.1.1.2");
        assert_eq!(map.len(), 1);
    }
}
