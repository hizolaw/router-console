mod auth;
mod config;
mod engine;
mod model;
mod plugins;
mod rules;
mod server;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Lightweight, modular Linux router console")]
struct Args {
    #[arg(short, long, default_value = "config/local.toml")]
    config: PathBuf,
    /// Read a token from a protected file (or use ROUTER_CONSOLE_TOKEN).
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Validate configuration without starting collectors or HTTP server.
    #[arg(long)]
    check: bool,
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("router-console: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let args = Args::parse();
    let config = config::Config::load(&args.config)?;
    let token = if config.auth.mode == config::AuthMode::System {
        None
    } else if let Some(path) = args.token_file {
        Some(
            std::fs::read_to_string(path)
                .map_err(|_| "Cannot read token file".to_string())?
                .trim()
                .to_owned(),
        )
    } else {
        std::env::var("ROUTER_CONSOLE_TOKEN").ok()
    };
    if config.auth.mode == config::AuthMode::Token {
        validate_auth(config.listen, token.as_deref())?;
    }
    if args.check {
        println!("Configuration valid");
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .map_err(|e| format!("Cannot listen: {e}"))?;
    let mut engine = engine::Engine::start(&config.plugins)?;
    let state = server::AppState {
        cache: engine.cache.clone(),
        title: config.title,
        rules: std::sync::Arc::new(rules::RulesRpc::new(&config.plugins)),
        token_hash: token.as_deref().map(server::token_hash),
        loopback_only: config.listen.ip().is_loopback()
            && config.auth.mode == config::AuthMode::Token
            && token.is_none(),
        auth: std::sync::Arc::new(auth::Auth::new(config.auth)),
    };
    println!(
        "Router Console {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let result = axum::serve(listener, server::router(state))
        .with_graceful_shutdown(shutdown())
        .await;
    engine.shutdown().await;
    result.map_err(|e| e.to_string())
}
fn validate_auth(listen: std::net::SocketAddr, token: Option<&str>) -> Result<(), String> {
    if token.is_some_and(|token| token.len() < 24) {
        return Err("Authentication token must contain at least 24 bytes".into());
    }
    if !listen.ip().is_loopback() && token.is_none() {
        return Err("Non-loopback listening requires --token-file or ROUTER_CONSOLE_TOKEN".into());
    }
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lan_listener_requires_strong_token() {
        assert!(validate_auth("127.0.0.1:9080".parse().unwrap(), None).is_ok());
        assert!(validate_auth("0.0.0.0:9080".parse().unwrap(), None).is_err());
        assert!(validate_auth("0.0.0.0:9080".parse().unwrap(), Some("short")).is_err());
        assert!(
            validate_auth(
                "0.0.0.0:9080".parse().unwrap(),
                Some("a-sufficiently-long-test-token")
            )
            .is_ok()
        );
    }
}
