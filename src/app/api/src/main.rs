mod artifacts;
mod auth;
mod config;
mod error;
mod guard;
mod isolate;
mod llm;
mod mail;
mod oauth;
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

fn main() {
    // Started by ourselves to run one strategy job (see `isolate.rs`): do that and nothing else.
    if std::env::args().nth(1).as_deref() == Some(isolate::CHILD_ARG) {
        isolate::child_main();
    }
    serve();
}

/// Which part of the program this process is: `ROLE` = `api` (answers requests),
/// `worker` (runs queued runs) or `both` (the default; one process does everything,
/// which is what development uses). In production the two are separate services, so
/// that a long run cannot get in the way of requests.
fn roles() -> Result<(bool, bool), String> {
    match std::env::var("ROLE").ok().map(|v| v.trim().to_lowercase()).as_deref() {
        None | Some("") | Some("both") => Ok((true, true)),
        Some("api") => Ok((true, false)),
        Some("worker") => Ok((false, true)),
        Some(other) => Err(format!("ROLE must be api, worker or both, got \"{other}\"")),
    }
}

/// With `APP_ENV=production` the program refuses to start in a state that is only
/// safe on a developer's machine.
fn production_problems(config: &config::Config, mailer: &mail::Mailer, serves_requests: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if serves_requests && !config.secure_cookies() {
        problems.push(format!("PUBLIC_URL must be the site's https address (it is \"{}\"): without it the session cookie is sent unprotected and sign-in links point to the wrong place", config.public_url));
    }
    if serves_requests && matches!(mailer, mail::Mailer::Log) {
        problems.push("SMTP_URL must be set: without a mail server, sign-in links are printed to the log, where anyone who can read it can use them".into());
    }
    problems
}

#[tokio::main]
async fn serve() {
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set (see compose.yaml)");
    let (serves_requests, runs_worker) = roles().unwrap_or_else(|e| panic!("{e}"));
    let production = std::env::var("APP_ENV").is_ok_and(|v| v.trim() == "production");

    let config = config::Config::from_env().unwrap_or_else(|e| panic!("{e}"));
    let mailer = mail::Mailer::from_env().unwrap_or_else(|e| panic!("{e}"));
    if production {
        let problems = production_problems(&config, &mailer, serves_requests);
        if !problems.is_empty() {
            panic!("not starting in production:\n- {}", problems.join("\n- "));
        }
    }
    let llm = llm::Llm::from_env().unwrap_or_else(|e| panic!("{e}"));
    let oauth = oauth::OAuth::from_env().unwrap_or_else(|e| panic!("{e}"));
    if serves_requests {
        match &llm {
            Some(llm) => println!("plain-English strategy compile uses {}", llm.describe()),
            None => println!("AI_GATEWAY_API_KEY is not set (see src/app/api/src/llm.rs): plain-English strategy compile is switched off"),
        }
        println!("{}", oauth.describe());
    }

    let store = connect(&database_url).await;
    let state = worker::AppState::new(store, mailer, llm, oauth, config);
    let worker = if runs_worker {
        // Only the worker puts interrupted runs back: when the two are separate, a restart
        // of the API must not re-queue a run the worker is in the middle of.
        match state.store.requeue_interrupted().await {
            Ok(0) => {}
            Ok(n) => println!("re-queued {n} run(s) interrupted by the last shutdown"),
            Err(e) => eprintln!("could not re-queue interrupted runs: {e}"),
        }
        Some(worker::spawn(state.clone()))
    } else {
        None
    };

    if !serves_requests {
        println!("lilanaya worker started: running queued runs");
        shutdown_signal().await;
        return;
    }
    drop(worker);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!("lilanaya-api listening on {addr}{}", if runs_worker { "" } else { " (runs are left to a separate worker)" });
    axum::serve(listener, routes::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_needs_https_and_a_mail_server_where_requests_are_served() {
        let local = config::Config::for_tests();
        let live = config::Config { public_url: "https://lilanaya.example".into(), ..config::Config::for_tests() };
        let memory = mail::Mailer::Memory(Default::default());
        assert_eq!(production_problems(&live, &memory, true), Vec::<String>::new());
        let problems = production_problems(&local, &mail::Mailer::Log, true);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("PUBLIC_URL") && problems[1].contains("SMTP_URL"));
        // The worker sends no mail and sets no cookies.
        assert_eq!(production_problems(&local, &mail::Mailer::Log, false), Vec::<String>::new());
    }
}
