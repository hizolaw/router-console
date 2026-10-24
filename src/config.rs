use serde::Deserialize;
use std::{collections::HashSet, net::SocketAddr, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "listen")]
    pub listen: SocketAddr,
    #[serde(default = "title")]
    pub title: String,
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "interval")]
    pub interval_secs: u64,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub url_env: Option<String>,
    #[serde(default)]
    pub probe_url: Option<String>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub dns_server: Option<SocketAddr>,
    #[serde(default)]
    pub dns_name: Option<String>,
    #[serde(default)]
    pub status_file: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    System,
    Http,
    Dns,
    Link,
    Subscription,
}

fn listen() -> SocketAddr {
    "127.0.0.1:9080".parse().unwrap()
}
fn title() -> String {
    "Router Console".into()
}
fn yes() -> bool {
    true
}
fn interval() -> u64 {
    15
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("Cannot read configuration: {e}"))?;
        let cfg: Self = toml::from_str(&text)
            .map_err(|_| "Invalid TOML configuration; check field names and types".to_string())?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.plugins.len() > 32 {
            return Err("At most 32 plugins are supported".into());
        }
        if self.title.is_empty() || self.title.len() > 120 {
            return Err("Title must contain 1–120 bytes".into());
        }
        let mut ids = HashSet::new();
        for p in &self.plugins {
            if p.id.is_empty()
                || p.id.len() > 48
                || !p
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(
                    "Plugin IDs must contain 1–48 lowercase letters, digits or hyphens".into(),
                );
            }
            if !ids.insert(&p.id) {
                return Err(format!("Duplicate plugin ID: {}", p.id));
            }
            if p.name.is_empty() || p.name.len() > 120 {
                return Err(format!("Invalid display name: {}", p.id));
            }
            if !(5..=3600).contains(&p.interval_secs) {
                return Err(format!("Interval must be 5–3600 seconds: {}", p.id));
            }
            for value in [&p.url, &p.probe_url].into_iter().flatten() {
                validate_url(value).map_err(|e| format!("{e}: {}", p.id))?;
            }
            if let Some(key) = &p.url_env {
                if key.is_empty()
                    || key.len() > 128
                    || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(format!("Invalid URL environment variable name: {}", p.id));
                }
            }
            if p.enabled {
                match p.kind {
                    Kind::Http if p.probe_url.is_none() => {
                        return Err(format!("HTTP plugin needs probe_url: {}", p.id));
                    }
                    Kind::Link if p.url.is_none() && p.url_env.is_none() => {
                        return Err(format!("Link plugin needs url or url_env: {}", p.id));
                    }
                    Kind::Subscription if p.status_file.as_deref().is_none_or(str::is_empty) => {
                        return Err(format!("Subscription plugin needs status_file: {}", p.id));
                    }
                    Kind::Dns => {
                        if p.dns_server.is_none() {
                            return Err(format!("DNS plugin needs dns_server: {}", p.id));
                        }
                        let name = p
                            .dns_name
                            .as_deref()
                            .ok_or_else(|| format!("DNS plugin needs dns_name: {}", p.id))?;
                        if name.is_empty() || hickory_proto::rr::Name::from_ascii(name).is_err() {
                            return Err(format!("Invalid DNS probe name: {}", p.id));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

impl PluginConfig {
    pub fn entry_url(&self) -> Option<String> {
        self.entry_url_with(|key| std::env::var(key).ok())
    }

    fn entry_url_with(&self, lookup: impl FnOnce(&str) -> Option<String>) -> Option<String> {
        let value = match &self.url_env {
            Some(key) => lookup(key),
            None => self.url.clone(),
        }?;
        validate_url(&value).ok()?;
        Some(value)
    }
}

fn validate_url(value: &str) -> Result<(), String> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err("HTTP URL is too long or contains control characters".into());
    }
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid HTTP URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only HTTP(S) URLs without embedded credentials are allowed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_and_duplicate_plugins() {
        let mut cfg: Config = toml::from_str("[[plugins]]\nid='dns'\nname='DNS'\nkind='dns'\ndns_server='127.0.0.1:53'\ndns_name='example.com'\n").unwrap();
        assert!(cfg.validate().is_ok());
        cfg.plugins.push(cfg.plugins[0].clone());
        assert!(cfg.validate().is_err());
        cfg.plugins.pop();
        cfg.plugins[0].interval_secs = 0;
        assert!(cfg.validate().is_err());
    }
    #[test]
    fn subscription_requires_status_file_only_when_enabled() {
        let mut cfg: Config = toml::from_str(
            "[[plugins]]\nid='subscription'\nname='Subscription'\nkind='subscription'\n",
        )
        .unwrap();
        assert!(cfg.validate().is_err());
        cfg.plugins[0].enabled = false;
        assert!(cfg.validate().is_ok());
        cfg.plugins[0].enabled = true;
        cfg.plugins[0].status_file = Some("/var/lib/router-status/subscription.json".into());
        assert!(cfg.validate().is_ok());
    }
    #[test]
    fn runtime_entry_url_overrides_static_without_affecting_probe() {
        let p: PluginConfig = toml::from_str("id='mihomo'\nname='Mihomo'\nkind='http'\nprobe_url='http://localhost/health'\nurl='http://localhost/static'\nurl_env='DASHBOARD_URL'").unwrap();
        assert_eq!(
            p.entry_url_with(|_| Some("https://router/ui/?secret=runtime".into()))
                .as_deref(),
            Some("https://router/ui/?secret=runtime")
        );
        for value in [
            None,
            Some("javascript:alert(1)".into()),
            Some("http://user:password@router/".into()),
            Some("http://router/".to_owned() + &"x".repeat(8192)),
        ] {
            assert_eq!(p.entry_url_with(|_| value), None);
        }
        assert_eq!(p.probe_url.as_deref(), Some("http://localhost/health"));
        let mut static_plugin = p;
        static_plugin.url_env = None;
        assert_eq!(
            static_plugin
                .entry_url_with(|_| panic!("must not read environment"))
                .as_deref(),
            Some("http://localhost/static")
        );
    }
    #[test]
    fn link_accepts_runtime_url_even_when_environment_is_missing() {
        let cfg: Config = toml::from_str(
            "[[plugins]]\nid='dashboard'\nname='Dashboard'\nkind='link'\nurl_env='DASHBOARD_URL'",
        )
        .unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.plugins[0].entry_url_with(|_| None), None);
    }
    #[test]
    fn rejects_active_content_and_url_credentials() {
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "http://user:password@localhost/",
        ] {
            assert!(validate_url(url).is_err());
        }
        assert!(validate_url("http://127.0.0.1:9090/ui/").is_ok());
    }
}
