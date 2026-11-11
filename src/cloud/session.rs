//! account.json 会话持久化。

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AccountSession {
    #[serde(default)]
    pub token: String,
    #[serde(default, rename = "accessToken")]
    pub access_token: String,
    #[serde(default, rename = "refreshToken")]
    pub refresh_token: String,
    #[serde(default, rename = "openId")]
    pub open_id: String,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub mobile: String,
    #[serde(default, rename = "deviceId")]
    pub device_id: String,
    #[serde(default, rename = "savedAt")]
    pub saved_at: u64,
}

impl AccountSession {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("account.json")
    }

    pub fn logged_in(&self) -> bool {
        !self.token.is_empty() && !self.access_token.is_empty() && !self.open_id.is_empty()
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
        let tmp = data_dir.join("account.json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(tmp, path)?;
        #[cfg(unix)]
        {
            let _ = fs::set_permissions(
                Self::path(data_dir),
                fs::Permissions::from_mode(0o600),
            );
        }
        Ok(())
    }

    pub fn clear(data_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(data_dir);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn ensure_device_id(&mut self) {
        if self.device_id.is_empty() {
            self.device_id = uuid::Uuid::new_v4().to_string().to_uppercase();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_account_session() {
        let dir = tempdir().unwrap();
        let sess = AccountSession {
            token: "jwt".into(),
            access_token: "at".into(),
            refresh_token: "rt".into(),
            open_id: "abcdef0123456789abcdef0123456789".into(),
            uid: "13469929".into(),
            mobile: "17600000000".into(),
            device_id: "ABCD".into(),
            saved_at: 1_700_000_000,
        };
        sess.save(dir.path()).unwrap();
        let loaded = AccountSession::load(dir.path());
        assert_eq!(loaded, sess);
        assert!(loaded.logged_in());
    }

    #[test]
    fn clear_removes_file() {
        let dir = tempdir().unwrap();
        let sess = AccountSession {
            token: "t".into(),
            access_token: "a".into(),
            open_id: "o".into(),
            ..Default::default()
        };
        sess.save(dir.path()).unwrap();
        AccountSession::clear(dir.path()).unwrap();
        assert!(!AccountSession::path(dir.path()).exists());
        assert!(!AccountSession::load(dir.path()).logged_in());
    }

    #[test]
    fn ensure_device_id_fills_once() {
        let mut s = AccountSession::default();
        s.ensure_device_id();
        let first = s.device_id.clone();
        assert!(!first.is_empty());
        s.ensure_device_id();
        assert_eq!(s.device_id, first);
    }
}
