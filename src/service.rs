//! Device orchestration: read / write / refresh / CRUD via LanBackend + HostReporter.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::attr::{lookup_attr, report_triple, values_from_state};
use crate::host_ipc::HostGateway;
use crate::lan::LanBackend;
use crate::light_decl::{self, DEFAULT_MODEL, MANUFACTURER, SCHEMA_ID};
use crate::state::{DeviceConfig, DeviceStore};

#[async_trait]
pub trait HostReporter: Send + Sync {
    async fn report_attrs(&self, device_key: &str, attrs: &[(&str, Value)]) -> anyhow::Result<()>;
    async fn register_light(&self, device_key: &str, name: &str, model: &str) -> anyhow::Result<()>;
}

#[async_trait]
impl HostReporter for HostGateway {
    async fn report_attrs(&self, device_key: &str, attrs: &[(&str, Value)]) -> anyhow::Result<()> {
        HostGateway::report_attrs(self, device_key, attrs).await
    }
    async fn register_light(&self, device_key: &str, name: &str, model: &str) -> anyhow::Result<()> {
        HostGateway::register_light(self, device_key, name, model).await
    }
}

pub struct DeviceService<L, H> {
    pub store: Arc<Mutex<DeviceStore>>,
    pub lan: L,
    pub host: H,
    pub data_dir: PathBuf,
    /// 最近一次 Host register 结果（进程内；不二次查询 Host）。
    pub register_status: Arc<Mutex<HashMap<String, Result<(), String>>>>,
}

impl<L: LanBackend, H: HostReporter> DeviceService<L, H> {
    pub async fn read(&self, device_key: &str, attr_ids: &[String], _force: bool) -> Value {
        let host = {
            let store = self.store.lock().await;
            match store.get(device_key) {
                Some(d) => d.host.clone(),
                None => return json!({ "ok": false, "message": "unknown device" }),
            }
        };
        let lamp = match self.lan.read_all(&host) {
            Ok(s) => s,
            Err(e) => return json!({ "ok": false, "message": e }),
        };
        json!({ "ok": true, "values": values_from_state(&lamp, attr_ids) })
    }

    pub async fn write(&self, device_key: &str, attr_id: &str, value: Value) -> Value {
        let host = {
            let store = self.store.lock().await;
            match store.get(device_key) {
                Some(d) => d.host.clone(),
                None => return json!({ "ok": false, "message": "unknown device" }),
            }
        };
        let Some(attr) = lookup_attr(attr_id) else {
            return json!({ "ok": false, "message": "unknown attr" });
        };
        let applied = match attr.parse(&value) {
            Ok(v) => v,
            Err(e) => return json!({ "ok": false, "message": e }),
        };
        if let Err(e) = attr.apply(&self.lan as &dyn LanBackend, &host, &applied) {
            return json!({ "ok": false, "message": e });
        }
        let _ = self
            .host
            .report_attrs(device_key, &[(attr.id(), applied.clone())])
            .await;
        json!({ "ok": true, "applied_value": applied })
    }

    pub async fn refresh(&self) -> Value {
        let devices = self.store.lock().await.devices.clone();
        for d in devices {
            let lamp = match self.lan.read_all(&d.host) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("refresh read {} failed: {e}", d.id);
                    continue;
                }
            };
            let attrs = report_triple(&lamp);
            if let Err(e) = self.host.report_attrs(&d.id, &attrs).await {
                eprintln!("refresh report {} failed: {e:#}", d.id);
            }
        }
        json!({ "ok": true })
    }

    pub fn valid_device_id(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }

    pub async fn list_devices(&self) -> Value {
        let store = self.store.lock().await;
        let mut devices = Vec::new();
        for d in &store.devices {
            let lamp = self.lan.read_all(&d.host).unwrap_or_default();
            let mut obj = json!({
                "id": d.id,
                "name": d.name,
                "host": d.host,
                "on": lamp.on,
                "brightness": lamp.brightness,
                "color_temp_k": lamp.color_temp_k,
                "pid": d.pid,
                "device_type": d.device_type,
                "channel": d.channel,
                "product_name": d.product_name,
            });
            if let Some(secret) = &d.secret {
                obj.as_object_mut()
                    .unwrap()
                    .insert("secret".into(), json!(secret));
            }
            devices.push(obj);
        }
        json!({ "devices": devices })
    }

    pub async fn upsert_device(&self, cfg: DeviceConfig) -> Value {
        if !Self::valid_device_id(&cfg.id) {
            return json!({ "ok": false, "message": "invalid id" });
        }
        if cfg.host.trim().is_empty() {
            return json!({ "ok": false, "message": "host required" });
        }
        self.persist_and_register(cfg).await
    }

    /// 导入（云可空 host；LAN 应带 host）。校验 id 后持久化并 register。
    pub async fn import_device(&self, cfg: DeviceConfig) -> Value {
        if !Self::valid_device_id(&cfg.id) {
            return json!({ "ok": false, "message": "invalid id" });
        }
        self.persist_and_register(cfg).await
    }

    async fn persist_and_register(&self, cfg: DeviceConfig) -> Value {
        {
            let mut store = self.store.lock().await;
            store.upsert(cfg.clone());
            if let Err(e) = store.save(&self.data_dir) {
                return json!({ "ok": false, "message": format!("save failed: {e}") });
            }
        }
        if let Err(e) = self.register_with_host(&cfg.id, &cfg.name).await {
            return json!({ "ok": false, "message": format!("register failed: {e}") });
        }
        json!({ "ok": true })
    }

    pub async fn register_with_host(&self, id: &str, name: &str) -> Result<(), String> {
        match self.host.register_light(id, name, "").await {
            Ok(()) => {
                self.register_status.lock().await.insert(id.to_string(), Ok(()));
                Ok(())
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.register_status
                    .lock()
                    .await
                    .insert(id.to_string(), Err(msg.clone()));
                Err(msg)
            }
        }
    }

    pub async fn device_detail(&self, id: &str) -> Value {
        let cfg = {
            let store = self.store.lock().await;
            match store.get(id) {
                Some(d) => d.clone(),
                None => return json!({ "ok": false, "message": "unknown device" }),
            }
        };
        let status = self.register_status.lock().await.get(id).cloned();
        let (host_registered, register_error) = match status {
            Some(Ok(())) => (true, Value::Null),
            Some(Err(e)) => (false, json!(e)),
            None => (false, Value::Null),
        };
        json!({
            "ok": true,
            "id": cfg.id,
            "name": cfg.name,
            "did": format!("xgimi.{}", cfg.id),
            "host": cfg.host,
            "model": DEFAULT_MODEL,
            "manufacturer": MANUFACTURER,
            "schema_id": SCHEMA_ID,
            "host_registered": host_registered,
            "register_error": register_error,
            "services": light_decl::light_services_for_detail(),
            "pid": cfg.pid,
            "device_type": cfg.device_type,
            "channel": cfg.channel,
            "product_name": cfg.product_name,
        })
    }

    pub async fn delete_device(&self, id: &str) -> Value {
        let mut store = self.store.lock().await;
        let _ = store.remove(id);
        if let Err(e) = store.save(&self.data_dir) {
            return json!({ "ok": false, "message": format!("save failed: {e}") });
        }
        self.register_status.lock().await.remove(id);
        json!({ "ok": true })
    }

    pub async fn load_store_from_disk(&self, data_dir: &Path) {
        let loaded = DeviceStore::load(data_dir);
        *self.store.lock().await = loaded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::StubLan;
    use crate::state::{DeviceConfig, DeviceStore};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    struct NoopHost {
        reports: Arc<Mutex<Vec<(String, Vec<(String, Value)>)>>>,
    }

    #[async_trait]
    impl HostReporter for NoopHost {
        async fn report_attrs(
            &self,
            device_key: &str,
            attrs: &[(&str, Value)],
        ) -> anyhow::Result<()> {
            let owned: Vec<_> = attrs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect();
            self.reports
                .lock()
                .unwrap()
                .push((device_key.to_string(), owned));
            Ok(())
        }
        async fn register_light(
            &self,
            _device_key: &str,
            _name: &str,
            _model: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn svc(host: NoopHost) -> DeviceService<StubLan, NoopHost> {
        let mut store = DeviceStore::default();
        store.upsert(DeviceConfig {
            id: "dev1".into(),
            name: "灯".into(),
            host: "192.168.1.50".into(),
            secret: None,
            pid: None,
            device_type: None,
            channel: None,
            product_name: None,
        });
        DeviceService {
            store: Arc::new(tokio::sync::Mutex::new(store)),
            lan: StubLan::default(),
            host,
            data_dir: std::env::temp_dir().join("xgimi-svc-test"),
            register_status: Arc::new(tokio::sync::Mutex::new(Default::default())),
        }
    }

    #[tokio::test]
    async fn read_unknown_device() {
        let s = svc(NoopHost {
            reports: Arc::new(Mutex::new(vec![])),
        });
        let r = s.read("nope", &[], false).await;
        assert_eq!(r["ok"], false);
    }

    #[tokio::test]
    async fn write_power_and_report() {
        let reports = Arc::new(Mutex::new(vec![]));
        let s = svc(NoopHost {
            reports: reports.clone(),
        });
        let r = s.write("dev1", "power-state", json!(true)).await;
        assert_eq!(r["ok"], true);
        assert_eq!(r["applied_value"], true);
        assert_eq!(reports.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refresh_reports_triple() {
        let reports = Arc::new(Mutex::new(vec![]));
        let s = svc(NoopHost {
            reports: reports.clone(),
        });
        let r = s.refresh().await;
        assert_eq!(r["ok"], true);
        let lock = reports.lock().unwrap();
        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].1.len(), 3);
    }

    #[tokio::test]
    async fn import_device_allows_empty_host() {
        let dir = tempfile::tempdir().unwrap();
        let s = DeviceService {
            store: Arc::new(tokio::sync::Mutex::new(DeviceStore::default())),
            lan: StubLan::default(),
            host: NoopHost {
                reports: Arc::new(Mutex::new(vec![])),
            },
            data_dir: dir.path().to_path_buf(),
            register_status: Arc::new(tokio::sync::Mutex::new(Default::default())),
        };
        let r = s
            .import_device(DeviceConfig {
                id: "SWX5_light_cloud".into(),
                name: "云神灯".into(),
                host: String::new(),
                secret: Some("ICM".into()),
                pid: Some("SWX5".into()),
                device_type: Some("light".into()),
                channel: Some("cloud".into()),
                product_name: None,
            })
            .await;
        assert_eq!(r["ok"], true);
        let stored = s.store.lock().await.get("SWX5_light_cloud").cloned().unwrap();
        assert!(stored.host.is_empty());
        assert_eq!(stored.channel.as_deref(), Some("cloud"));
    }
}
