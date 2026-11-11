//! Host `device.register` 用的 light 声明；与详情 API 共用，避免两套漂移。

use serde_json::{json, Value};

pub const MANUFACTURER: &str = "XGIMI";
pub const DEFAULT_MODEL: &str = "CP01";
pub const SCHEMA_ID: &str = "light";

/// register / detail 共用的 services 数组（含 write 策略，Host 需要）。
pub fn light_services() -> Value {
    json!([{
        "service_type": "light",
        "attributes": [
            {
                "attr_type": "power-state",
                "access": "readwrite",
                "write": { "policy": "optimistic", "verify_capable": false }
            },
            {
                "attr_type": "brightness",
                "access": "readwrite",
                "write": { "policy": "optimistic", "verify_capable": false }
            },
            {
                "attr_type": "color-temperature",
                "access": "readwrite",
                "write": { "policy": "optimistic", "verify_capable": false }
            }
        ]
    }])
}

/// 详情弹窗展示用（可去掉 write 块，保留 attr_type/access）。
pub fn light_services_for_detail() -> Value {
    json!([{
        "service_type": "light",
        "attributes": [
            { "attr_type": "power-state", "access": "readwrite" },
            { "attr_type": "brightness", "access": "readwrite" },
            { "attr_type": "color-temperature", "access": "readwrite" }
        ]
    }])
}

pub fn resolve_model(model: &str) -> &str {
    if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    }
}

pub fn register_body(device_key: &str, name: &str, model: &str) -> Value {
    let model = resolve_model(model);
    json!({
        "device_key": device_key,
        "schema_id": SCHEMA_ID,
        "name": name,
        "manufacturer": MANUFACTURER,
        "model": model,
        "services": light_services(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_detail_share_attr_types() {
        let reg = light_services();
        let detail = light_services_for_detail();
        let reg_attrs = reg[0]["attributes"].as_array().unwrap();
        let detail_attrs = detail[0]["attributes"].as_array().unwrap();
        assert_eq!(reg_attrs.len(), detail_attrs.len());
        for (a, b) in reg_attrs.iter().zip(detail_attrs.iter()) {
            assert_eq!(a["attr_type"], b["attr_type"]);
            assert_eq!(a["access"], b["access"]);
        }
    }
}
