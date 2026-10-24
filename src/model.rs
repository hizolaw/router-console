use crate::config::{Kind, PluginConfig};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Healthy,
    Degraded,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct Metric {
    pub label: String,
    pub value: String,
    pub unit: String,
}
impl Metric {
    pub fn new(label: &str, value: impl ToString, unit: &str) -> Self {
        Self {
            label: label.into(),
            value: value.to_string(),
            unit: unit.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub visible: bool,
    pub url: Option<String>,
    pub health: Health,
    pub summary: String,
    pub metrics: Vec<Metric>,
    pub checked_at: Option<u64>,
    pub stale_after_secs: u64,
}
impl Snapshot {
    pub fn initial(p: &PluginConfig) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            kind: p.kind,
            visible: p.visible,
            url: p.url.clone(),
            health: Health::Unknown,
            summary: "等待首次检查".into(),
            metrics: vec![],
            checked_at: None,
            stale_after_secs: p.interval_secs * 3,
        }
    }
    pub fn expire(mut self, now: u64) -> Self {
        if self
            .checked_at
            .is_some_and(|t| now.saturating_sub(t) > self.stale_after_secs)
        {
            self.health = Health::Unknown;
            self.summary = "采集数据已过期，等待下一次检查".into();
        }
        self
    }
}
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_success_becomes_unknown() {
        let p = PluginConfig {
            id: "test".into(),
            name: "Test".into(),
            kind: Kind::Link,
            enabled: true,
            visible: true,
            interval_secs: 5,
            url: None,
            probe_url: None,
            token_env: None,
            dns_server: None,
            dns_name: None,
        };
        let mut s = Snapshot::initial(&p);
        s.health = Health::Healthy;
        s.checked_at = Some(100);
        assert_eq!(s.clone().expire(110).health, Health::Healthy);
        assert_eq!(s.expire(116).health, Health::Unknown);
    }
}
