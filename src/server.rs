use crate::{
    engine::Cache,
    model::{Snapshot, now_secs},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::RustEmbed;
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

#[derive(Clone)]
pub struct AppState {
    pub cache: Cache,
    pub title: String,
    pub token_hash: Option<[u8; 32]>,
    pub loopback_only: bool,
}
#[derive(Serialize)]
struct Status {
    title: String,
    version: &'static str,
    generated_at: u64,
    plugins: Vec<Snapshot>,
}
pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route_layer(middleware::from_fn_with_state(state.clone(), authorize));
    Router::new()
        .nest("/api", api)
        .fallback(asset)
        .layer(middleware::from_fn_with_state(state.clone(), headers))
        .with_state(state)
}
async fn authorize(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if let Some(expected) = state.token_hash {
        let authorized = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|token| bool::from(token_hash(token).ct_eq(&expected)));
        if !authorized {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Authentication required"})),
            )
                .into_response();
        }
    }
    next.run(request).await
}
async fn headers(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // Protect unauthenticated loopback mode against browser DNS rebinding.
    if state.loopback_only {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<axum::http::uri::Authority>().ok());
        let allowed = host.is_some_and(|h| {
            h.host() == "localhost"
                || h.host()
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !allowed {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    // The UI only uses same-origin fetch; no CORS support is intentionally exposed.
    if request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(request).await;
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
        ("cache-control", "no-store"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
async fn status(State(state): State<AppState>) -> Json<Status> {
    let now = now_secs();
    let plugins = state
        .cache
        .read()
        .await
        .values()
        .cloned()
        .map(|s| s.expire(now))
        .collect();
    Json(Status {
        title: state.title,
        version: env!("CARGO_PKG_VERSION"),
        generated_at: now,
        plugins,
    })
}
async fn asset(request: Request) -> Response {
    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(file) = Assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(file.data.into_owned()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};
    #[tokio::test]
    async fn authentication_and_host_boundary() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = AppState {
            cache: Arc::new(tokio::sync::RwLock::new(BTreeMap::new())),
            title: "Test".into(),
            token_hash: Some(token_hash("a-long-test-token")),
            loopback_only: true,
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, router(state)).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{address}/api/status");
        assert_eq!(
            client.get(&url).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .get(&url)
                .bearer_auth("wrong")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = client
            .get(&url)
            .bearer_auth("a-long-test-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("content-security-policy"));
        assert_eq!(
            client
                .get(&url)
                .header("host", "attacker.example")
                .bearer_auth("a-long-test-token")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .get(&url)
                .header("sec-fetch-site", "cross-site")
                .bearer_auth("a-long-test-token")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .get(format!("http://{address}/missing.js"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        server.abort();
    }
}
