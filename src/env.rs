use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PluginEnv {
    pub sock: PathBuf,
    pub host_sock: String,
    pub token: String,
    pub domain: String,
    pub version: String,
    pub data_dir: PathBuf,
}

impl PluginEnv {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            sock: PathBuf::from(env::var("OLINK_PLUGIN_SOCK")?),
            host_sock: env::var("OLINK_PLUGIN_HOST_SOCK")?,
            token: env::var("OLINK_PLUGIN_TOKEN")?,
            domain: env::var("OLINK_PLUGIN_DOMAIN").unwrap_or_else(|_| "xgimi".into()),
            version: env::var("OLINK_PLUGIN_VERSION").unwrap_or_else(|_| "0.1.0".into()),
            data_dir: PathBuf::from(env::var("OLINK_PLUGIN_DATA_DIR")?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _g = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let keys = [
            "OLINK_PLUGIN_SOCK",
            "OLINK_PLUGIN_HOST_SOCK",
            "OLINK_PLUGIN_TOKEN",
            "OLINK_PLUGIN_DOMAIN",
            "OLINK_PLUGIN_VERSION",
            "OLINK_PLUGIN_DATA_DIR",
        ];
        let prev: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|k| ((*k).to_string(), std::env::var(k).ok()))
            .collect();
        for k in &keys {
            unsafe { std::env::remove_var(k) };
        }
        for (k, v) in vars {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        f();
        for (k, v) in prev {
            match v {
                Some(val) => unsafe { std::env::set_var(&k, val) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }
    }

    #[test]
    fn from_env_requires_sock() {
        with_env(
            &[
                ("OLINK_PLUGIN_SOCK", None),
                ("OLINK_PLUGIN_HOST_SOCK", Some("/tmp/host.sock")),
                ("OLINK_PLUGIN_TOKEN", Some("t")),
                ("OLINK_PLUGIN_DATA_DIR", Some("/tmp/data")),
            ],
            || assert!(PluginEnv::from_env().is_err()),
        );
    }

    #[test]
    fn from_env_defaults_domain_version() {
        with_env(
            &[
                ("OLINK_PLUGIN_SOCK", Some("/tmp/p.sock")),
                ("OLINK_PLUGIN_HOST_SOCK", Some("/tmp/h.sock")),
                ("OLINK_PLUGIN_TOKEN", Some("tok")),
                ("OLINK_PLUGIN_DATA_DIR", Some("/tmp/d")),
                ("OLINK_PLUGIN_DOMAIN", None),
                ("OLINK_PLUGIN_VERSION", None),
            ],
            || {
                let e = PluginEnv::from_env().unwrap();
                assert_eq!(e.domain, "xgimi");
                assert_eq!(e.version, "0.1.0");
                assert_eq!(e.sock.as_os_str(), "/tmp/p.sock");
                assert_eq!(e.host_sock, "/tmp/h.sock");
                assert_eq!(e.token, "tok");
                assert_eq!(e.data_dir.as_os_str(), "/tmp/d");
            },
        );
    }
}
