use crate::{
    auth::{Auth, Credentials, LoginError, same_origin},
    engine::Cache,
    model::{Snapshot, now_secs},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
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
    pub auth: std::sync::Arc<Auth>,
    pub rules: std::sync::Arc<crate::rules::RulesRpc>,
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
        .route(
            "/plugins/{id}/rules",
            get(crate::rules::get)
                .put(crate::rules::save)
                .layer(DefaultBodyLimit::max(512 * 1024)),
        )
        .route("/plugins/{id}/rules/reload", post(crate::rules::reload))
        .route_layer(middleware::from_fn_with_state(state.clone(), authorize));
    Router::new()
        .route("/api/auth", get(auth_info))
        .route("/api/login", post(login).layer(DefaultBodyLimit::max(4096)))
        .route("/api/logout", post(logout))
        .nest("/api", api)
        .fallback(asset)
        .layer(middleware::from_fn_with_state(state.clone(), headers))
        .with_state(state)
}
async fn authorize(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !authenticated(&state, request.headers()).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Authentication required"})),
        )
            .into_response();
    }
    next.run(request).await
}
async fn authenticated(state: &AppState, headers: &axum::http::HeaderMap) -> bool {
    if state.auth.system() {
        return state.auth.username(headers).await.is_some();
    }
    state.token_hash.is_none_or(|expected| {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|token| bool::from(token_hash(token).ct_eq(&expected)))
    })
}
async fn auth_info(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Json<serde_json::Value> {
    let username = if state.auth.system() {
        state.auth.username(&headers).await
    } else {
        None
    };
    let mut response = serde_json::json!({"mode": state.auth.config.mode, "authenticated": authenticated(&state, &headers).await});
    if let Some(username) = username {
        response["username"] = username.into();
    }
    Json(response)
}
async fn login(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(credentials): Json<Credentials>,
) -> Response {
    if !state.auth.system() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin(&headers, state.auth.config.secure_cookie) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match state.auth.login(credentials).await {
        Ok(cookie) => {
            let mut response = Json(serde_json::json!({"ok": true})).into_response();
            response
                .headers_mut()
                .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
            response
        }
        Err(error) => (
            match error {
                LoginError::Invalid => StatusCode::UNAUTHORIZED,
                LoginError::Limited => StatusCode::TOO_MANY_REQUESTS,
                LoginError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            },
            Json(serde_json::json!({"error": "Login unsuccessful"})),
        )
            .into_response(),
    }
}
async fn logout(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    if !state.auth.system() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin(&headers, state.auth.config.secure_cookie) {
        return StatusCode::FORBIDDEN.into_response();
    }
    state.auth.revoke(&headers).await;
    let mut response = Json(serde_json::json!({"ok": true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.auth.cookie("", true)).unwrap(),
    );
    response
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
    async fn system_mode_ignores_bearer_and_helper_failure_keeps_shell_available() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = AppState {
            cache: Arc::new(tokio::sync::RwLock::new(BTreeMap::new())),
            title: "Test".into(),
            rules: Arc::new(crate::rules::RulesRpc::new(&[])),
            token_hash: Some(token_hash("valid-token")),
            loopback_only: true,
            auth: Arc::new(Auth::new(crate::config::AuthConfig {
                mode: crate::config::AuthMode::System,
                socket: "/nonexistent/helper.sock".into(),
                ..Default::default()
            })),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, router(state)).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = format!("http://{address}");
        assert_eq!(
            client
                .get(format!("{base}/"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .get(format!("{base}/api/status"))
                .bearer_auth("valid-token")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let info = client.get(format!("{base}/api/auth")).send().await.unwrap();
        assert_eq!(info.headers()[header::CACHE_CONTROL], "no-store");
        let body: serde_json::Value = serde_json::from_str(&info.text().await.unwrap()).unwrap();
        assert_eq!(body["mode"], "system");
        assert_eq!(body["authenticated"], false);
        for (origin, expected) in [
            ("http://evil.example".to_owned(), StatusCode::FORBIDDEN),
            (base.clone(), StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = client
                .post(format!("{base}/api/login"))
                .header(header::ORIGIN, origin)
                .header(header::CONTENT_TYPE, "application/json")
                .body(r#"{"username":"alice","password":"test"}"#)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            assert!(!response.text().await.unwrap().contains("helper.sock"));
        }
        assert_eq!(
            client
                .post(format!("{base}/api/logout"))
                .header(header::ORIGIN, "http://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
    }
    #[tokio::test]
    async fn authentication_and_host_boundary() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = AppState {
            cache: Arc::new(tokio::sync::RwLock::new(BTreeMap::new())),
            title: "Test".into(),
            rules: Arc::new(crate::rules::RulesRpc::new(&[])),
            token_hash: Some(token_hash("a-long-test-token")),
            loopback_only: true,
            auth: Arc::new(Auth::new(Default::default())),
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
