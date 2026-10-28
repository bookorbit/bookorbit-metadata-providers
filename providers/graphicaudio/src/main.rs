mod graphicaudio;
mod mapping;
mod server;

use std::sync::Arc;

pub const VERSION: &str = "0.1.0";

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let auth_token = std::env::var("AUTH_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    let source = Arc::new(graphicaudio::GraphicAudio::new());
    let port_number = port.parse::<u16>().expect("PORT must be a port number");
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port_number))
        .await
        .expect("could not bind the port");
    eprintln!("[provider] [start] port={port} version={VERSION} - listening");
    axum::serve(listener, server::routes(source, auth_token))
        .await
        .expect("server stopped");
}
