mod dns;
mod http;
mod subscription;
mod system;

use crate::{
    config::{Kind, PluginConfig},
    model::{Health, Metric},
};
use async_trait::async_trait;

pub struct Reading {
    pub health: Health,
    pub summary: String,
    pub metrics: Vec<Metric>,
}
impl Reading {
    pub fn failed(summary: &str) -> Self {
        Self {
            health: Health::Failed,
            summary: summary.into(),
            metrics: vec![],
        }
    }
}
#[async_trait]
pub trait Collector: Send {
    async fn collect(&mut self) -> Reading;
}

pub fn build(p: &PluginConfig) -> Result<Box<dyn Collector>, String> {
    match p.kind {
        Kind::System => Ok(Box::new(system::SystemCollector::default())),
        Kind::Http => Ok(Box::new(http::HttpCollector::new(p)?)),
        Kind::Dns => Ok(Box::new(dns::DnsCollector::new(p))),
        Kind::Subscription => Ok(Box::new(subscription::SubscriptionCollector::new(p))),
        Kind::MihomoRules => Ok(Box::new(RulesCollector)),
        Kind::Link => Ok(Box::new(LinkCollector)),
    }
}
struct LinkCollector;
#[async_trait]
impl Collector for LinkCollector {
    async fn collect(&mut self) -> Reading {
        Reading {
            health: Health::Unknown,
            summary: "服务入口 · 未配置健康探测".into(),
            metrics: vec![],
        }
    }
}

struct RulesCollector;
#[async_trait]
impl Collector for RulesCollector {
    async fn collect(&mut self) -> Reading {
        Reading {
            health: Health::Unknown,
            summary: "规则管理 · 按需载入".into(),
            metrics: vec![],
        }
    }
}
