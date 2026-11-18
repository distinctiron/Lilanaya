mod artifacts;
mod auth;
mod config;
mod error;
mod llm;
mod mail;
mod routes;
mod sandbox;
mod store;
mod strategies;
mod worker;

use std::time::Duration;
use store::Store;

/// Resolves on SIGINT or SIGTERM so `docker stop` shuts the server down promptly.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

/// The database may still be starting when we are; keep trying for a minute.
async fn connect(url: &str) -> Store {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match Store::connect(url, None).await {
            Ok(store) => return store,
            Err(e) if attempt < 60 => {
                eprintln!("database not ready (attempt {attempt}): {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => panic!("could not connect to the database: {e}"),
        }
    }
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set (see compose.yaml)");

    let store = connect(&database_url).await;
    match store.requeue_interrupted().await {
        Ok(0) => {}
        Ok(n) => println!("re-queued {n} run(s) interrupted by the last shutdown"),
        Err(e) => eprintln!("could not re-queue interrupted runs: {e}"),
    }
    let s3 = artifacts::S3Config::from_env().unwrap_or_else(|e| panic!("{e}"));
    let artifact_store = artifacts::ArtifactStore::new(&s3).unwrap_or_else(|e| panic!("object storage configuration: {e}"));
    let config = config::Config::from_env().unwrap_or_else(|e| panic!("{e}"));
    let mailer = mail::Mailer::from_env().unwrap_or_else(|e| panic!("{e}"));
    let llm = llm::Llm::from_env().unwrap_or_else(|e| panic!("{e}"));
    match &llm {
        Some(llm) => println!("plain-English strategy compile uses {}", llm.describe()),
        None => println!("no language model is configured (see src/app/api/src/llm.rs): plain-English strategy compile is switched off"),
    }
    let state = worker::AppState::new(store, artifact_store, mailer, llm, config);
    worker::spawn(state.clone());

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!("lilanaya-api listening on {addr}");
    axum::serve(listener, routes::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}
