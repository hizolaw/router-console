use crate::{
    config::PluginConfig,
    model::{Health, Snapshot, now_secs},
    plugins::{self, Collector, Reading},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    sync::{RwLock, Semaphore},
    task::JoinSet,
    time::{sleep, timeout},
};

pub type Cache = Arc<RwLock<BTreeMap<String, Snapshot>>>;
pub struct Engine {
    pub cache: Cache,
    tasks: JoinSet<()>,
}
impl Engine {
    pub fn start(configs: &[PluginConfig]) -> Result<Self, String> {
        // Build everything before spawning, so partial initialization cannot leak tasks.
        let collectors = configs
            .iter()
            .filter(|p| p.enabled)
            .map(|p| plugins::build(p).map(|collector| (p.clone(), collector)))
            .collect::<Result<Vec<_>, _>>()?;
        let cache: Cache = Arc::new(RwLock::new(
            collectors
                .iter()
                .map(|(p, _)| (p.id.clone(), Snapshot::initial(p)))
                .collect(),
        ));
        let permits = Arc::new(Semaphore::new(4));
        let mut tasks = JoinSet::new();
        for (config, collector) in collectors {
            tasks.spawn(run(config, collector, cache.clone(), permits.clone()));
        }
        Ok(Self { cache, tasks })
    }
    pub async fn shutdown(&mut self) {
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
    }
}
async fn run(
    config: PluginConfig,
    mut collector: Box<dyn Collector>,
    cache: Cache,
    permits: Arc<Semaphore>,
) {
    let mut failures = 0u32;
    loop {
        let Ok(_permit) = permits.acquire().await else {
            return;
        };
        let result = timeout(Duration::from_secs(4), collector.collect())
            .await
            .unwrap_or_else(|_| Reading::failed("探测超时（4 秒）"));
        failures = if result.health == Health::Failed {
            (failures + 1).min(3)
        } else {
            0
        };
        let delay = config
            .interval_secs
            .saturating_mul(1 << failures)
            .min(config.interval_secs.max(300));
        let mut snapshot = Snapshot::initial(&config);
        snapshot.stale_after_secs = snapshot.stale_after_secs.max(delay + 10);
        snapshot.health = result.health;
        snapshot.summary = result.summary;
        snapshot.metrics = result.metrics;
        snapshot.checked_at = Some(now_secs());
        cache.write().await.insert(config.id.clone(), snapshot);
        drop(_permit);
        sleep(Duration::from_secs(delay)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Stuck;
    #[async_trait::async_trait]
    impl Collector for Stuck {
        async fn collect(&mut self) -> Reading {
            std::future::pending().await
        }
    }
    #[tokio::test(start_paused = true)]
    async fn stuck_probe_times_out_and_releases_capacity() {
        let p: PluginConfig = toml::from_str(
            "id='stuck'\nname='Stuck'\nkind='link'\ninterval_secs=5\nurl='http://localhost/'",
        )
        .unwrap();
        let cache: Cache = Arc::new(RwLock::new(BTreeMap::new()));
        let permits = Arc::new(Semaphore::new(1));
        let task = tokio::spawn(run(p, Box::new(Stuck), cache.clone(), permits.clone()));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert_eq!(cache.read().await["stuck"].health, Health::Failed);
        assert_eq!(permits.available_permits(), 1);
        task.abort();
    }
    #[tokio::test]
    async fn disabled_plugins_are_not_constructed_and_hidden_plugins_remain() {
        let config: crate::config::Config = toml::from_str("[[plugins]]\nid='disabled'\nname='Disabled'\nkind='http'\nenabled=false\n[[plugins]]\nid='hidden'\nname='Hidden'\nkind='link'\nvisible=false\nurl='http://localhost/'\n").unwrap();
        config.validate().unwrap();
        let mut engine = Engine::start(&config.plugins).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let cache = engine.cache.read().await;
        assert!(!cache.contains_key("disabled"));
        assert!(!cache["hidden"].visible);
        assert!(cache["hidden"].checked_at.is_some());
        drop(cache);
        engine.shutdown().await;
    }
}
