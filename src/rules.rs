use crate::{
    auth::same_origin,
    config::{Kind, PluginConfig},
    server::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::Semaphore,
};

const MAX_BYTES: usize = 512 * 1024;
pub struct RulesRpc {
    sockets: HashMap<String, String>,
    permits: Semaphore,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    revision: String,
    rules: String,
    subscriptions: Vec<Value>,
}
impl RulesRpc {
    pub fn new(plugins: &[PluginConfig]) -> Self {
        Self {
            sockets: plugins
                .iter()
                .filter(|p| p.enabled && p.kind == Kind::MihomoRules)
                .filter_map(|p| {
                    p.rpc_socket
                        .as_ref()
                        .map(|path| (p.id.clone(), path.clone()))
                })
                .collect(),
            permits: Semaphore::new(1),
        }
    }
    async fn call(&self, id: &str, action: &str, data: Option<Value>) -> Response {
        let Some(socket) = self.sockets.get(id) else {
            return error(StatusCode::NOT_FOUND, "Rules plugin unavailable");
        };
        let Ok(_permit) = self.permits.try_acquire() else {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Rules operation already in progress",
            );
        };
        let mut request = json!({"action": action});
        if let Some(data) = data {
            request["data"] = data;
        }
        let Ok(mut bytes) = serde_json::to_vec(&request) else {
            return error(StatusCode::BAD_REQUEST, "Invalid rules request");
        };
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "Rules request too large");
        }
        match tokio::time::timeout(Duration::from_secs(45), exchange(socket, &bytes)).await {
            Ok(Ok(reply)) if reply.ok => Json(reply.data.unwrap_or(Value::Null)).into_response(),
            Ok(Ok(reply)) => error(
                match reply.status {
                    Some(400) => StatusCode::BAD_REQUEST,
                    Some(409) => StatusCode::CONFLICT,
                    _ => StatusCode::SERVICE_UNAVAILABLE,
                },
                "Rules operation failed",
            ),
            _ => error(StatusCode::SERVICE_UNAVAILABLE, "Rules service unavailable"),
        }
    }
}
#[derive(Deserialize)]
struct Reply {
    ok: bool,
    data: Option<Value>,
    status: Option<u16>,
}
async fn exchange(socket: &str, request: &[u8]) -> Result<Reply, ()> {
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|_| ())?;
    stream.write_all(request).await.map_err(|_| ())?;
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_BYTES as u64 + 1))
        .read_until(b'\n', &mut line)
        .await
        .map_err(|_| ())?;
    if line.len() > MAX_BYTES || line.last() != Some(&b'\n') {
        return Err(());
    }
    serde_json::from_slice(&line).map_err(|_| ())
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}
pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    state.rules.call(&id, "get", None).await
}
pub async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Save>,
) -> Response {
    if !same_origin(&headers, state.auth.config.secure_cookie) {
        return error(StatusCode::FORBIDDEN, "Same-origin request required");
    }
    state.rules.call(&id, "save", Some(json!({"revision": body.revision, "rules": body.rules, "subscriptions": body.subscriptions}))).await
}
pub async fn reload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !same_origin(&headers, state.auth.config.secure_cookie) {
        return error(StatusCode::FORBIDDEN, "Same-origin request required");
    }
    state.rules.call(&id, "reload", None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::Auth,
        server::{router, token_hash},
    };
    use std::{collections::BTreeMap, sync::Arc};
    fn plugin(socket: &str) -> PluginConfig {
        toml::from_str(&format!(
            "id='rules'\nname='Rules'\nkind='mihomo_rules'\nrpc_socket='{socket}'"
        ))
        .unwrap()
    }
    #[tokio::test]
    async fn only_enabled_configured_plugins_can_connect_and_bounds_are_enforced() {
        let rpc = RulesRpc::new(&[]);
        assert_eq!(
            rpc.call("unknown", "get", None).await.status(),
            StatusCode::NOT_FOUND
        );
        let mut p = plugin("/nonexistent/private.sock");
        p.enabled = false;
        assert_eq!(
            RulesRpc::new(&[p.clone()])
                .call("rules", "get", None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        p.enabled = true;
        let rpc = RulesRpc::new(&[p]);
        let response = rpc.call("rules", "get", None).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("private.sock"));
        assert_eq!(
            rpc.call(
                "rules",
                "save",
                Some(json!({"rules": "x".repeat(MAX_BYTES)}))
            )
            .await
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let _permit = rpc.permits.acquire().await.unwrap();
        assert_eq!(
            rpc.call("rules", "get", None).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    #[tokio::test]
    async fn worker_protocol_and_reply_bounds() {
        let path =
            std::env::temp_dir().join(format!("console-rules-{}.sock", rand::random::<u64>()));
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let worker = tokio::spawn(async move {
            for reply in [
                b"{\"ok\":true,\"data\":{\"revision\":\"v1\"}}\n".to_vec(),
                b"{\"ok\":false,\"status\":409,\"error\":\"secret must not leak\"}\n".to_vec(),
                vec![b'x'; MAX_BYTES + 1],
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream)
                    .read_line(&mut line)
                    .await
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["action"], "save");
                assert_eq!(request["data"]["revision"], "v0");
                let _ = stream.write_all(&reply).await;
            }
        });
        let rpc = RulesRpc::new(&[plugin(path.to_str().unwrap())]);
        let data = Some(json!({"revision":"v0","rules":"MATCH,DIRECT","subscriptions":[]}));
        let success = rpc.call("rules", "save", data.clone()).await;
        assert_eq!(success.status(), StatusCode::OK);
        let body = axum::body::to_bytes(success.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["revision"],
            "v1"
        );
        let conflict = rpc.call("rules", "save", data.clone()).await;
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(conflict.into_body(), 4096)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("secret"));
        assert_eq!(
            rpc.call("rules", "save", data).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        worker.await.unwrap();
        tokio::fs::remove_file(path).await.unwrap();
    }
    #[tokio::test]
    async fn endpoints_require_authentication_and_same_origin_even_in_token_mode() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = AppState {
            cache: Arc::new(tokio::sync::RwLock::new(BTreeMap::new())),
            title: "Test".into(),
            token_hash: Some(token_hash("test-token")),
            loopback_only: true,
            auth: Arc::new(Auth::new(Default::default())),
            rules: Arc::new(RulesRpc::new(&[plugin("/nonexistent/private.sock")])),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, router(state)).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{address}/api/plugins/rules/rules");
        assert_eq!(
            client.get(&url).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .get(&url)
                .bearer_auth("test-token")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            client
                .get(format!("http://{address}/api/plugins/unknown/rules"))
                .bearer_auth("test-token")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .put(&url)
                .bearer_auth("test-token")
                .header("origin", "http://evil.example")
                .header("content-type", "application/json")
                .body(r#"{"revision":"v0","rules":"","subscriptions":[]}"#)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(format!("{url}/reload"))
                .bearer_auth("test-token")
                .header("origin", "http://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .put(&url)
                .bearer_auth("test-token")
                .header("content-type", "application/json")
                .body("x".repeat(MAX_BYTES + 1))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        server.abort();
    }
}
