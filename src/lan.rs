//! 极米 CP01 局域网协议（TCP 13145，`E0 4A` 帧 + JSON 信封）。
//!
//! 协议细节与实测记录见
//! `docs/reference/2026-11-09-xgimi-cp01-lan-protocol.md`。
//! `StubLan` 保留给单测与无设备环境。

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::device_id::supports_light;
use crate::map::{
    brightness_level_from_schema, brightness_schema_from_level, clamp_brightness, clamp_color_temp,
    color_temp_level_from_schema, color_temp_schema_from_level, DEVICE_LEVEL_MAX,
};

/// 未声明端口时的默认控制通道（实测 CP01 的 `agreement.port`）。
pub const DEFAULT_TCP_PORT: u16 = 13145;
/// 帧头魔数。
pub const FRAME_MAGIC: [u8; 2] = [0xE0, 0x4A];

const MAX_BODY: usize = 1 << 20;
const APP_ID: &str = "com.xgimi.XAssistant";
const APP_VERSION: &str = "4.10.36";

/// UDP 发现广播端口（App→设备）。
pub const UDP_DISCOVER_PORT: u16 = 16750;
/// UDP 发现回包端口（设备→本机）。
pub const UDP_REPLY_PORT: u16 = 16751;

/// 鉴权（可省，设备回 50000）。
pub const ACTION_AUTH: u32 = 10000;
/// 心跳，设备回 10003。
pub const ACTION_HEARTBEAT: u32 = 10002;
/// LAN 发现请求 / 应答。
pub const ACTION_DISCOVER: u32 = 9998;
pub const ACTION_DISCOVER_REPLY: u32 = 9999;
/// 读灯状态。
pub const ACTION_GET_LIGHT: u32 = 50040;
/// 写灯状态。
pub const ACTION_SET_LIGHT: u32 = 50041;

/// 灯控子项（`commonCmd.type`）。
pub const TYPE_SWITCH: &str = "0";
pub const TYPE_COLOR_TEMP: &str = "1";
pub const TYPE_BRIGHTNESS: &str = "2";
pub const TYPE_MODE: &str = "3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LampState {
    pub on: bool,
    /// schema 亮度 0–100。
    pub brightness: u8,
    /// schema 色温 2000–6500 K。
    pub color_temp_k: u32,
}

impl Default for LampState {
    fn default() -> Self {
        Self {
            on: false,
            brightness: 100,
            color_temp_k: 4000,
        }
    }
}

pub trait LanBackend: Send + Sync {
    fn read_all(&self, host: &str) -> Result<LampState, String>;
    fn write_power(&self, host: &str, on: bool) -> Result<(), String>;
    fn write_brightness(&self, host: &str, bri: u8) -> Result<(), String>;
    fn write_color_temp(&self, host: &str, k: u32) -> Result<(), String>;

    /// 最近一次已知状态，供列表页免阻塞展示。
    fn cached(&self, _host: &str) -> Option<LampState> {
        None
    }
}

/// UDP 9999 发现应答解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredLanDevice {
    pub pid: String,
    pub name: String,
    pub host: String,
    pub product_name: Option<String>,
    pub device: Option<String>,
    pub support_light: bool,
}

/// 从 `action==9999` JSON 抽出设备字段；缺 `pid` 则失败。
pub fn parse_discover_reply(raw: &Value) -> Result<DiscoveredLanDevice, String> {
    let info = raw
        .get("deviceInfo")
        .ok_or_else(|| "discover reply missing deviceInfo".to_string())?;
    let pid = info
        .get("pid")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "discover reply missing pid".to_string())?
        .to_string();
    let device = info
        .get("device")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let name = info
        .get("devicename")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| device.clone())
        .unwrap_or_else(|| pid.clone());
    let host = info
        .get("deviceip")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "discover reply missing deviceip".to_string())?
        .to_string();
    let product_name = info
        .get("productName")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let support_light = supports_light(
        product_name.as_deref(),
        device.as_deref(),
        Some(name.as_str()),
    );
    Ok(DiscoveredLanDevice {
        pid,
        name,
        host,
        product_name,
        device,
        support_light,
    })
}

/// UDP 广播发现：绑定 `:16751` 收 9999，向 `255.255.255.255:16750` 发 9998。
///
/// 绑定失败返回 `Err`。`SO_REUSEADDR` / macOS `SO_REUSEPORT` 在 `bind_reply_socket` 内设置。
pub fn discover_devices(timeout: Duration) -> Result<Vec<DiscoveredLanDevice>, String> {
    let listen = bind_reply_socket()?;
    let poll = Duration::from_millis(200);
    listen
        .set_read_timeout(Some(poll))
        .map_err(|e| format!("set read timeout: {e}"))?;

    let tx = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| format!("bind send sock: {e}"))?;
    tx.set_broadcast(true)
        .map_err(|e| format!("enable broadcast: {e}"))?;

    let req = json!({
        "action": ACTION_DISCOVER,
        "msgid": next_msgid(),
        "appid": APP_ID,
        "version": APP_VERSION,
        "packageName": APP_ID,
    })
    .to_string();
    let req_bytes = req.as_bytes();
    let dest = ("255.255.255.255", UDP_DISCOVER_PORT);

    let mut found: Vec<DiscoveredLanDevice> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let deadline = Instant::now() + timeout;
    let mut next_send = Instant::now();
    let mut sends_left = 3u8;

    while Instant::now() < deadline {
        if sends_left > 0 && Instant::now() >= next_send {
            let _ = tx.send_to(req_bytes, dest);
            sends_left -= 1;
            next_send = Instant::now() + Duration::from_millis(400);
        }

        let mut buf = [0u8; 65536];
        match listen.recv_from(&mut buf) {
            Ok((n, _from)) => {
                let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else {
                    continue;
                };
                let action = v.get("action").and_then(|a| a.as_u64()).unwrap_or(0);
                if action != ACTION_DISCOVER_REPLY as u64 {
                    continue;
                }
                let Ok(d) = parse_discover_reply(&v) else {
                    continue;
                };
                if seen.insert(d.pid.clone()) {
                    found.push(d);
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => {}
        }
    }

    Ok(found)
}

fn bind_reply_socket() -> Result<UdpSocket, String> {
    // std::net 无法在 bind 前设 SO_REUSE*；用 socket2 对齐 probe（REUSEADDR + macOS REUSEPORT）。
    let sock = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| format!("socket UDP failed: {e}"))?;
    sock.set_reuse_address(true)
        .map_err(|e| format!("SO_REUSEADDR failed: {e}"))?;
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    {
        let _ = sock.set_reuse_port(true);
    }
    let addr = SocketAddr::from(([0, 0, 0, 0], UDP_REPLY_PORT));
    sock.bind(&socket2::SockAddr::from(addr))
        .map_err(|e| format!("bind UDP 0.0.0.0:{UDP_REPLY_PORT} failed: {e}"))?;
    Ok(sock.into())
}

// ---------------------------------------------------------------- 帧编解码

pub fn encode_frame(body: &str) -> Vec<u8> {
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(6 + bytes.len());
    out.extend_from_slice(&FRAME_MAGIC);
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    out
}

fn find_magic(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    (0..=buf.len() - 2).find(|&i| buf[i] == FRAME_MAGIC[0] && buf[i + 1] == FRAME_MAGIC[1])
}

/// 从缓冲里取出所有完整帧的 body，未完成的留在 `buf`。
pub fn decode_frames(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        match find_magic(buf) {
            Some(0) => {}
            Some(pos) => {
                buf.drain(..pos);
            }
            None => {
                // 末尾可能是半个魔数，留一个字节
                if buf.last().copied() == Some(FRAME_MAGIC[0]) {
                    let keep = buf.len() - 1;
                    buf.drain(..keep);
                } else {
                    buf.clear();
                }
                break;
            }
        }
        if buf.len() < 6 {
            break;
        }
        let n = u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]) as usize;
        if n == 0 || n > MAX_BODY {
            buf.drain(..2);
            continue;
        }
        if buf.len() < 6 + n {
            break;
        }
        let body: Vec<u8> = buf[6..6 + n].to_vec();
        buf.drain(..6 + n);
        if let Ok(s) = String::from_utf8(body) {
            out.push(s);
        }
    }
    out
}

// ---------------------------------------------------------------- 真机客户端

static MSG_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_msgid() -> String {
    let seq = MSG_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("{:010}", (nanos ^ (seq * 2654435761)) % 10_000_000_000)
}

fn envelope(action: u32, common: Option<Value>) -> String {
    let mut v = json!({
        "action": action,
        "msgid": next_msgid(),
        "appid": APP_ID,
        "version": APP_VERSION,
        "packageName": APP_ID,
    });
    if let Some(c) = common {
        v["commonCmd"] = c;
    }
    v.to_string()
}

fn resolve_addr(host: &str) -> Result<SocketAddr, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("host required".into());
    }
    if let Ok(addr) = host.parse::<SocketAddr>() {
        return Ok(addr);
    }
    (host, DEFAULT_TCP_PORT)
        .to_socket_addrs()
        .ok()
        .and_then(|mut it| it.next())
        .ok_or_else(|| format!("cannot resolve {host}"))
}

pub struct XgimiLan {
    conns: Arc<Mutex<HashMap<String, TcpStream>>>,
    cache: Arc<Mutex<HashMap<String, LampState>>>,
    timeout: Duration,
}

impl Default for XgimiLan {
    fn default() -> Self {
        Self::new()
    }
}

impl XgimiLan {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(2))
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            conns: Arc::new(Mutex::new(HashMap::new())),
            cache: Arc::new(Mutex::new(HashMap::new())),
            timeout,
        }
    }

    fn connect(&self, host: &str) -> Result<TcpStream, String> {
        let addr = resolve_addr(host)?;
        let stream = TcpStream::connect_timeout(&addr, self.timeout)
            .map_err(|e| format!("connect {addr} failed: {e}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|e| format!("set read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| format!("set write timeout: {e}"))?;
        let _ = stream.set_nodelay(true);
        Ok(stream)
    }

    fn exchange(
        &self,
        stream: &mut TcpStream,
        action: u32,
        common: Option<Value>,
    ) -> Result<Value, String> {
        let body = envelope(action, common);
        stream
            .write_all(&encode_frame(&body))
            .map_err(|e| format!("write failed: {e}"))?;
        let _ = stream.flush();

        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            for body in decode_frames(&mut buf) {
                let Ok(v) = serde_json::from_str::<Value>(&body) else {
                    continue;
                };
                let got = v.get("action").and_then(|a| a.as_u64()).unwrap_or(u64::MAX);
                if got == action as u64 {
                    return Ok(v);
                }
                // 50022 / 50023 等主动推送：跳过
            }
            let n = stream
                .read(&mut chunk)
                .map_err(|e| format!("read failed waiting action {action}: {e}"))?;
            if n == 0 {
                return Err(format!("connection closed waiting action {action}"));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    fn request(&self, host: &str, action: u32, common: Option<Value>) -> Result<Value, String> {
        let mut guard = self.conns.lock().map_err(|e| e.to_string())?;
        let reusable = guard.remove(host);
        drop(guard);

        if let Some(mut stream) = reusable {
            match self.exchange(&mut stream, action, common.clone()) {
                Ok(v) => {
                    if let Ok(mut g) = self.conns.lock() {
                        g.insert(host.to_string(), stream);
                    }
                    return Ok(v);
                }
                Err(_) => { /* 连接可能已失效，重连一次 */ }
            }
        }

        let mut stream = self.connect(host)?;
        let v = self.exchange(&mut stream, action, common)?;
        if let Ok(mut g) = self.conns.lock() {
            g.insert(host.to_string(), stream);
        }
        Ok(v)
    }

    fn query_field(&self, host: &str, typ: &str) -> Result<String, String> {
        let v = self.request(host, ACTION_GET_LIGHT, Some(json!({ "type": typ })))?;
        let info = v
            .get("featureDetailInfo")
            .ok_or_else(|| format!("no featureDetailInfo for type {typ}: {v}"))?;
        info.get("value")
            .and_then(|x| x.as_str())
            .or_else(|| info.get("curChildItem").and_then(|x| x.as_str()))
            .map(|s| s.to_string())
            .ok_or_else(|| format!("no value for type {typ}: {v}"))
    }

    fn set_field(&self, host: &str, typ: &str, value: impl Into<String>) -> Result<(), String> {
        self.request(
            host,
            ACTION_SET_LIGHT,
            Some(json!({ "type": typ, "value": value.into() })),
        )
        .map(|_| ())
    }

    fn update_cache(&self, host: &str, f: impl FnOnce(&mut LampState)) {
        if let Ok(mut cache) = self.cache.lock() {
            f(cache.entry(host.to_string()).or_default());
        }
    }

    /// 设备在「自定义」模式下报 1–10 档；其它模式（节律光/日光/观影）
    /// 报 0–100 的百分比。用 >10 判定为百分比。
    fn brightness_from_raw(raw: &str) -> u8 {
        let v = raw.trim().parse::<u8>().unwrap_or(50);
        if v > DEVICE_LEVEL_MAX {
            v.min(100)
        } else {
            brightness_schema_from_level(v)
        }
    }

    fn color_temp_from_raw(raw: &str) -> u32 {
        let v = raw.trim().parse::<u32>().unwrap_or(50).min(100);
        if v > DEVICE_LEVEL_MAX as u32 {
            2000 + v * (4500 / 100)
        } else {
            color_temp_schema_from_level(v as u8)
        }
    }

    /// 亮度/色温只在「自定义」模式（mode=0）下是 1–10 档，写入前先切换。
    fn ensure_custom_mode(&self, host: &str) -> Result<(), String> {
        let mode = self.query_field(host, TYPE_MODE)?;
        if mode != "0" {
            self.set_field(host, TYPE_MODE, "0")?;
        }
        Ok(())
    }
}

impl LanBackend for XgimiLan {
    fn read_all(&self, host: &str) -> Result<LampState, String> {
        let on = self.query_field(host, TYPE_SWITCH)? == "1";
        let brightness = Self::brightness_from_raw(&self.query_field(host, TYPE_BRIGHTNESS)?);
        let color_temp_k = Self::color_temp_from_raw(&self.query_field(host, TYPE_COLOR_TEMP)?);
        // 模式（type 3）暂不建模，读一次保证与 App 行为一致，失败忽略
        let _ = self.query_field(host, TYPE_MODE);

        let state = LampState {
            on,
            brightness,
            color_temp_k,
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(host.to_string(), state.clone());
        }
        Ok(state)
    }

    fn write_power(&self, host: &str, on: bool) -> Result<(), String> {
        self.set_field(host, TYPE_SWITCH, if on { "1" } else { "0" })?;
        self.update_cache(host, |s| s.on = on);
        Ok(())
    }

    fn write_brightness(&self, host: &str, bri: u8) -> Result<(), String> {
        let bri = clamp_brightness(bri as i64);
        self.ensure_custom_mode(host)?;
        let level = brightness_level_from_schema(bri);
        self.set_field(host, TYPE_BRIGHTNESS, level.to_string())?;
        self.update_cache(host, |s| s.brightness = bri);
        Ok(())
    }

    fn write_color_temp(&self, host: &str, k: u32) -> Result<(), String> {
        let k = clamp_color_temp(k as i64);
        self.ensure_custom_mode(host)?;
        let level = color_temp_level_from_schema(k);
        self.set_field(host, TYPE_COLOR_TEMP, level.to_string())?;
        self.update_cache(host, |s| s.color_temp_k = k);
        Ok(())
    }

    fn cached(&self, host: &str) -> Option<LampState> {
        self.cache
            .lock()
            .ok()
            .and_then(|c| c.get(host).cloned())
    }
}

// ---------------------------------------------------------------- Stub（无设备环境）

#[derive(Clone, Default)]
pub struct StubLan {
    inner: Arc<Mutex<HashMap<String, LampState>>>,
}

impl StubLan {
    pub fn read_all(&self, host: &str) -> Result<LampState, String> {
        let map = self.inner.lock().map_err(|e| e.to_string())?;
        Ok(map.get(host).cloned().unwrap_or_default())
    }

    pub fn write_power(&self, host: &str, on: bool) -> Result<(), String> {
        let mut map = self.inner.lock().map_err(|e| e.to_string())?;
        map.entry(host.to_string()).or_default().on = on;
        Ok(())
    }

    pub fn write_brightness(&self, host: &str, bri: u8) -> Result<(), String> {
        let mut map = self.inner.lock().map_err(|e| e.to_string())?;
        map.entry(host.to_string()).or_default().brightness = bri;
        Ok(())
    }

    pub fn write_color_temp(&self, host: &str, k: u32) -> Result<(), String> {
        let mut map = self.inner.lock().map_err(|e| e.to_string())?;
        map.entry(host.to_string()).or_default().color_temp_k = k;
        Ok(())
    }
}

impl LanBackend for StubLan {
    fn read_all(&self, host: &str) -> Result<LampState, String> {
        StubLan::read_all(self, host)
    }
    fn write_power(&self, host: &str, on: bool) -> Result<(), String> {
        StubLan::write_power(self, host, on)
    }
    fn write_brightness(&self, host: &str, bri: u8) -> Result<(), String> {
        StubLan::write_brightness(self, host, bri)
    }
    fn write_color_temp(&self, host: &str, k: u32) -> Result<(), String> {
        StubLan::write_color_temp(self, host, k)
    }
    fn cached(&self, host: &str) -> Option<LampState> {
        self.inner.lock().ok().and_then(|m| m.get(host).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_write_read_roundtrip() {
        let lan = StubLan::default();
        let host = "192.168.1.50";

        lan.write_power(host, true).unwrap();
        lan.write_brightness(host, 42).unwrap();
        lan.write_color_temp(host, 3500).unwrap();

        assert_eq!(
            lan.read_all(host).unwrap(),
            LampState {
                on: true,
                brightness: 42,
                color_temp_k: 3500,
            }
        );
    }

    #[test]
    fn unknown_host_defaults() {
        let lan = StubLan::default();
        assert_eq!(lan.read_all("10.0.0.1").unwrap(), LampState::default());
        assert_eq!(
            lan.read_all("10.0.0.1").unwrap(),
            LampState {
                on: false,
                brightness: 100,
                color_temp_k: 4000,
            }
        );
    }

    #[test]
    fn lan_backend_trait_roundtrip() {
        fn exercise<L: LanBackend>(lan: &L) {
            lan.write_power("h", true).unwrap();
            lan.write_brightness("h", 40).unwrap();
            lan.write_color_temp("h", 3000).unwrap();
            let s = lan.read_all("h").unwrap();
            assert_eq!(
                s,
                LampState {
                    on: true,
                    brightness: 40,
                    color_temp_k: 3000,
                }
            );
            assert_eq!(lan.cached("h").map(|c| c.on), Some(true));
        }
        exercise(&StubLan::default());
    }

    #[test]
    fn envelope_has_required_fields() {
        let v: Value = serde_json::from_str(&envelope(50040, Some(json!({"type": "0"})))).unwrap();
        assert_eq!(v["action"], 50040);
        assert_eq!(v["appid"], APP_ID);
        assert_eq!(v["version"], APP_VERSION);
        assert_eq!(v["packageName"], APP_ID);
        assert_eq!(v["commonCmd"]["type"], "0");
        assert!(v["msgid"].as_str().unwrap().len() <= 10);
    }

    #[test]
    fn raw_values_handle_both_scales() {
        // 自定义模式：1–10 档
        assert_eq!(XgimiLan::brightness_from_raw("5"), 50);
        assert_eq!(XgimiLan::color_temp_from_raw("1"), 2000);
        assert_eq!(XgimiLan::color_temp_from_raw("10"), 6500);
        // 节律光等模式：0–100 百分比
        assert_eq!(XgimiLan::brightness_from_raw("77"), 77);
        assert_eq!(XgimiLan::color_temp_from_raw("62"), 2000 + 62 * 45);
    }

    #[test]
    fn resolve_addr_defaults_port() {
        assert_eq!(
            resolve_addr("127.0.0.1:13145").unwrap().port(),
            13145
        );
        assert_eq!(
            resolve_addr("127.0.0.1").unwrap().port(),
            DEFAULT_TCP_PORT
        );
        assert!(resolve_addr("  ").is_err());
    }

    #[test]
    fn parse_discover_reply_extracts_fields() {
        let raw = serde_json::json!({
            "action": 9999,
            "deviceInfo": {
                "device": "极米神灯",
                "devicename": "我的极米神灯",
                "deviceip": "172.21.10.36",
                "productName": "Product_cp01",
                "pid": "SWX5PZA97YWC"
            }
        });
        let d = parse_discover_reply(&raw).unwrap();
        assert_eq!(d.pid, "SWX5PZA97YWC");
        assert_eq!(d.host, "172.21.10.36");
        assert_eq!(d.product_name.as_deref(), Some("Product_cp01"));
        assert!(d.support_light);
    }
}
