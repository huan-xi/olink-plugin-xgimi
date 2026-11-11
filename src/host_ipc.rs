//! Host UDS JSON-RPC client: hello / register / report / available.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Request;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::UnixStream;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn uds_json(
    sock: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> anyhow::Result<(u16, String)> {
    let stream = UnixStream::connect(sock).await?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let bytes = match body {
        Some(b) => Bytes::from(serde_json::to_vec(b)?),
        None => Bytes::new(),
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("X-Plugin-Token", token);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let req = builder.body(Full::new(bytes))?;
    let resp = sender.send_request(req).await?;
    let status = resp.status().as_u16();
    let collected = http_body_util::BodyExt::collect(resp.into_body()).await?;
    let text = String::from_utf8_lossy(&collected.to_bytes()).into_owned();
    Ok((status, text))
}

#[derive(Clone)]
pub struct HostGateway {
    pub host_sock: String,
    pub token: String,
}

impl HostGateway {
    pub fn new(host_sock: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            host_sock: host_sock.into(),
            token: token.into(),
        }
    }

    pub async fn hello(&self, domain: &str, version: &str) -> anyhow::Result<()> {
        let body = json!({
            "domain": domain,
            "plugin_version": version,
            "protocol": 1,
            "features": ["device.register", "device.attrs", "schedules"],
            "features_required": ["device.register", "device.attrs"],
        });
        let (status, text) =
            uds_json(&self.host_sock, "POST", "/olink/v1/hello", &self.token, Some(&body)).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!("hello status={status} body={text}");
        }
        Ok(())
    }

    /// 注册灯光设备
    pub async fn register_light(
        &self,
        device_key: &str,
        name: &str,
        model: &str,
    ) -> anyhow::Result<()> {
        let body = crate::light_decl::register_body(device_key, name, model);
        let (status, text) =
            uds_json(&self.host_sock, "POST", "/olink/v1/register", &self.token, Some(&body)).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!("register status={status} body={text}");
        }
        Ok(())
    }

    pub async fn report_attrs(
        &self,
        device_key: &str,
        attrs: &[(&str, Value)],
    ) -> anyhow::Result<()> {
        let ts = now_ms();
        let reports: Vec<Value> = attrs
            .iter()
            .map(|(attr_id, value)| {
                json!({
                    "device_key": device_key,
                    "attr_id": attr_id,
                    "value": value,
                    "ts": ts,
                })
            })
            .collect();
        let body = json!({ "reports": reports });
        let (status, text) =
            uds_json(&self.host_sock, "POST", "/olink/v1/report", &self.token, Some(&body)).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!("report status={status} body={text}");
        }
        Ok(())
    }

    pub async fn set_available(
        &self,
        device_key: &str,
        available: bool,
    ) -> anyhow::Result<()> {
        let body = json!({
            "device_key": device_key,
            "available": available,
        });
        let (status, text) =
            uds_json(&self.host_sock, "POST", "/olink/v1/available", &self.token, Some(&body))
                .await?;
        if !(200..300).contains(&status) {
            anyhow::bail!("available status={status} body={text}");
        }
        Ok(())
    }

    /// Host AsyncTaskRegistry：创建任务，返回 `(task_id, write_token)`。
    pub async fn create_task(
        &self,
        kind: &str,
        title: &str,
        total: u32,
        ttl_secs: Option<u64>,
        timeout_secs: Option<u64>,
    ) -> anyhow::Result<(String, String)> {
        let body = json!({
            "kind": kind,
            "title": title,
            "total": total,
            "ttl_secs": ttl_secs,
            "timeout_secs": timeout_secs,
        });
        let (status, text) =
            uds_json(&self.host_sock, "POST", "/olink/v1/tasks", &self.token, Some(&body)).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!("create_task status={status} body={text}");
        }
        let v: Value = serde_json::from_str(&text)?;
        let task_id = v
            .get("task_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let write_token = v
            .get("write_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if task_id.is_empty() || write_token.is_empty() {
            anyhow::bail!("create_task missing ids: {text}");
        }
        Ok((task_id, write_token))
    }

    /// 用 write_token 回写任务进度/结果（body 可含 phase/message/result/…）。
    pub async fn patch_task(
        &self,
        task_id: &str,
        write_token: &str,
        mut patch: Value,
    ) -> anyhow::Result<()> {
        if let Some(obj) = patch.as_object_mut() {
            obj.insert("write_token".into(), json!(write_token));
        }
        let path = format!("/olink/v1/tasks/{task_id}");
        let (status, text) =
            uds_json(&self.host_sock, "PATCH", &path, &self.token, Some(&patch)).await?;
        if !(200..300).contains(&status) {
            anyhow::bail!("patch_task status={status} body={text}");
        }
        Ok(())
    }
}
