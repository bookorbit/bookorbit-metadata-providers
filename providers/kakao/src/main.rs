mod author_normalizer;
mod kakao;
mod mapping;
mod server;

use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct Config {
    pub auth_token: Option<String>,
    pub kakao_rest_api_key: String,
    pub user_agent: String,
}

#[tokio::main]
async fn main() {
    let port = env("PORT", "8080");
    let cfg = Arc::new(Config {
        auth_token: std::env::var("AUTH_TOKEN").ok().filter(|v| !v.is_empty()),
        kakao_rest_api_key: std::env::var("KAKAO_REST_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .expect("KAKAO_REST_API_KEY must be set"),
        user_agent: env("USER_AGENT", &format!("bookorbit-kakao-provider/{VERSION}")),
    });

    let client = Arc::new(kakao::Client::new(&cfg.kakao_rest_api_key, &cfg.user_agent));
    let app = server::routes(cfg, client);
    let listener = tokio::net::TcpListener::bind((
        "0.0.0.0",
        port.parse::<u16>().expect("PORT must be a port number"),
    ))
    .await
    .expect("could not bind the port");
    eprintln!("[provider] [start] port={port} version={VERSION} - listening");
    axum::serve(listener, app).await.expect("server stopped");
}

fn env(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}
