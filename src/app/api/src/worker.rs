//! The background worker: takes queued runs one at a time, executes them off
//! the async threads, writes their artifacts to object storage and stores the outcome.

use crate::artifacts::ArtifactStore;
use crate::config::Config;
use crate::llm::Llm;
use crate::mail::Mailer;
use crate::store::Store;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

pub struct AppState {
    pub store: Store,
    pub artifacts: ArtifactStore,
    pub mailer: Mailer,
    /// The model behind plain-English compile; `None` switches the feature off.
    pub llm: Option<Llm>,
    pub config: Config,
    /// Poked whenever a run is queued, so the worker starts at once.
    pub wake: Notify,
}

impl AppState {
    pub fn new(store: Store, artifacts: ArtifactStore, mailer: Mailer, llm: Option<Llm>, config: Config) -> Arc<Self> {
        Arc::new(AppState { store, artifacts, mailer, llm, config, wake: Notify::new() })
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
                    let outcome =
                        tokio::task::spawn_blocking(move || crate::strategies::execute(&input)).await;
                    let stored = match outcome {
                        // A run is only "completed" once its files are in object storage.
                        Ok(Ok(out)) => match state.artifacts.write_run(id, &out).await {
                            Ok(written) => state.store.complete(id, &out, &written).await,
                            Err(e) => {
                                eprintln!("run {id}: could not write artifacts: {e}");
                                state.store.fail(id, "The run's files could not be written to storage", &[]).await
                            }
                        },
                        Ok(Err(issues)) => state.store.fail(id, "The run could not be completed", &issues).await,
                        Err(panic) => {
                            eprintln!("run {id}: engine panicked: {panic}");
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
