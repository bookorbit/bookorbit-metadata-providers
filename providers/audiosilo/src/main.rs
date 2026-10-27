mod audiosilo;
mod server;

use std::sync::Arc;

pub const VERSION: &str = "0.1.0";

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let api = std::env::var("AUDIOSILO_API").unwrap_or_else(|_| audiosilo::DEFAULT_API.to_string());
    let source = Arc::new(audiosilo::AudioSilo::new(&api));
    let port_number = port.parse::<u16>().expect("PORT must be a port number");
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port_number))
        .await
        .expect("could not bind the port");
    eprintln!("[provider] [start] port={port} api={api} version={VERSION} - listening");
    axum::serve(listener, server::routes(source))
        .await
        .expect("server stopped");
}
