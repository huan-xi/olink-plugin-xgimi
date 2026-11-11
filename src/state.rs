//! devices.json 持久化。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeviceStore {
    pub devices: Vec<DeviceConfig>,
}

impl DeviceStore {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("devices.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        let Ok(bytes) = fs::read(Self::path(data_dir)) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(data_dir)?;
        let path = Self::path(data_dir);
        let tmp = data_dir.join("devices.json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn upsert(&mut self, cfg: DeviceConfig) {
        if let Some(slot) = self.devices.iter_mut().find(|d| d.id == cfg.id) {
            *slot = cfg;
        } else {
            self.devices.push(cfg);
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        self.devices.len() != before
    }

    pub fn get(&self, id: &str) -> Option<&DeviceConfig> {
        self.devices.iter().find(|d| d.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_devices() {
        let dir = tempdir().unwrap();
        let mut store = DeviceStore::default();
        store.upsert(DeviceConfig {
            id: "aabbccddeeff".into(),
            name: "主卧神灯".into(),
            host: "192.168.1.50".into(),
            secret: None,
            pid: None,
            device_type: None,
            channel: None,
            product_name: None,
        });
        store.save(dir.path()).unwrap();
        let loaded = DeviceStore::load(dir.path());
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.devices[0].host, "192.168.1.50");
    }

    #[test]
    fn remove_device() {
        let mut store = DeviceStore::default();
        store.upsert(DeviceConfig {
            id: "a".into(),
            name: "n".into(),
            host: "1.2.3.4".into(),
            secret: None,
            pid: None,
            device_type: None,
            channel: None,
            product_name: None,
        });
        assert!(store.remove("a"));
        assert!(store.devices.is_empty());
    }

    #[test]
    fn upsert_replaces_same_id() {
        let mut store = DeviceStore::default();
        store.upsert(DeviceConfig {
            id: "a".into(),
            name: "old".into(),
            host: "1.1.1.1".into(),
            secret: None,
            pid: None,
            device_type: None,
            channel: None,
            product_name: None,
        });
        store.upsert(DeviceConfig {
            id: "a".into(),
            name: "new".into(),
            host: "2.2.2.2".into(),
            secret: None,
            pid: None,
            device_type: None,
            channel: None,
            product_name: None,
        });
        assert_eq!(store.devices.len(), 1);
        assert_eq!(store.devices[0].name, "new");
    }
}
