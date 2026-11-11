//! 极米 CP01 局域网协议：帧编解码 + 客户端行为（对假设备）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use olink_plugin_xgimi::lan::{decode_frames, encode_frame, LanBackend, LampState, XgimiLan};
use serde_json::{json, Value};

const MAGIC: [u8; 2] = [0xE0, 0x4A];

fn read_frame(stream: &mut TcpStream) -> Option<Value> {
    let mut hdr = [0u8; 6];
    stream.read_exact(&mut hdr).ok()?;
    assert_eq!(&hdr[0..2], &MAGIC, "frame magic");
    let n = u32::from_be_bytes([hdr[2], hdr[3], hdr[4], hdr[5]]) as usize;
    assert!(n > 0 && n < 1 << 20, "frame length sane");
    let mut body = vec![0u8; n];
    stream.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// 起一个假设备：对每个收到的帧调用 handler，返回 Some(JSON) 就回一帧。
fn fake_device<F>(handler: F) -> (String, Arc<Mutex<Vec<Value>>>)
where
    F: Fn(&Value) -> Vec<Value> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            while let Some(req) = read_frame(&mut s) {
                seen2.lock().unwrap().push(req.clone());
                for resp in handler(&req) {
                    let _ = s.write_all(&encode_frame(&resp.to_string()));
                }
            }
        }
    });
    (addr, seen)
}

fn lighting_reply(typ: &str) -> Value {
    let (key, value) = match typ {
        "0" => ("lightSwitch", "1"),
        "1" => ("lightColorTemp", "1"),
        "2" => ("lightBrightness", "5"),
        _ => ("ceilingLampMode", "0"),
    };
    json!({
        "action": 50040,
        "type": 0,
        "featureDetailInfo": { "key": key, "value": value },
        "agreement": { "agreementType": 1, "isTcp": true, "port": 13145 }
    })
}

// ---------------- 帧编解码 ----------------

#[test]
fn frame_roundtrip() {
    let body = r#"{"action":50040,"commonCmd":{"type":"0"}}"#;
    let raw = encode_frame(body);
    assert_eq!(&raw[0..2], &MAGIC);
    let n = u32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]) as usize;
    assert_eq!(n, body.len());
    assert_eq!(raw.len(), 6 + body.len());

    let mut buf = raw;
    let frames = decode_frames(&mut buf);
    assert_eq!(frames, vec![body.to_string()]);
    assert!(buf.is_empty());
}

#[test]
fn frame_decode_keeps_partial() {
    let raw = encode_frame("{\"a\":1}");
    let mut buf = raw[..4].to_vec();
    assert!(decode_frames(&mut buf).is_empty());
    buf.extend_from_slice(&raw[4..]);
    assert_eq!(decode_frames(&mut buf), vec!["{\"a\":1}".to_string()]);
}

#[test]
fn frame_decode_skips_garbage_and_concatenates() {
    let mut buf = b"\x00\xff".to_vec();
    buf.extend_from_slice(&encode_frame("{\"a\":1}"));
    buf.extend_from_slice(&encode_frame("{\"b\":2}"));
    assert_eq!(
        decode_frames(&mut buf),
        vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
    );
}

#[test]
fn frame_roundtrip_is_utf8_byte_length() {
    let body = "{\"device\":\"极米神灯\"}";
    let raw = encode_frame(body);
    let n = u32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]) as usize;
    assert_eq!(n, body.as_bytes().len(), "长度必须是 UTF-8 字节数");
    let mut buf = raw;
    assert_eq!(decode_frames(&mut buf), vec![body.to_string()]);
}

// ---------------- 客户端 ----------------

#[test]
fn read_all_maps_four_types() {
    let (addr, seen) = fake_device(|req| {
        assert_eq!(req["action"], 50040);
        let t = req["commonCmd"]["type"].as_str().unwrap().to_string();
        vec![lighting_reply(&t)]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    let state = lan.read_all(&addr).unwrap();
    assert_eq!(
        state,
        LampState {
            on: true,
            brightness: 50,
            color_temp_k: 2000,
        }
    );
    assert_eq!(seen.lock().unwrap().len(), 4);
}

#[test]
fn read_all_ignores_unsolicited_push() {
    let (addr, _) = fake_device(|req| {
        let t = req["commonCmd"]["type"].as_str().unwrap().to_string();
        // 设备可能在应答前先推一条 50023
        vec![
            json!({"action":50023,"deviceStateInfo":{"state":[{"key":"lightSwitch","value":"1"}]}}),
            lighting_reply(&t),
        ]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    let state = lan.read_all(&addr).unwrap();
    assert!(state.on);
}

#[test]
fn write_power_sends_type0() {
    let (addr, seen) = fake_device(|req| {
        vec![json!({"action": req["action"], "status": {"state": 1}})]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    lan.write_power(&addr, true).unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0]["action"], 50041);
    assert_eq!(seen[0]["commonCmd"]["type"], "0");
    assert_eq!(seen[0]["commonCmd"]["value"], "1");
}

#[test]
fn write_brightness_converts_schema_to_level() {
    let (addr, seen) = fake_device(|req| {
        if req["action"] == 50040 {
            let t = req["commonCmd"]["type"].as_str().unwrap().to_string();
            return vec![lighting_reply(&t)];
        }
        vec![json!({"action": req["action"], "status": {"state": 1}})]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    lan.write_brightness(&addr, 57).unwrap();
    let seen = seen.lock().unwrap();
    let set = seen
        .iter()
        .find(|r| r["action"] == 50041 && r["commonCmd"]["type"] == "2")
        .expect("应下发 type=2 的写入");
    assert_eq!(set["commonCmd"]["value"], "6", "57% -> 6 档");
}

#[test]
fn write_color_temp_converts_schema_to_level() {
    let (addr, seen) = fake_device(|req| {
        if req["action"] == 50040 {
            let t = req["commonCmd"]["type"].as_str().unwrap().to_string();
            return vec![lighting_reply(&t)];
        }
        vec![json!({"action": req["action"], "status": {"state": 1}})]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    lan.write_color_temp(&addr, 6500).unwrap();
    let seen = seen.lock().unwrap();
    let set = seen
        .iter()
        .find(|r| r["action"] == 50041 && r["commonCmd"]["type"] == "1")
        .expect("应下发 type=1 的写入");
    assert_eq!(set["commonCmd"]["value"], "10", "6500K -> 10 档");
}

#[test]
fn write_brightness_switches_to_custom_mode_first() {
    let (addr, seen) = fake_device(|req| {
        if req["action"] == 50040 && req["commonCmd"]["type"] == "3" {
            return vec![json!({"action":50040,
                "featureDetailInfo":{"key":"ceilingLampMode","curChildItem":"1"}})];
        }
        vec![json!({"action": req["action"], "status": {"state": 1}})]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    lan.write_brightness(&addr, 50).unwrap();
    let seen = seen.lock().unwrap();
    let kinds: Vec<(u64, String)> = seen
        .iter()
        .map(|r| (r["action"].as_u64().unwrap(), r["commonCmd"]["type"].as_str().unwrap_or("").to_string()))
        .collect();
    assert_eq!(
        kinds,
        vec![(50040, "3".to_string()), (50041, "3".to_string()), (50041, "2".to_string())],
        "节律光模式写入亮度前应先切自定义"
    );
    assert_eq!(seen[1]["commonCmd"]["value"], "0");
}

#[test]
fn request_fails_fast_when_host_unreachable() {
    let lan = XgimiLan::with_timeout(Duration::from_millis(300));
    let err = lan.read_all("127.0.0.1:9").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn cached_state_after_successful_read() {
    let (addr, _) = fake_device(|req| {
        let t = req["commonCmd"]["type"].as_str().unwrap().to_string();
        vec![lighting_reply(&t)]
    });
    let lan = XgimiLan::with_timeout(Duration::from_secs(2));
    assert!(lan.cached(&addr).is_none());
    let _ = lan.read_all(&addr).unwrap();
    assert_eq!(lan.cached(&addr).map(|s| s.on), Some(true));
}

// ---------------- 真机（默认忽略） ----------------

/// XGIMI_HOST=172.21.10.36 cargo test --test lan_protocol -- --ignored --nocapture
#[test]
#[ignore = "需要真机：设置 XGIMI_HOST 后加 --ignored 运行"]
fn live_device_read_only() {
    let host = std::env::var("XGIMI_HOST").expect("XGIMI_HOST=设备IP");
    let lan = XgimiLan::with_timeout(Duration::from_secs(3));
    let state = lan.read_all(&host).unwrap();
    println!("live state: {state:?}");
    assert!(state.brightness <= 100);
    assert!((2000..=6500).contains(&state.color_temp_k));
    assert_eq!(lan.cached(&host).map(|c| c.brightness), Some(state.brightness));
}

/// 真机写测试（会真的开灯，默认跳过）：
/// XGIMI_HOST=172.21.10.36 XGIMI_LIVE_WRITE=1 cargo test --test lan_protocol -- --ignored
#[test]
#[ignore = "需要真机且会改变灯状态：设置 XGIMI_HOST + XGIMI_LIVE_WRITE=1"]
fn live_device_power_on() {
    if std::env::var("XGIMI_LIVE_WRITE").ok().as_deref() != Some("1") {
        println!("skip: XGIMI_LIVE_WRITE != 1");
        return;
    }
    let host = std::env::var("XGIMI_HOST").expect("XGIMI_HOST=设备IP");
    let lan = XgimiLan::with_timeout(Duration::from_secs(3));

    fn wait_for(lan: &XgimiLan, host: &str, want: bool) -> bool {
        for _ in 0..10 {
            if lan.read_all(host).map(|s| s.on).unwrap_or(false) == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        false
    }

    lan.write_power(&host, false).unwrap();
    println!("power off applied: {}", wait_for(&lan, &host, false));
    lan.write_power(&host, true).unwrap();
    let on = wait_for(&lan, &host, true);
    println!("after power on: {:?}", lan.read_all(&host));
    assert!(on, "设备应报告 lightSwitch=1");
}
