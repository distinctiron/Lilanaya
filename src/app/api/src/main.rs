use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

fn app() -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/hello", get(hello))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn hello() -> Json<Value> {
    Json(json!({ "message": "Hello from Lilanaya!" }))
}

/// Resolves on SIGINT or SIGTERM so `docker stop` shuts the server down promptly.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!("lilanaya-api listening on {addr}");
    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hello_returns_message() {
        let Json(body) = hello().await;
        assert_eq!(body["message"], "Hello from Lilanaya!");
    }

    #[tokio::test]
    async fn health_reports_ok() {
        let Json(body) = health().await;
        assert_eq!(body["status"], "ok");
    }
}
