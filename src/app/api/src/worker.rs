//! The background worker: takes queued runs one at a time, executes each in a
//! child process it can stop (`isolate.rs`) and stores the outcome.

use crate::config::Config;
use crate::isolate::{self, Failure};
use crate::llm::Llm;
use crate::mail::Mailer;
use crate::oauth::OAuth;
use crate::store::Store;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

pub struct AppState {
    pub store: Store,
    pub mailer: Mailer,
    /// The model behind plain-English compile; `None` switches the feature off.
    pub llm: Option<Llm>,
    /// Sign-in through Google, Apple, Facebook and GitHub; only the ones with credentials.
    pub oauth: OAuth,
    pub config: Config,
    /// Poked whenever a run is queued, so the worker starts at once.
    pub wake: Notify,
    /// Strategy checks (previews and saves) running at the same time. Each is a child
    /// process busy for up to a few seconds; more than this wait their turn.
    pub sandbox: Semaphore,
}

/// How many strategy checks may run at once.
const SANDBOX_SLOTS: usize = 4;

impl AppState {
    pub fn new(store: Store, mailer: Mailer, llm: Option<Llm>, oauth: OAuth, config: Config) -> Arc<Self> {
        Arc::new(AppState { store, mailer, llm, oauth, config, wake: Notify::new(), sandbox: Semaphore::new(SANDBOX_SLOTS) })
    }
}

/// How long the worker sleeps when the queue is empty and nobody pokes it.
/// (A safety net: runs queued by this process always poke.)
const IDLE_POLL: Duration = Duration::from_secs(2);

/// Starts the worker loop. It runs until the process exits.
pub fn spawn(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match state.store.claim().await {
                Ok(Some((id, input))) => {
                    let limit = isolate::run_limit(&input);
                    let stored = match isolate::execute(input, limit).await {
                        Ok(Ok(out)) => state.store.complete(id, &out).await,
                        Ok(Err(issues)) => state.store.fail(id, "The run could not be completed", &issues).await,
                        Err(Failure::TimedOut(limit)) => {
                            eprintln!("run {id}: stopped after {} s", limit.as_secs());
                            let message = format!("The run was stopped by the server after {} seconds: a strategy in it takes too long to decide", limit.as_secs());
                            state.store.fail(id, &message, &[]).await
                        }
                        Err(Failure::Broken(detail)) => {
                            eprintln!("run {id}: {detail}");
                            state.store.fail(id, "The simulation stopped unexpectedly", &[]).await
                        }
                    };
                    if let Err(e) = stored {
                        eprintln!("run {id}: could not store the outcome: {e}");
                        // Do not leave it "running" for ever.
                        let _ = state.store.fail(id, "The result could not be stored", &[]).await;
                    }
                }
                Ok(None) => {
                    tokio::select! {
                        _ = state.wake.notified() => {}
                        _ = tokio::time::sleep(IDLE_POLL) => {}
                    }
                }
                Err(e) => {
                    eprintln!("worker: could not read the queue: {e}");
                    tokio::time::sleep(IDLE_POLL).await;
                }
            }
        }
    })
}
