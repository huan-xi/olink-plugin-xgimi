//! 灯属性策略：parse / apply / from_state 统一行为。

use crate::lan::{LampState, LanBackend};
use crate::map::{normalize_attr_id, parse_brightness, parse_color_temp, parse_power};
use serde_json::{json, Value};

pub trait LightAttr: Send + Sync {
    fn id(&self) -> &'static str;
    fn parse(&self, value: &Value) -> Result<Value, String>;
    fn apply(&self, lan: &dyn LanBackend, host: &str, schema_value: &Value) -> Result<(), String>;
    fn from_state(&self, state: &LampState) -> Value;
}

pub struct PowerAttr;
pub struct BrightnessAttr;
pub struct ColorTempAttr;

impl LightAttr for PowerAttr {
    fn id(&self) -> &'static str {
        "power-state"
    }
    fn parse(&self, value: &Value) -> Result<Value, String> {
        Ok(Value::Bool(parse_power(value)?))
    }
    fn apply(&self, lan: &dyn LanBackend, host: &str, schema_value: &Value) -> Result<(), String> {
        lan.write_power(host, parse_power(schema_value)?)
    }
    fn from_state(&self, state: &LampState) -> Value {
        Value::Bool(state.on)
    }
}

impl LightAttr for BrightnessAttr {
    fn id(&self) -> &'static str {
        "brightness"
    }
    fn parse(&self, value: &Value) -> Result<Value, String> {
        Ok(json!(parse_brightness(value)?))
    }
    fn apply(&self, lan: &dyn LanBackend, host: &str, schema_value: &Value) -> Result<(), String> {
        lan.write_brightness(host, parse_brightness(schema_value)?)
    }
    fn from_state(&self, state: &LampState) -> Value {
        json!(state.brightness)
    }
}

impl LightAttr for ColorTempAttr {
    fn id(&self) -> &'static str {
        "color-temperature"
    }
    fn parse(&self, value: &Value) -> Result<Value, String> {
        Ok(json!(parse_color_temp(value)?))
    }
    fn apply(&self, lan: &dyn LanBackend, host: &str, schema_value: &Value) -> Result<(), String> {
        lan.write_color_temp(host, parse_color_temp(schema_value)?)
    }
    fn from_state(&self, state: &LampState) -> Value {
        json!(state.color_temp_k)
    }
}

const ATTRS: &[&dyn LightAttr] = &[&PowerAttr, &BrightnessAttr, &ColorTempAttr];

pub fn lookup_attr(attr_id: &str) -> Option<&'static dyn LightAttr> {
    let id = normalize_attr_id(attr_id);
    ATTRS.iter().copied().find(|a| a.id() == id)
}

pub fn all_attr_ids() -> [&'static str; 3] {
    ["power-state", "brightness", "color-temperature"]
}

pub fn values_from_state(state: &LampState, attr_ids: &[String]) -> serde_json::Map<String, Value> {
    let ids: Vec<&str> = if attr_ids.is_empty() {
        all_attr_ids().into_iter().collect()
    } else {
        attr_ids.iter().map(|s| s.as_str()).collect()
    };
    let mut values = serde_json::Map::new();
    for id in ids {
        if let Some(attr) = lookup_attr(id) {
            values.insert(attr.id().to_string(), attr.from_state(state));
        }
    }
    values
}

pub fn report_triple(state: &LampState) -> [(&'static str, Value); 3] {
    [
        ("power-state", PowerAttr.from_state(state)),
        ("brightness", BrightnessAttr.from_state(state)),
        ("color-temperature", ColorTempAttr.from_state(state)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::StubLan;
    use serde_json::json;

    #[test]
    fn power_attr_parse_apply_from_state() {
        let lan = StubLan::default();
        let attr = PowerAttr;
        let v = attr.parse(&json!(true)).unwrap();
        attr.apply(&lan, "h", &v).unwrap();
        let state = lan.read_all("h").unwrap();
        assert_eq!(attr.from_state(&state), json!(true));
        assert_eq!(attr.id(), "power-state");
    }

    #[test]
    fn brightness_attr_clamps() {
        let attr = BrightnessAttr;
        let v = attr.parse(&json!(150)).unwrap();
        assert_eq!(v, json!(100));
        let lan = StubLan::default();
        attr.apply(&lan, "h", &v).unwrap();
        assert_eq!(attr.from_state(&lan.read_all("h").unwrap()), json!(100));
    }

    #[test]
    fn color_temp_attr_clamps() {
        let attr = ColorTempAttr;
        let v = attr.parse(&json!(1000)).unwrap();
        assert_eq!(v, json!(2000));
        let lan = StubLan::default();
        attr.apply(&lan, "h", &v).unwrap();
        assert_eq!(attr.from_state(&lan.read_all("h").unwrap()), json!(2000));
    }

    #[test]
    fn registry_lookup_normalizes_prefix() {
        assert!(lookup_attr("xgimi.lamp#power-state").is_some());
        assert!(lookup_attr("brightness").is_some());
        assert!(lookup_attr("nope").is_none());
        assert_eq!(all_attr_ids(), ["power-state", "brightness", "color-temperature"]);
    }
}
