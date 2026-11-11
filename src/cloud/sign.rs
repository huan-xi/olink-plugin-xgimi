//! 极米云请求签名（与 GmApp / scripts/xgimi_login.py 一致）。

use md5::{Digest, Md5};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SIGN_SALT: &str = "9y$B&E)H@McQeThW";

/// 构造签名原文：`k=v&` 按 key 升序拼接（含末尾 `&`），不含盐。
pub fn build_sign_raw(params: &BTreeMap<String, String>) -> String {
    let mut raw = String::new();
    for (k, v) in params {
        raw.push_str(k);
        raw.push('=');
        raw.push_str(v);
        raw.push('&');
    }
    raw
}

/// MD5 小写 hex：`md5(raw + salt)`。
pub fn sign_md5(raw: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(raw.as_bytes());
    hasher.update(salt.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// 为参数表写入 timestamp，并返回 `(timestamp, sign)`。
pub fn sign_params(params: &mut BTreeMap<String, String>, timestamp_ms: &str) -> (String, String) {
    params.insert("timestamp".into(), timestamp_ms.to_string());
    let raw = build_sign_raw(params);
    let sign = sign_md5(&raw, SIGN_SALT);
    (timestamp_ms.to_string(), sign)
}

pub fn now_millis() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    ms.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_sign_raw_sorted_with_trailing_amp() {
        let mut p = BTreeMap::new();
        p.insert("type".into(), "2".into());
        p.insert("mobile".into(), "17600000000".into());
        p.insert("timestamp".into(), "1000".into());
        assert_eq!(
            build_sign_raw(&p),
            "mobile=17600000000&timestamp=1000&type=2&"
        );
    }

    #[test]
    fn sign_md5_matches_known_vector() {
        let raw = "mobile=17600000000&timestamp=1000&type=2&";
        assert_eq!(
            sign_md5(raw, SIGN_SALT),
            "703d6512228acd1b2807290226fb06fb"
        );
    }

    #[test]
    fn sign_params_injects_timestamp() {
        let mut p = BTreeMap::new();
        p.insert("mobile".into(), "17600000000".into());
        p.insert("type".into(), "2".into());
        let (ts, sig) = sign_params(&mut p, "1000");
        assert_eq!(ts, "1000");
        assert_eq!(p.get("timestamp").map(String::as_str), Some("1000"));
        assert_eq!(sig, "703d6512228acd1b2807290226fb06fb");
    }
}
