//! 极米云 HTTP 客户端。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::session::AccountSession;
use super::sign::{now_millis, sign_params};

pub const HOST_SUPERAPP: &str = "https://superapp.xgimi.com";
pub const HOST_MOBILE_API: &str = "https://mobile-api.xgimi.com";
pub const APP_VERSION: &str = "4.10.36";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudDevice {
    pub sn: String,
    pub name: String,
    #[serde(default)]
    pub item_code: String,
    #[serde(default)]
    pub product_sn: String,
    #[serde(default)]
    pub xgimi_product_name: String,
}

#[derive(Debug, Clone)]
pub struct LoginResult {
    pub session: AccountSession,
}

#[derive(Clone)]
pub struct XgimiCloudClient {
    http: reqwest::Client,
    device_id: String,
}

impl XgimiCloudClient {
    pub fn new(device_id: impl Into<String>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            http,
            device_id: device_id.into(),
        })
    }

    pub fn with_device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = device_id.into();
        self
    }

    fn base_headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("platform", "ios".parse().unwrap());
        h.insert("appVersion", APP_VERSION.parse().unwrap());
        h.insert("channel", "appstore".parse().unwrap());
        h.insert("superApp", "1".parse().unwrap());
        h.insert(
            "deviceId",
            self.device_id
                .parse()
                .unwrap_or_else(|_| "UNKNOWN".parse().unwrap()),
        );
        h.insert("deviceModel", "iPhone".parse().unwrap());
        h.insert("osName", "iOS 18.0".parse().unwrap());
        h.insert("Accept-Encoding", "identity".parse().unwrap());
        h
    }

    fn auth_headers(sess: &AccountSession) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = sess.token.parse() {
            h.insert("token", v);
        }
        if let Ok(v) = sess.access_token.parse() {
            h.insert("accessToken", v);
        }
        if let Ok(v) = sess.open_id.parse() {
            h.insert("openId", v);
        }
        h
    }

    async fn post_json(
        &self,
        url: &str,
        body: Value,
        need_sign: bool,
        sess: Option<&AccountSession>,
    ) -> anyhow::Result<Value> {
        let mut headers = self.base_headers();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        if let Some(s) = sess {
            headers.extend(Self::auth_headers(s));
        }
        let body_for_sign = body.clone();
        if need_sign {
            let mut params = json_to_sign_map(&body_for_sign);
            let ts = now_millis();
            let (timestamp, sign) = sign_params(&mut params, &ts);
            headers.insert("timestamp", timestamp.parse().unwrap());
            headers.insert("sign", sign.parse().unwrap());
        }
        let resp = self
            .http
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
            json!({ "code": "ParseError", "message": text, "data": null })
        });
        if !status.is_success() {
            let code = json
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("http_error");
            let msg = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or(text.as_str());
            bail!("{code}: {msg} (HTTP {})", status.as_u16());
        }
        let code = json.get("code").and_then(|v| v.as_str()).unwrap_or("");
        if code != "ok" && !code.is_empty() {
            let msg = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or(code);
            bail!("{code}: {msg}");
        }
        Ok(json)
    }

    pub async fn send_sms(&self, mobile: &str) -> anyhow::Result<()> {
        let url = format!("{HOST_MOBILE_API}/app/v4/account/sendSmsCode");
        let body = json!({ "mobile": mobile, "type": 2 });
        let _ = self.post_json(&url, body, true, None).await?;
        Ok(())
    }

    pub async fn login(&self, mobile: &str, code: &str) -> anyhow::Result<LoginResult> {
        let url = format!("{HOST_SUPERAPP}/api/v1/user/access");
        let body = json!({ "mobile": mobile, "code": code });
        let json = self.post_json(&url, body, false, None).await?;
        let data = json.get("data").cloned().unwrap_or(Value::Null);
        let session = parse_login_session(&data, mobile, &self.device_id)?;
        Ok(LoginResult { session })
    }

    pub async fn refresh(&self, sess: &AccountSession) -> anyhow::Result<AccountSession> {
        if sess.refresh_token.is_empty() {
            bail!("missing refreshToken");
        }
        let url = format!("{HOST_MOBILE_API}/app/v4/user/refreshToken");
        let body = json!({ "refreshToken": sess.refresh_token });
        let json = self.post_json(&url, body, true, None).await?;
        let data = json.get("data").cloned().unwrap_or(Value::Null);
        let mut next = sess.clone();
        if let Some(at) = data.get("accessToken").and_then(|v| v.as_str()) {
            next.access_token = at.to_string();
        }
        if let Some(rt) = data.get("refreshToken").and_then(|v| v.as_str()) {
            next.refresh_token = rt.to_string();
        }
        next.saved_at = unix_now();
        Ok(next)
    }

    pub async fn list_devices(&self, sess: &AccountSession) -> anyhow::Result<Vec<CloudDevice>> {
        if !sess.logged_in() {
            bail!("not logged in");
        }
        let url = format!("{HOST_MOBILE_API}/app/v4/device/new/list");
        let body = json!({ "pageSize": 50, "currentPage": 1 });
        let json = self.post_json(&url, body, true, Some(sess)).await?;
        Ok(parse_device_list(&json)?)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn json_to_sign_map(body: &Value) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(obj) = body.as_object() else {
        return map;
    };
    for (k, v) in obj {
        let s = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        map.insert(k.clone(), s);
    }
    map
}

pub fn parse_login_session(
    data: &Value,
    mobile: &str,
    device_id: &str,
) -> anyhow::Result<AccountSession> {
    let token = data
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("login missing token"))?
        .to_string();
    let user = data.get("user").cloned().unwrap_or(Value::Null);
    let access = user.get("access").cloned().unwrap_or(Value::Null);
    let access_token = user
        .get("accessToken")
        .and_then(|v| v.as_str())
        .or_else(|| access.get("accessToken").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let refresh_token = access
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let open_id = user
        .get("openId")
        .and_then(|v| v.as_str())
        .or_else(|| access.get("openId").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let uid = user
        .get("uid")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default();
    let mobile = user
        .get("mobile")
        .and_then(|v| v.as_str())
        .unwrap_or(mobile)
        .to_string();
    if access_token.is_empty() || open_id.is_empty() {
        bail!("login missing accessToken/openId");
    }
    Ok(AccountSession {
        token,
        access_token,
        refresh_token,
        open_id,
        uid,
        mobile,
        device_id: device_id.to_string(),
        saved_at: unix_now(),
    })
}

pub fn parse_device_list(root: &Value) -> anyhow::Result<Vec<CloudDevice>> {
    let data = root.get("data").cloned().unwrap_or(Value::Null);
    let list = data
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for item in list {
        let sn = item
            .get("sn")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if sn.is_empty() {
            continue;
        }
        let name = item
            .get("productName")
            .and_then(|v| v.as_str())
            .or_else(|| {
                item.get("spuConfQueryDTO")
                    .and_then(|s| s.get("abbr"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or(sn.as_str())
            .to_string();
        let item_code = item
            .get("itemCode")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let product_sn = item
            .get("productSn")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let xgimi_product_name = item
            .get("xgimiProductName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push(CloudDevice {
            sn,
            name,
            item_code,
            product_sn,
            xgimi_product_name,
        });
    }
    Ok(out)
}

/// 云 sn → Host device_key：`{pid}_light_cloud`。
pub fn device_id_from_sn(sn: &str) -> String {
    crate::device_id::make_device_id(
        sn,
        crate::device_id::XgimiDeviceType::Light,
        crate::device_id::Channel::Cloud,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_login_fixture_fields() {
        let data = json!({
            "token": "jwt-token",
            "user": {
                "uid": "13469929",
                "mobile": "17600000000",
                "openId": "f2b4aaaaaaaaaaaaaaaaaaaaaaaa0929",
                "accessToken": "at-top",
                "access": {
                    "accessToken": "at-nested",
                    "refreshToken": "rt-1",
                    "openId": "f2b4aaaaaaaaaaaaaaaaaaaaaaaa0929",
                    "expiresIn": 2592000
                }
            }
        });
        let sess = parse_login_session(&data, "17600000000", "DEV1").unwrap();
        assert_eq!(sess.token, "jwt-token");
        assert_eq!(sess.access_token, "at-top");
        assert_eq!(sess.refresh_token, "rt-1");
        assert_eq!(sess.open_id, "f2b4aaaaaaaaaaaaaaaaaaaaaaaa0929");
        assert_eq!(sess.uid, "13469929");
        assert_eq!(sess.mobile, "17600000000");
        assert_eq!(sess.device_id, "DEV1");
        assert!(sess.logged_in());
    }

    #[test]
    fn parse_device_list_fixture() {
        let root = json!({
            "code": "ok",
            "data": {
                "data": [{
                    "productSn": "GMN_001022",
                    "productName": "极米神灯",
                    "sn": "SWX5ABCDYWC",
                    "xgimiProductName": "Product_cp01",
                    "itemCode": "ICM_011801",
                    "spuConfQueryDTO": { "abbr": "极米神灯" }
                }],
                "paginator": { "currentPage": 1, "pageSize": 20, "totalRecord": 1 }
            }
        });
        let list = parse_device_list(&root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].sn, "SWX5ABCDYWC");
        assert_eq!(list[0].name, "极米神灯");
        assert_eq!(list[0].item_code, "ICM_011801");
        assert_eq!(list[0].xgimi_product_name, "Product_cp01");
    }

    #[test]
    fn device_id_from_sn_sanitizes() {
        assert_eq!(device_id_from_sn("SWX5****YWC"), "SWX5____YWC_light_cloud");
        assert_eq!(device_id_from_sn("ok-id_1"), "ok-id_1_light_cloud");
    }

    #[test]
    fn json_to_sign_map_stringifies_numbers() {
        let body = json!({ "mobile": "1", "type": 2, "pageSize": 20 });
        let map = json_to_sign_map(&body);
        assert_eq!(map.get("type").map(String::as_str), Some("2"));
        assert_eq!(map.get("pageSize").map(String::as_str), Some("20"));
    }
}
