//! Run artifacts in S3-compatible object storage.
//!
//! Each completed run writes, under `runs/{run_id}/`:
//! `run_summary.json`, `pairwise_matrix.json`, `traces/{a}__{b}.json` (one per
//! pairing) and `manifest.json`, which lists the others with size and SHA-256.
//! Files are private; the API hands out short-lived signed URLs.

use lilanaya_engine::{RunOutput, RunResult};
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path;
use object_store::signer::Signer;
use object_store::{Attribute, Attributes, ObjectStore, PutOptions, PutPayload};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// How long a signed download link stays valid.
pub const SIGNED_URL_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct S3Config {
    /// Where the API reaches the storage (inside the container network).
    pub endpoint: String,
    /// Where a browser reaches it; signed URLs are made for this address.
    pub public_endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Prepended to every key (tests use it to stay out of each other's way).
    pub prefix: String,
}

impl S3Config {
    /// Reads `S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`
    /// (required) and `S3_PUBLIC_ENDPOINT`, `S3_REGION` (optional).
    pub fn from_env() -> Result<Self, String> {
        let need = |name: &str| std::env::var(name).map_err(|_| format!("{name} must be set (see compose.yaml)"));
        let endpoint = need("S3_ENDPOINT")?;
        Ok(S3Config {
            public_endpoint: std::env::var("S3_PUBLIC_ENDPOINT").unwrap_or_else(|_| endpoint.clone()),
            endpoint,
            bucket: need("S3_BUCKET")?,
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key_id: need("S3_ACCESS_KEY_ID")?,
            secret_access_key: need("S3_SECRET_ACCESS_KEY")?,
            prefix: String::new(),
        })
    }
}

/// One stored file, as recorded in the manifest and in `run_artifacts`.
#[derive(Clone, Debug, Serialize)]
pub struct ArtifactMeta {
    pub name: String,
    pub key: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

#[derive(Clone)]
pub struct ArtifactStore {
    store: AmazonS3,
    /// Same bucket and credentials, addressed by the public endpoint. Only signs; never connects.
    signer: AmazonS3,
    prefix: String,
}

const JSON: &str = "application/json";

impl ArtifactStore {
    pub fn new(config: &S3Config) -> Result<Self, object_store::Error> {
        let build = |endpoint: &str| {
            AmazonS3Builder::new()
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"))
                .with_bucket_name(&config.bucket)
                .with_region(&config.region)
                .with_access_key_id(&config.access_key_id)
                .with_secret_access_key(&config.secret_access_key)
                // Path-style addresses (endpoint/bucket/key) work with every S3-compatible server.
                .with_virtual_hosted_style_request(false)
                .build()
        };
        Ok(ArtifactStore { store: build(&config.endpoint)?, signer: build(&config.public_endpoint)?, prefix: config.prefix.clone() })
    }

    fn key(&self, run_id: i64, name: &str) -> String {
        format!("{}runs/{run_id}/{name}", self.prefix)
    }

    /// Writes every artifact of a finished run, the manifest last, and returns what was written.
    pub async fn write_run(&self, run_id: i64, out: &RunOutput) -> Result<Vec<ArtifactMeta>, object_store::Error> {
        let mut written = Vec::new();
        for (name, bytes) in run_files(run_id, out) {
            written.push(self.put(run_id, &name, bytes).await?);
        }
        let manifest = json!({
            "run_id": run_id,
            "world_id": out.result.world_id,
            "seed": out.result.seed,
            "iterations": out.result.iterations,
            "policy": out.result.policy,
            "schedule_mode": out.result.schedule_mode,
            "artifacts": written,
        });
        written.push(self.put(run_id, "manifest.json", pretty(&manifest)).await?);
        Ok(written)
    }

    async fn put(&self, run_id: i64, name: &str, bytes: Vec<u8>) -> Result<ArtifactMeta, object_store::Error> {
        let key = self.key(run_id, name);
        let meta = ArtifactMeta {
            name: name.to_string(),
            key: key.clone(),
            content_type: JSON.to_string(),
            size_bytes: bytes.len() as i64,
            sha256: Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect(),
        };
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, JSON.into());
        let options = PutOptions { attributes, ..PutOptions::default() };
        self.store.put_opts(&Path::from(key), PutPayload::from(bytes), options).await?;
        Ok(meta)
    }

    /// Reads a stored object back (used by tests to check what was written).
    #[cfg(test)]
    pub async fn read(&self, key: &str) -> Result<Vec<u8>, object_store::Error> {
        use object_store::ObjectStoreExt;
        Ok(self.store.get(&Path::from(key)).await?.bytes().await?.to_vec())
    }

    /// A time-limited download link for a stored object.
    pub async fn signed_url(&self, key: &str) -> Result<String, object_store::Error> {
        let url = self.signer.signed_url(axum::http::Method::GET, &Path::from(key), SIGNED_URL_TTL).await?;
        Ok(url.to_string())
    }
}

fn pretty(value: &impl Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialise artifact");
    bytes.push(b'\n');
    bytes
}

/// The files of a run, apart from the manifest: `(name, contents)`.
fn run_files(run_id: i64, out: &RunOutput) -> Vec<(String, Vec<u8>)> {
    let r = &out.result;
    let mut files = vec![
        ("run_summary.json".to_string(), pretty(&json!({ "run_id": run_id, "summary": r }))),
        (
            "pairwise_matrix.json".to_string(),
            pretty(&json!({
                "run_id": run_id,
                "world_id": r.world_id,
                "seed": r.seed,
                "iterations": r.iterations,
                "actors": r.actors.iter().map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>(),
                // average_payoff[row][col]: the row actor's average per round against the column actor.
                "average_payoff": r.pairwise,
                "pairs": pair_metrics(r),
            })),
        ),
    ];
    for trace in &out.traces {
        let body = json!({
            "run_id": run_id,
            "a": trace.a,
            "b": trace.b,
            "decisions": r.decisions,
            // Per round: [a's decision, b's decision] as indices into `decisions`.
            "moves": trace.moves,
        });
        // Compact: traces are the bulk of a run's bytes.
        let mut bytes = serde_json::to_vec(&body).expect("serialise trace");
        bytes.push(b'\n');
        files.push((format!("traces/{}__{}.json", trace.a, trace.b), bytes));
    }
    files
}

/// Per-pairing metrics: average payoffs, outcome mix, first defection round, punish duration.
pub fn pair_metrics(r: &RunResult) -> Vec<Value> {
    // "Defection" only means something in worlds that have a decision called D.
    let has_defect = r.decisions.iter().any(|d| d == "D");
    r.matches
        .iter()
        .map(|m| {
            let (first_defection, punish) = if has_defect {
                let first = m.first_played.get("D").and_then(|sides| sides.iter().flatten().min().copied());
                let rounds = m.outcome_counts.get("D/D").copied().unwrap_or(0);
                let streaks = m.outcome_runs.get("D/D").copied().unwrap_or(0);
                (json!(first), if streaks == 0 { Value::Null } else { json!(rounds as f64 / streaks as f64) })
            } else {
                (Value::Null, Value::Null)
            };
            json!({
                "a": m.a,
                "b": m.b,
                "average_payoff": m.averages,
                "outcome_counts": m.outcome_counts,
                // The first round (from 0) in which either actor defected; null if neither ever did.
                "first_defection_round": first_defection,
                // Average length of an unbroken stretch of mutual defection; null if there was none.
                "punish_duration_average": punish,
            })
        })
        .collect()
}
