use crate::config::{AuthConfig, AuthMode};
use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, Semaphore},
};

const COOKIE: &str = "router_console_session";
struct Session {
    username: String,
    expires: Instant,
}
struct Failures {
    started: Instant,
    count: u32,
}
pub struct Auth {
    pub config: AuthConfig,
    sessions: Mutex<HashMap<[u8; 32], Session>>,
    attempts: Semaphore,
    failures: Mutex<Failures>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}
#[derive(Debug, PartialEq)]
pub enum LoginError {
    Invalid,
    Limited,
    Unavailable,
}
fn hash(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}
impl Auth {
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config,
            sessions: Mutex::new(HashMap::new()),
            attempts: Semaphore::new(2),
            failures: Mutex::new(Failures {
                started: Instant::now(),
                count: 0,
            }),
        }
    }
    pub fn system(&self) -> bool {
        self.config.mode == AuthMode::System
    }
    pub async fn username(&self, headers: &HeaderMap) -> Option<String> {
        let cookie = headers
            .get(header::COOKIE)?
            .to_str()
            .ok()?
            .split(';')
            .filter_map(|s| s.trim().split_once('='))
            .find(|(key, _)| *key == COOKIE)?
            .1;
        if cookie.len() != 64 || !cookie.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, s| s.expires > Instant::now());
        sessions.get(&hash(cookie)).map(|s| s.username.clone())
    }
    pub async fn revoke(&self, headers: &HeaderMap) {
        if let Some(value) = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.split(';')
                    .filter_map(|s| s.trim().split_once('='))
                    .find(|(key, _)| *key == COOKIE)
                    .map(|(_, v)| v)
            })
        {
            self.sessions.lock().await.remove(&hash(value));
        }
    }
    pub fn cookie(&self, token: &str, clear: bool) -> String {
        format!(
            "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{}",
            if clear {
                0
            } else {
                self.config.session_ttl_secs
            },
            if self.config.secure_cookie {
                "; Secure"
            } else {
                ""
            }
        )
    }
    pub async fn login(&self, credentials: Credentials) -> Result<String, LoginError> {
        if credentials.username.is_empty()
            || credentials.username.len() > 64
            || credentials.username.chars().any(char::is_control)
            || credentials.password.is_empty()
            || credentials.password.len() > 1024
        {
            return Err(LoginError::Invalid);
        }
        let _permit = self
            .attempts
            .try_acquire()
            .map_err(|_| LoginError::Limited)?;
        {
            let mut failures = self.failures.lock().await;
            if failures.started.elapsed() > Duration::from_secs(60) {
                failures.started = Instant::now();
                failures.count = 0;
            }
            if failures.count >= 8 {
                return Err(LoginError::Limited);
            }
        }
        let result = tokio::time::timeout(Duration::from_secs(4), self.verify(&credentials)).await;
        if !matches!(result, Ok(Ok(true))) {
            self.failures.lock().await.count += 1;
            return Err(if matches!(result, Ok(Ok(false))) {
                LoginError::Invalid
            } else {
                LoginError::Unavailable
            });
        }
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, s| s.expires > Instant::now());
        if sessions.len() >= 128
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, s)| s.expires)
                .map(|(k, _)| *k)
        {
            sessions.remove(&oldest);
        }
        let bytes: [u8; 32] = rand::random();
        let token = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        sessions.insert(
            hash(&token),
            Session {
                username: credentials.username,
                expires: Instant::now() + Duration::from_secs(self.config.session_ttl_secs),
            },
        );
        Ok(self.cookie(&token, false))
    }
    async fn verify(&self, credentials: &Credentials) -> Result<bool, ()> {
        let mut stream = tokio::net::UnixStream::connect(&self.config.socket)
            .await
            .map_err(|_| ())?;
        let mut request = serde_json::to_vec(credentials).map_err(|_| ())?;
        request.push(b'\n');
        stream.write_all(&request).await.map_err(|_| ())?;
        let mut line = String::new();
        BufReader::new(stream.take(1025))
            .read_line(&mut line)
            .await
            .map_err(|_| ())?;
        if line.len() > 1024 || !line.ends_with('\n') {
            return Err(());
        }
        #[derive(Deserialize)]
        struct Reply {
            ok: bool,
        }
        serde_json::from_str::<Reply>(&line)
            .map(|r| r.ok)
            .map_err(|_| ())
    }
}
pub fn same_origin(headers: &HeaderMap, secure: bool) -> bool {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let expected = format!("{}://{host}", if secure { "https" } else { "http" });
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    reqwest::Url::parse(origin)
        .ok()
        .zip(reqwest::Url::parse(&expected).ok())
        .is_some_and(|(a, b)| {
            a.origin() == b.origin()
                && a.username().is_empty()
                && a.password().is_none()
                && a.path() == "/"
                && a.query().is_none()
                && a.fragment().is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csrf_rejects_foreign_origin_and_fetch_site() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "router:9080".parse().unwrap());
        assert!(same_origin(&h, false));
        h.insert(header::ORIGIN, "http://router:9080".parse().unwrap());
        assert!(same_origin(&h, false));
        h.insert(header::ORIGIN, "http://evil.example".parse().unwrap());
        assert!(!same_origin(&h, false));
        h.remove(header::ORIGIN);
        h.insert("sec-fetch-site", "same-site".parse().unwrap());
        assert!(!same_origin(&h, false));
    }
    #[tokio::test]
    async fn helper_failure_closed_and_failures_are_globally_bounded() {
        let auth = Auth::new(AuthConfig {
            mode: AuthMode::System,
            socket: "/nonexistent/router-console-auth.sock".into(),
            ..Default::default()
        });
        for _ in 0..8 {
            assert_eq!(
                auth.login(Credentials {
                    username: "alice".into(),
                    password: "test".into()
                })
                .await,
                Err(LoginError::Unavailable)
            );
        }
        assert_eq!(
            auth.login(Credentials {
                username: "bob".into(),
                password: "test".into()
            })
            .await,
            Err(LoginError::Limited)
        );
        assert!(auth.sessions.lock().await.is_empty());
    }
    #[tokio::test]
    async fn sessions_expire_revoke_and_never_store_clear_token() {
        let auth = Auth::new(AuthConfig::default());
        let token = "a".repeat(64);
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, format!("{COOKIE}={token}").parse().unwrap());
        auth.sessions.lock().await.insert(
            hash(&token),
            Session {
                username: "alice".into(),
                expires: Instant::now() + Duration::from_secs(60),
            },
        );
        assert_eq!(auth.username(&h).await.as_deref(), Some("alice"));
        auth.revoke(&h).await;
        assert!(auth.username(&h).await.is_none());
        auth.sessions.lock().await.insert(
            hash(&token),
            Session {
                username: "alice".into(),
                expires: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(auth.username(&h).await.is_none());
        assert!(auth.sessions.lock().await.is_empty());
        assert!(
            auth.cookie(&token, false)
                .contains("HttpOnly; SameSite=Strict; Path=/")
        );
        assert!(auth.cookie("", true).contains("Max-Age=0"));
    }
    #[tokio::test]
    async fn helper_success_creates_session_and_bounds_capacity() {
        let path =
            std::env::temp_dir().join(format!("console-auth-{}.sock", rand::random::<u64>()));
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let helper = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let credentials: Credentials = serde_json::from_str(&line).unwrap();
            assert_eq!(credentials.username, "alice");
            stream.write_all(b"{\"ok\":true}\n").await.unwrap();
        });
        let auth = Auth::new(AuthConfig {
            mode: AuthMode::System,
            socket: path.to_string_lossy().into_owned(),
            ..Default::default()
        });
        for n in 0..128 {
            auth.sessions.lock().await.insert(
                hash(&n.to_string()),
                Session {
                    username: "old".into(),
                    expires: Instant::now() + Duration::from_secs(60),
                },
            );
        }
        let cookie = auth
            .login(Credentials {
                username: "alice".into(),
                password: "test".into(),
            })
            .await
            .unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            cookie.split(';').next().unwrap().parse().unwrap(),
        );
        assert_eq!(auth.username(&h).await.as_deref(), Some("alice"));
        assert_eq!(auth.sessions.lock().await.len(), 128);
        helper.await.unwrap();
        tokio::fs::remove_file(path).await.unwrap();
    }
}
