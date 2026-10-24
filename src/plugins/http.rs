use crate::{
    config::PluginConfig,
    model::{Health, Metric},
    plugins::{Collector, Reading},
};
use async_trait::async_trait;
use std::time::{Duration, Instant};

pub struct HttpCollector {
    client: reqwest::Client,
    url: String,
    token: Option<String>,
}
impl HttpCollector {
    pub fn new(p: &PluginConfig) -> Result<Self, String> {
        let token = p
            .token_env
            .as_ref()
            .map(|key| {
                std::env::var(key)
                    .map_err(|_| format!("Missing token environment variable for plugin {}", p.id))
            })
            .transpose()?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .connect_timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(1)
            .build()
            .map_err(|_| "Cannot initialize HTTP client".to_string())?;
        Ok(Self {
            client,
            url: p.probe_url.clone().unwrap(),
            token,
        })
    }
}
#[async_trait]
impl Collector for HttpCollector {
    async fn collect(&mut self) -> Reading {
        let started = Instant::now();
        let mut request = self.client.get(&self.url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                Reading {
                    health: if status.is_success() {
                        Health::Healthy
                    } else {
                        Health::Failed
                    },
                    summary: if status.is_success() {
                        "HTTP 接口可达；不代表全部功能正常".into()
                    } else {
                        format!("HTTP 接口返回 {}", status.as_u16())
                    },
                    metrics: vec![
                        Metric::new("响应时间", started.elapsed().as_millis(), "ms"),
                        Metric::new("HTTP", status.as_u16(), ""),
                    ],
                }
            }
            Err(e) => Reading::failed(if e.is_timeout() {
                "HTTP 探测超时"
            } else {
                "HTTP 接口无法连接或 TLS 校验失败"
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    #[tokio::test]
    async fn detects_failure_and_does_not_follow_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/ok", get(|| async { "ok" }))
                    .route(
                        "/fail",
                        get(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
                    )
                    .route(
                        "/redirect",
                        get(|| async { axum::response::Redirect::temporary("/ok") }),
                    ),
            )
            .await
            .unwrap();
        });
        let mut p: PluginConfig = toml::from_str(&format!(
            "id='http'\nname='HTTP'\nkind='http'\nprobe_url='http://{address}/ok'\n"
        ))
        .unwrap();
        assert_eq!(
            HttpCollector::new(&p).unwrap().collect().await.health,
            Health::Healthy
        );
        for path in ["fail", "redirect"] {
            p.probe_url = Some(format!("http://{address}/{path}"));
            assert_eq!(
                HttpCollector::new(&p).unwrap().collect().await.health,
                Health::Failed
            );
        }
        server.abort();
    }
}
