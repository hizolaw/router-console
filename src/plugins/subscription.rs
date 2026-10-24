use crate::{
    config::PluginConfig,
    model::{Health, Metric},
    plugins::{Collector, Reading},
};
use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

const MAX_BYTES: u64 = 64 * 1024;

pub struct SubscriptionCollector {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    last_attempt: Option<String>,
    last_success: Option<String>,
    // Only a trusted updater's credential-free error field is displayed.
    error: Option<String>,
    count: Option<u64>,
    attempt_status: Option<String>,
}

impl SubscriptionCollector {
    pub fn new(p: &PluginConfig) -> Self {
        Self {
            path: p.status_file.clone().unwrap_or_default(),
        }
    }

    async fn read(&self) -> Result<Status, ()> {
        let metadata = tokio::fs::metadata(&self.path).await.map_err(|_| ())?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err(());
        }
        let file = tokio::fs::File::open(&self.path).await.map_err(|_| ())?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(());
        }
        serde_json::from_slice(&bytes).map_err(|_| ())
    }
}

#[async_trait]
impl Collector for SubscriptionCollector {
    async fn collect(&mut self) -> Reading {
        let Ok(status) = self.read().await else {
            return Reading {
                health: Health::Unknown,
                summary: "订阅状态不可用".into(),
                metrics: vec![],
            };
        };
        let (health, summary) = match status.attempt_status.as_deref() {
            Some("success") => (Health::Healthy, "订阅更新正常".into()),
            Some("failed") => (
                Health::Degraded,
                status
                    .error
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "订阅更新失败".into()),
            ),
            _ => (Health::Unknown, "订阅尚未更新或本次更新已跳过".into()),
        };
        let mut metrics = Vec::new();
        for (label, value) in [
            ("最近成功", status.last_success),
            ("最近尝试", status.last_attempt),
        ] {
            if let Some(value) = value {
                metrics.push(Metric::new(label, value, ""));
            }
        }
        if let Some(count) = status.count {
            metrics.push(Metric::new("节点", count, ""));
        }
        Reading {
            health,
            summary,
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;

    fn config() -> PluginConfig {
        toml::from_str("id='subscription'\nname='Subscription'\nkind='subscription'\nstatus_file='/not/exposed/to/browser'").unwrap()
    }

    #[tokio::test]
    async fn missing_invalid_and_oversized_status_are_generic() {
        let path = std::env::temp_dir().join(format!(
            "router-console-subscription-{}",
            rand::random::<u64>()
        ));
        let mut collector = SubscriptionCollector {
            path: path.to_string_lossy().into_owned(),
        };
        let missing = collector.collect().await;
        assert_eq!(missing.health, Health::Unknown);
        assert!(!missing.summary.contains(&collector.path));
        for bytes in [b"{bad json".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            tokio::fs::write(&path, bytes).await.unwrap();
            let reading = collector.collect().await;
            assert_eq!(reading.health, Health::Unknown);
            assert_eq!(reading.summary, missing.summary);
        }
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn latest_failure_overrides_previous_success_and_snapshot_expires() {
        let path = std::env::temp_dir().join(format!(
            "router-console-subscription-{}",
            rand::random::<u64>()
        ));
        let mut collector = SubscriptionCollector {
            path: path.to_string_lossy().into_owned(),
        };
        tokio::fs::write(&path, br#"{"lastSuccess":"2026-10-25T01:00:00Z","lastAttempt":"2026-10-25T02:00:00Z","error":"subscription download failed","count":12,"attemptStatus":"failed"}"#).await.unwrap();
        let reading = collector.collect().await;
        assert_eq!(reading.health, Health::Degraded);
        assert_eq!(reading.summary, "subscription download failed");
        assert_eq!(reading.metrics.len(), 3);
        assert_eq!(reading.metrics[2].value, "12");
        let mut snapshot = Snapshot::initial(&config());
        snapshot.health = reading.health;
        snapshot.checked_at = Some(100);
        assert_eq!(snapshot.clone().expire(110).health, Health::Degraded);
        assert_eq!(snapshot.expire(146).health, Health::Unknown);
        for (json, expected) in [
            (r#"{"attemptStatus":"success"}"#, Health::Healthy),
            (r#"{"attemptStatus":"skipped"}"#, Health::Unknown),
            ("{}", Health::Unknown),
        ] {
            tokio::fs::write(&path, json).await.unwrap();
            assert_eq!(collector.collect().await.health, expected);
        }
        tokio::fs::remove_file(path).await.unwrap();
    }
}
