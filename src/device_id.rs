#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Lan,
    Cloud,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lan => "lan",
            Self::Cloud => "cloud",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XgimiDeviceType {
    Light,
    Projector,
}

impl XgimiDeviceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Projector => "projector",
        }
    }
}

pub fn sanitize_pid(pid: &str) -> String {
    let mapped: String = pid
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if mapped.is_empty() {
        "device".into()
    } else {
        mapped
    }
}

/// Host device_key：`{pid}_{type}_{channel}`，总长 ≤ 64。
pub fn make_device_id(pid: &str, ty: XgimiDeviceType, ch: Channel) -> String {
    let suffix = format!("_{}_{}", ty.as_str(), ch.as_str());
    let budget = 64usize.saturating_sub(suffix.len()).max(1);
    let mut core = sanitize_pid(pid);
    if core.len() > budget {
        core.truncate(budget);
    }
    format!("{core}{suffix}")
}

pub fn supports_light(
    product_name: Option<&str>,
    device: Option<&str>,
    name: Option<&str>,
) -> bool {
    if product_name.is_some_and(|p| p.eq_ignore_ascii_case("Product_cp01")) {
        return true;
    }
    [device, name]
        .into_iter()
        .flatten()
        .any(|s| s.contains("神灯"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_device_id_light_lan() {
        assert_eq!(
            make_device_id("SWX5PZA97YWC", XgimiDeviceType::Light, Channel::Lan),
            "SWX5PZA97YWC_light_lan"
        );
    }

    #[test]
    fn make_device_id_sanitizes_and_differs_by_channel() {
        let lan = make_device_id("SWX5****YWC", XgimiDeviceType::Light, Channel::Lan);
        let cloud = make_device_id("SWX5****YWC", XgimiDeviceType::Light, Channel::Cloud);
        assert_eq!(lan, "SWX5____YWC_light_lan");
        assert_eq!(cloud, "SWX5____YWC_light_cloud");
        assert_ne!(lan, cloud);
        assert!(lan.len() <= 64);
    }

    #[test]
    fn supports_light_cp01_and_name() {
        assert!(supports_light(Some("Product_cp01"), None, None));
        assert!(supports_light(None, Some("极米神灯"), None));
        assert!(!supports_light(Some("Product_other"), Some("投影仪"), None));
    }
}
