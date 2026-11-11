//! 极米灯插件：UDS health/read/write/trigger + 私有 /devices + 云账号接入。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use olink_plugin_xgimi::cloud::{AccountSession, XgimiCloudClient};
use olink_plugin_xgimi::device_id::{make_device_id, supports_light, Channel, XgimiDeviceType};
use olink_plugin_xgimi::env::PluginEnv;
use olink_plugin_xgimi::host_ipc::HostGateway;
use olink_plugin_xgimi::lan::XgimiLan;
use olink_plugin_xgimi::lan_search::LanSearchRegistry;
use olink_plugin_xgimi::service::DeviceService;
use olink_plugin_xgimi::state::{DeviceConfig, DeviceStore};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, Notify};

type AppService = DeviceService<XgimiLan, HostGateway>;

#[derive(Clone)]
struct AppState {
    service: Arc<AppService>,
    data_dir: PathBuf,
    session: Arc<Mutex<AccountSession>>,
    shutdown: Arc<Notify>,
    lan_search: Arc<LanSearchRegistry>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = PluginEnv::from_env()?;
    let path = env.sock.clone();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let _ = fs::remove_file(&path);

    let shutdown = Arc::new(Notify::new());
    let host = HostGateway::new(env.host_sock.clone(), env.token.clone());
    let service = Arc::new(DeviceService {
        store: Arc::new(Mutex::new(DeviceStore::default())),
        lan: XgimiLan::new(),
        host: host.clone(),
        data_dir: env.data_dir.clone(),
        register_status: Arc::new(Mutex::new(Default::default())),
    });

    let mut sess = AccountSession::load(&env.data_dir);
    sess.ensure_device_id();
    let _ = sess.save(&env.data_dir);

    let lan_search = Arc::new(LanSearchRegistry::new(host.clone()));
    let app_state = AppState {
        service: service.clone(),
        data_dir: env.data_dir.clone(),
        session: Arc::new(Mutex::new(sess)),
        shutdown: shutdown.clone(),
        lan_search,
    };

    let app = Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route("/olink/v1/shutdown", post(shutdown_handler))
        .route("/olink/v1/read", post(read_handler))
        .route("/olink/v1/write", post(write_handler))
        .route("/olink/v1/trigger", post(trigger_handler))
        .route("/devices", get(list_devices).post(upsert_device))
        .route("/devices/lan/discover", get(lan_discover_devices))
        .route("/devices/lan/search", post(lan_search_start))
        .route("/devices/lan/search/{task_id}", get(lan_search_status))
        .route(
            "/devices/lan/search/{task_id}/cancel",
            post(lan_search_cancel),
        )
        .route("/devices/lan/import", post(lan_import_devices))
        .route("/devices/discover", get(discover_devices))
        .route("/devices/import", post(import_devices))
        .route("/devices/{id}/detail", get(device_detail))
        .route("/devices/{id}", delete(delete_device))
        .route("/account/status", get(account_status))
        .route("/account/sms", post(account_sms))
        .route("/account/login", post(account_login))
        .route("/account/logout", post(account_logout))
        .route("/account/refresh", post(account_refresh))
        .with_state(app_state.clone());

    let listener = UnixListener::bind(&path)?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    let serve = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    tokio::time::sleep(Duration::from_millis(30)).await;
    host.hello(&env.domain, &env.version).await?;
    service.load_store_from_disk(&env.data_dir).await;
    let devices = service.store.lock().await.devices.clone();
    for d in &devices {
        if let Err(e) = service.register_with_host(&d.id, &d.name).await {
            eprintln!("register_light {} failed: {e}", d.id);
        }
    }

    shutdown.notified().await;
    serve.abort();
    let _ = fs::remove_file(&path);
    Ok(())
}

async fn cloud_client(st: &AppState) -> anyhow::Result<XgimiCloudClient> {
    let sess = st.session.lock().await;
    XgimiCloudClient::new(sess.device_id.clone())
}

async fn shutdown_handler(State(st): State<AppState>) -> Json<Value> {
    st.shutdown.notify_waiters();
    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
struct ReadBody {
    device_key: String,
    #[serde(default)]
    attr_ids: Vec<String>,
    #[serde(default)]
    force: bool,
}

async fn read_handler(State(st): State<AppState>, Json(body): Json<ReadBody>) -> Json<Value> {
    Json(st.service.read(&body.device_key, &body.attr_ids, body.force).await)
}

#[derive(Debug, Deserialize)]
struct WriteBody {
    device_key: String,
    attr_id: String,
    value: Value,
}

async fn write_handler(State(st): State<AppState>, Json(body): Json<WriteBody>) -> Json<Value> {
    Json(
        st.service
            .write(&body.device_key, &body.attr_id, body.value)
            .await,
    )
}

#[derive(Debug, Deserialize)]
struct TriggerBody {
    job_id: String,
    #[serde(default)]
    manual: bool,
}

async fn trigger_handler(State(st): State<AppState>, Json(body): Json<TriggerBody>) -> Json<Value> {
    let _ = body.manual;
    if body.job_id != "refresh" {
        return Json(json!({ "ok": false, "message": "unknown job" }));
    }
    Json(st.service.refresh().await)
}

async fn list_devices(State(st): State<AppState>) -> Json<Value> {
    Json(st.service.list_devices().await)
}

async fn device_detail(State(st): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    Json(st.service.device_detail(&id).await)
}

#[derive(Debug, Deserialize)]
struct UpsertDeviceBody {
    id: String,
    name: String,
    host: String,
    #[serde(default)]
    secret: Option<String>,
}

async fn upsert_device(
    State(st): State<AppState>,
    Json(body): Json<UpsertDeviceBody>,
) -> Json<Value> {
    Json(
        st.service
            .upsert_device(DeviceConfig {
                id: body.id,
                name: body.name,
                host: body.host,
                secret: body.secret,
                pid: None,
                device_type: None,
                channel: None,
                product_name: None,
            })
            .await,
    )
}

async fn delete_device(State(st): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    Json(st.service.delete_device(&id).await)
}

async fn account_status(State(st): State<AppState>) -> Json<Value> {
    let sess = st.session.lock().await;
    Json(json!({
        "logged_in": sess.logged_in(),
        "mobile": if sess.mobile.is_empty() { Value::Null } else { json!(sess.mobile) },
        "uid": if sess.uid.is_empty() { Value::Null } else { json!(sess.uid) },
        "open_id": if sess.open_id.is_empty() { Value::Null } else { json!(sess.open_id) },
    }))
}

#[derive(Debug, Deserialize)]
struct SmsBody {
    mobile: String,
}

async fn account_sms(State(st): State<AppState>, Json(body): Json<SmsBody>) -> Json<Value> {
    let mobile = body.mobile.trim();
    if mobile.is_empty() {
        return Json(json!({ "ok": false, "message": "mobile required" }));
    }
    let client = match cloud_client(&st).await {
        Ok(c) => c,
        Err(e) => return Json(json!({ "ok": false, "message": format!("{e:#}") })),
    };
    match client.send_sms(mobile).await {
        Ok(()) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "ok": false, "message": format!("{e:#}") })),
    }
}

#[derive(Debug, Deserialize)]
struct LoginBody {
    mobile: String,
    code: String,
}

async fn account_login(State(st): State<AppState>, Json(body): Json<LoginBody>) -> Json<Value> {
    let mobile = body.mobile.trim();
    let code = body.code.trim();
    if mobile.is_empty() || code.is_empty() {
        return Json(json!({ "ok": false, "message": "mobile and code required" }));
    }
    let client = match cloud_client(&st).await {
        Ok(c) => c,
        Err(e) => return Json(json!({ "ok": false, "message": format!("{e:#}") })),
    };
    match client.login(mobile, code).await {
        Ok(result) => {
            {
                let mut sess = st.session.lock().await;
                *sess = result.session.clone();
                if let Err(e) = sess.save(&st.data_dir) {
                    return Json(json!({ "ok": false, "message": format!("save session: {e}") }));
                }
            }
            Json(json!({
                "ok": true,
                "uid": result.session.uid,
                "open_id": result.session.open_id,
                "mobile": result.session.mobile,
            }))
        }
        Err(e) => Json(json!({ "ok": false, "message": format!("{e:#}") })),
    }
}

async fn account_logout(State(st): State<AppState>) -> Json<Value> {
    let device_id = {
        let sess = st.session.lock().await;
        sess.device_id.clone()
    };
    let _ = AccountSession::clear(&st.data_dir);
    let mut next = AccountSession {
        device_id,
        ..Default::default()
    };
    next.ensure_device_id();
    let _ = next.save(&st.data_dir);
    *st.session.lock().await = next;
    Json(json!({ "ok": true }))
}

async fn account_refresh(State(st): State<AppState>) -> Json<Value> {
    let (client, sess) = match cloud_client(&st).await {
        Ok(c) => {
            let s = st.session.lock().await.clone();
            (c, s)
        }
        Err(e) => return Json(json!({ "ok": false, "message": format!("{e:#}") })),
    };
    match client.refresh(&sess).await {
        Ok(next) => {
            {
                let mut guard = st.session.lock().await;
                *guard = next.clone();
                if let Err(e) = guard.save(&st.data_dir) {
                    return Json(json!({ "ok": false, "message": format!("save session: {e}") }));
                }
            }
            Json(json!({ "ok": true }))
        }
        Err(e) => Json(json!({ "ok": false, "message": format!("{e:#}") })),
    }
}

async fn discover_devices(State(st): State<AppState>) -> Json<Value> {
    let sess = st.session.lock().await.clone();
    if !sess.logged_in() {
        return Json(json!({ "ok": false, "message": "not logged in" }));
    }
    let client = match XgimiCloudClient::new(sess.device_id.clone()) {
        Ok(c) => c,
        Err(e) => return Json(json!({ "ok": false, "message": format!("{e:#}") })),
    };
    match client.list_devices(&sess).await {
        Ok(list) => {
            let local = st.service.store.lock().await;
            let devices: Vec<Value> = list
                .into_iter()
                .map(|d| {
                    let id = make_device_id(&d.sn, XgimiDeviceType::Light, Channel::Cloud);
                    let imported = local.get(&id).is_some();
                    json!({
                        "sn": d.sn,
                        "id": id,
                        "name": d.name,
                        "item_code": d.item_code,
                        "product_sn": d.product_sn,
                        "xgimi_product_name": d.xgimi_product_name,
                        "imported": imported,
                    })
                })
                .collect();
            Json(json!({ "ok": true, "devices": devices }))
        }
        Err(e) => Json(json!({ "ok": false, "message": format!("{e:#}") })),
    }
}

#[derive(Debug, Deserialize)]
struct ImportItem {
    sn: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    item_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImportBody {
    items: Vec<ImportItem>,
}

async fn import_devices(State(st): State<AppState>, Json(body): Json<ImportBody>) -> Json<Value> {
    let sess = st.session.lock().await.clone();
    if !sess.logged_in() {
        return Json(json!({ "ok": false, "message": "not logged in" }));
    }
    if body.items.is_empty() {
        return Json(json!({ "ok": false, "message": "items required" }));
    }
    let mut imported = Vec::new();
    let mut errors = Vec::new();
    for item in body.items {
        let id = make_device_id(&item.sn, XgimiDeviceType::Light, Channel::Cloud);
        let name = if item.name.trim().is_empty() {
            item.sn.clone()
        } else {
            item.name.trim().to_string()
        };
        let secret = item.item_code.filter(|s| !s.is_empty());
        let r = st
            .service
            .import_device(DeviceConfig {
                id: id.clone(),
                name,
                host: String::new(),
                secret,
                pid: Some(item.sn),
                device_type: Some("light".into()),
                channel: Some("cloud".into()),
                product_name: None,
            })
            .await;
        if r.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            imported.push(id);
        } else {
            errors.push(json!({
                "id": id,
                "message": r.get("message").cloned().unwrap_or(json!("failed")),
            }));
        }
    }
    Json(json!({
        "ok": errors.is_empty(),
        "imported": imported,
        "errors": errors,
    }))
}

async fn lan_discover_devices(State(st): State<AppState>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(|| {
        olink_plugin_xgimi::lan::discover_devices(Duration::from_secs(4))
    })
    .await;
    let list = match result {
        Ok(Ok(list)) => list,
        Ok(Err(e)) => return Json(json!({ "ok": false, "message": e })),
        Err(e) => return Json(json!({ "ok": false, "message": format!("discover join: {e}") })),
    };
    let local = st.service.store.lock().await;
    let devices: Vec<Value> = list
        .into_iter()
        .map(|d| {
            let id = make_device_id(&d.pid, XgimiDeviceType::Light, Channel::Lan);
            let imported = local.get(&id).is_some();
            json!({
                "pid": d.pid,
                "name": d.name,
                "host": d.host,
                "product_name": d.product_name,
                "device": d.device,
                "id": id,
                "imported": imported,
                "support": { "light": d.support_light, "projector": false },
            })
        })
        .collect();
    Json(json!({ "ok": true, "devices": devices }))
}

async fn lan_search_start(State(st): State<AppState>) -> Json<Value> {
    match st.lan_search.start().await {
        Ok(task_id) => Json(json!({ "ok": true, "task_id": task_id })),
        Err(e) => Json(json!({ "ok": false, "message": e })),
    }
}

async fn lan_search_status(
    State(st): State<AppState>,
    Path(task_id): Path<String>,
) -> Json<Value> {
    let Some(snap) = st.lan_search.snapshot(&task_id).await else {
        return Json(json!({ "ok": false, "message": "任务不存在或已过期" }));
    };
    let local = st.service.store.lock().await;
    let devices: Vec<Value> = snap
        .devices
        .into_iter()
        .map(|d| {
            let id = make_device_id(&d.pid, XgimiDeviceType::Light, Channel::Lan);
            let imported = local.get(&id).is_some();
            json!({
                "pid": d.pid,
                "name": d.name,
                "host": d.host,
                "product_name": d.product_name,
                "device": d.device,
                "id": id,
                "imported": imported,
                "support": { "light": d.support_light, "projector": false },
            })
        })
        .collect();
    Json(json!({
        "ok": true,
        "task_id": snap.task_id,
        "phase": snap.phase.as_str(),
        "message": snap.message,
        "progress_percent": snap.progress_percent,
        "devices": devices,
    }))
}

async fn lan_search_cancel(
    State(st): State<AppState>,
    Path(task_id): Path<String>,
) -> Json<Value> {
    let ok = st.lan_search.cancel(&task_id).await;
    Json(json!({ "ok": ok }))
}

#[derive(Debug, Deserialize)]
struct LanImportItem {
    pid: String,
    #[serde(default)]
    name: String,
    host: String,
    #[serde(default)]
    product_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LanImportBody {
    items: Vec<LanImportItem>,
}

async fn lan_import_devices(
    State(st): State<AppState>,
    Json(body): Json<LanImportBody>,
) -> Json<Value> {
    if body.items.is_empty() {
        return Json(json!({ "ok": false, "message": "items required" }));
    }
    let mut imported = Vec::new();
    let mut errors = Vec::new();
    for item in body.items {
        let host = item.host.trim().to_string();
        if host.is_empty() {
            errors.push(json!({
                "id": Value::Null,
                "message": "host required",
            }));
            continue;
        }
        let name = if item.name.trim().is_empty() {
            item.pid.clone()
        } else {
            item.name.trim().to_string()
        };
        let product_name = item
            .product_name
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if !supports_light(product_name.as_deref(), None, Some(name.as_str())) {
            errors.push(json!({
                "id": make_device_id(&item.pid, XgimiDeviceType::Light, Channel::Lan),
                "message": "light not supported",
            }));
            continue;
        }
        let id = make_device_id(&item.pid, XgimiDeviceType::Light, Channel::Lan);
        let r = st
            .service
            .import_device(DeviceConfig {
                id: id.clone(),
                name,
                host,
                secret: None,
                pid: Some(item.pid),
                device_type: Some("light".into()),
                channel: Some("lan".into()),
                product_name,
            })
            .await;
        if r.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            imported.push(id);
        } else {
            errors.push(json!({
                "id": id,
                "message": r.get("message").cloned().unwrap_or(json!("failed")),
            }));
        }
    }
    Json(json!({
        "ok": errors.is_empty(),
        "imported": imported,
        "errors": errors,
    }))
}
