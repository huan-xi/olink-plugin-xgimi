//! schema 值解析与量程转换。
//!
//! schema 侧：brightness 0–100、color-temp 2000–6500 K。
//! 设备侧：亮度/色温都是 **1–10 档**（实测 CP01，见 docs/reference 协议文档）。

use serde_json::Value;

/// 设备亮度/色温档位上限。
pub const DEVICE_LEVEL_MAX: u8 = 10;
pub const COLOR_TEMP_MIN_K: u32 = 2000;
pub const COLOR_TEMP_MAX_K: u32 = 6500;
const COLOR_TEMP_STEP_K: u32 = 500;

pub fn normalize_attr_id(attr_id: &str) -> &str {
    if let Some(i) = attr_id.rfind('#') {
        &attr_id[i + 1..]
    } else {
        attr_id
    }
}

pub fn clamp_brightness(v: i64) -> u8 {
    v.clamp(0, 100) as u8
}

/// Clamp color-temperature (Kelvin) into schema range 2000–6500.
pub fn clamp_color_temp(v: i64) -> u32 {
    v.clamp(COLOR_TEMP_MIN_K as i64, COLOR_TEMP_MAX_K as i64) as u32
}

/// schema 亮度 0–100 → 设备档位 1–10（四舍五入，0 也落到 1 档）。
pub fn brightness_level_from_schema(schema: u8) -> u8 {
    let level = ((schema.min(100) as u32 + 5) / 10) as u8;
    level.clamp(1, DEVICE_LEVEL_MAX)
}

/// 设备档位 1–10 → schema 亮度 0–100。
pub fn brightness_schema_from_level(level: u8) -> u8 {
    level.clamp(1, DEVICE_LEVEL_MAX) * 10
}

/// schema 色温 2000–6500 K → 设备档位 1–10。
pub fn color_temp_level_from_schema(k: u32) -> u8 {
    let k = clamp_color_temp(k as i64);
    let level = 1 + (k - COLOR_TEMP_MIN_K + COLOR_TEMP_STEP_K / 2) / COLOR_TEMP_STEP_K;
    (level as u8).clamp(1, DEVICE_LEVEL_MAX)
}

/// 设备档位 1–10 → schema 色温 2000–6500 K。
pub fn color_temp_schema_from_level(level: u8) -> u32 {
    COLOR_TEMP_MIN_K + (level.clamp(1, DEVICE_LEVEL_MAX) as u32 - 1) * COLOR_TEMP_STEP_K
}

pub fn parse_power(v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => Ok(n.as_u64().unwrap_or(0) != 0),
        _ => Err("power-state must be bool or number".into()),
    }
}

pub fn parse_brightness(v: &Value) -> Result<u8, String> {
    let n = v.as_i64().ok_or_else(|| "brightness must be number".to_string())?;
    Ok(clamp_brightness(n))
}

pub fn parse_color_temp(v: &Value) -> Result<u32, String> {
    let n = v.as_i64().ok_or_else(|| "color-temperature must be number".to_string())?;
    Ok(clamp_color_temp(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clamp_brightness_bounds() {
        assert_eq!(clamp_brightness(-1), 0);
        assert_eq!(clamp_brightness(0), 0);
        assert_eq!(clamp_brightness(50), 50);
        assert_eq!(clamp_brightness(100), 100);
        assert_eq!(clamp_brightness(200), 100);
    }

    #[test]
    fn clamp_color_temp_bounds() {
        assert_eq!(clamp_color_temp(1000), 2000);
        assert_eq!(clamp_color_temp(2000), 2000);
        assert_eq!(clamp_color_temp(4000), 4000);
        assert_eq!(clamp_color_temp(6500), 6500);
        assert_eq!(clamp_color_temp(9000), 6500);
    }

    #[test]
    fn parse_power_ok_and_err() {
        assert_eq!(parse_power(&json!(true)).unwrap(), true);
        assert_eq!(parse_power(&json!(false)).unwrap(), false);
        assert_eq!(parse_power(&json!(1)).unwrap(), true);
        assert_eq!(parse_power(&json!(0)).unwrap(), false);
        assert!(parse_power(&json!("x")).is_err());
    }

    #[test]
    fn parse_brightness_ok_and_err() {
        assert_eq!(parse_brightness(&json!(80)).unwrap(), 80);
        assert_eq!(parse_brightness(&json!(150)).unwrap(), 100);
        assert!(parse_brightness(&json!(true)).is_err());
    }

    #[test]
    fn parse_color_temp_ok_and_err() {
        assert_eq!(parse_color_temp(&json!(3000)).unwrap(), 3000);
        assert_eq!(parse_color_temp(&json!(1000)).unwrap(), 2000);
        assert_eq!(parse_color_temp(&json!(2000)).unwrap(), 2000);
        assert_eq!(parse_color_temp(&json!(6500)).unwrap(), 6500);
        assert!(parse_color_temp(&json!(true)).is_err());
        assert!(parse_color_temp(&json!("x")).is_err());
    }

    #[test]
    fn normalize_attr_id_strips_prefix() {
        assert_eq!(normalize_attr_id("power-state"), "power-state");
        assert_eq!(normalize_attr_id("xgimi.lamp#power-state"), "power-state");
        assert_eq!(normalize_attr_id("brightness"), "brightness");
        assert_eq!(normalize_attr_id("color-temperature"), "color-temperature");
    }

    #[test]
    fn brightness_schema_device_level_mapping() {
        assert_eq!(brightness_level_from_schema(0), 1);
        assert_eq!(brightness_level_from_schema(5), 1);
        assert_eq!(brightness_level_from_schema(50), 5);
        assert_eq!(brightness_level_from_schema(57), 6);
        assert_eq!(brightness_level_from_schema(100), 10);
        for level in 1..=10u8 {
            assert_eq!(brightness_level_from_schema(brightness_schema_from_level(level)), level);
        }
        assert_eq!(brightness_schema_from_level(1), 10);
        assert_eq!(brightness_schema_from_level(10), 100);
    }

    #[test]
    fn color_temp_schema_device_level_mapping() {
        assert_eq!(color_temp_level_from_schema(2000), 1);
        assert_eq!(color_temp_level_from_schema(4000), 5);
        assert_eq!(color_temp_level_from_schema(6500), 10);
        assert_eq!(color_temp_level_from_schema(1000), 1);
        assert_eq!(color_temp_level_from_schema(9000), 10);
        for level in 1..=10u8 {
            assert_eq!(color_temp_level_from_schema(color_temp_schema_from_level(level)), level);
        }
        assert_eq!(color_temp_schema_from_level(1), 2000);
        assert_eq!(color_temp_schema_from_level(10), 6500);
    }
}
