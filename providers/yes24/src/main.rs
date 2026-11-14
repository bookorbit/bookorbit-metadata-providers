mod author_normalizer;
mod mapping;
mod server;
mod yes24;

use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct Config {
    pub auth_token: Option<String>,
    pub yes24_api_key: String,
    pub user_agent: String,
    pub requests_per_second: u32,
    pub max_inflight_requests: usize,
}

#[tokio::main]
async fn main() {
    let port = env("PORT", "8080");
    let api_base_url = env("YES24_API_BASE_URL", yes24::DEFAULT_BASE_URL);
    let requests_per_second = env_u32("REQUESTS_PER_SECOND", yes24::DEFAULT_REQUESTS_PER_SECOND);
    let max_inflight_requests = env_usize(
        "MAX_INFLIGHT_REQUESTS",
        yes24::DEFAULT_MAX_INFLIGHT_REQUESTS,
    )
    .max(1);
    let cfg = Arc::new(Config {
        auth_token: std::env::var("AUTH_TOKEN")
            .ok()
            .filter(|v| !v.trim().is_empty()),
        yes24_api_key: std::env::var("YES24_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .expect("YES24_API_KEY must be set"),
        user_agent: env("USER_AGENT", &format!("bookorbit-yes24-provider/{VERSION}")),
        requests_per_second,
        max_inflight_requests,
    });

    let client = Arc::new(yes24::Client::with_base_url(
        &cfg.yes24_api_key,
        &cfg.user_agent,
        &api_base_url,
        requests_per_second,
        max_inflight_requests,
    ));
    let app = server::routes(cfg, client);
    let listener = tokio::net::TcpListener::bind((
        "0.0.0.0",
        port.parse::<u16>().expect("PORT must be a port number"),
    ))
    .await
    .expect("could not bind the port");
    eprintln!("[provider] [start] port={port} api={api_base_url} requests_per_second={requests_per_second} max_inflight_requests={max_inflight_requests} version={VERSION} - listening");
    axum::serve(listener, app).await.expect("server stopped");
}

fn env(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn env_u32(key: &str, fallback: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(fallback)
}

fn env_usize(key: &str, fallback: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(fallback)
}
