//! Postgres persistence: worlds, strategies, runs (the `runs` table is also
//! the job queue), per-pairing traces and KPI snapshots.

use crate::artifacts::ArtifactMeta;
use crate::auth::{User, SESSION_DAYS, SIGN_IN_LINK_MINUTES};
use crate::strategies::{RunActor, StrategyDoc};
use lilanaya_engine::{baseline_strategies, prisoners_dilemma, Issue, RunOutput, RunResult, World};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::types::Json;
use sqlx::{PgPool, Row};
use std::str::FromStr;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Timestamps leave the database as ISO 8601 UTC strings.
const TS: &str = r#"'YYYY-MM-DD"T"HH24:MI:SS"Z"'"#;

/// The frozen inputs of a run: enough to reproduce it exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunInput {
    pub world: World,
    pub actors: Vec<RunActor>,
    pub seed: u64,
}

#[derive(Clone, Debug)]
pub struct RunRow {
    pub id: i64,
    pub world_id: String,
    pub world_name: String,
    pub status: String,
    pub seed: i64,
    pub iterations: i32,
    pub actor_count: i32,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub result: Option<RunResult>,
    pub error: Option<Value>,
    /// Who started it; `None` for runs from before accounts existed.
    pub user_id: Option<i64>,
    /// Uses a private strategy: only its owner may see it.
    pub private: bool,
}

/// A stored strategy with who owns it and who may see it.
#[derive(Clone, Debug)]
pub struct StrategyRow {
    pub doc: StrategyDoc,
    pub builtin: bool,
    pub owner_id: Option<i64>,
    pub owner_name: Option<String>,
    pub visibility: String,
    pub world_id: String,
    pub prompt: Option<String>,
    pub updated_at: String,
}

/// How many sign-in links one address may ask for per quarter of an hour.
const SIGN_IN_LINKS_PER_WINDOW: i64 = 5;

/// One KPI value of one run, with the run's parameters.
#[derive(Clone, Debug)]
pub struct KpiRow {
    pub run_id: i64,
    pub seed: i64,
    pub iterations: i32,
    pub created_at: String,
    pub kpi_id: String,
    pub label: String,
    pub value: f64,
}

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

const USER_COLUMNS: &str = "id, email, display_name, roles, plan";

fn user_row(row: PgRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        roles: row.try_get("roles")?,
        plan: row.try_get("plan")?,
    })
}

const STRATEGY_SELECT: &str = r#"SELECT s.doc, s.builtin, s.owner_id, s.visibility, s.world_id, s.prompt, u.display_name AS owner_name,
    to_char(s.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS updated_at
    FROM strategies s LEFT JOIN users u ON u.id = s.owner_id"#;

fn strategy_row(row: PgRow) -> Result<StrategyRow, sqlx::Error> {
    Ok(StrategyRow {
        doc: row.try_get::<Json<StrategyDoc>, _>("doc")?.0,
        builtin: row.try_get("builtin")?,
        owner_id: row.try_get("owner_id")?,
        owner_name: row.try_get("owner_name")?,
        visibility: row.try_get("visibility")?,
        world_id: row.try_get("world_id")?,
        prompt: row.try_get("prompt")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn run_columns() -> String {
    format!(
        "id, world_id, world_name, status, seed, iterations, actor_count, result, error, user_id, private, \
         to_char(created_at AT TIME ZONE 'UTC', {TS}) AS created_at, \
         to_char(finished_at AT TIME ZONE 'UTC', {TS}) AS finished_at"
    )
}

fn run_row(row: PgRow) -> Result<RunRow, sqlx::Error> {
    Ok(RunRow {
        id: row.try_get("id")?,
        world_id: row.try_get("world_id")?,
        world_name: row.try_get("world_name")?,
        status: row.try_get("status")?,
        seed: row.try_get("seed")?,
        iterations: row.try_get("iterations")?,
        actor_count: row.try_get("actor_count")?,
        created_at: row.try_get("created_at")?,
        finished_at: row.try_get("finished_at")?,
        result: row.try_get::<Option<Json<RunResult>>, _>("result")?.map(|j| j.0),
        error: row.try_get::<Option<Json<Value>>, _>("error")?.map(|j| j.0),
        user_id: row.try_get("user_id")?,
        private: row.try_get("private")?,
    })
}

impl Store {
    /// Connects, applies migrations and refreshes the built-in world and
    /// strategies. `schema` (used by tests) keeps everything in its own schema.
    pub async fn connect(url: &str, schema: Option<&str>) -> Result<Store, sqlx::Error> {
        let mut options = PgConnectOptions::from_str(url)?;
        if let Some(schema) = schema {
            options = options.options([("search_path", schema)]);
        }
        let pool = PgPoolOptions::new().max_connections(5).connect_with(options).await?;
        if let Some(schema) = schema {
            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"")).execute(&pool).await?;
        }
        MIGRATOR.run(&pool).await?;
        let store = Store { pool };
        store.seed_builtins().await?;
        Ok(store)
    }

    #[cfg(test)]
    pub async fn drop_schema(&self, schema: &str) {
        let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE")).execute(&self.pool).await;
    }

    async fn seed_builtins(&self) -> Result<(), sqlx::Error> {
        let world = prisoners_dilemma();
        sqlx::query(
            "INSERT INTO worlds (id, doc, builtin) VALUES ($1, $2, true) \
             ON CONFLICT (id) DO UPDATE SET doc = EXCLUDED.doc, builtin = true, updated_at = now()",
        )
        .bind(&world.id)
        .bind(Json(&world))
        .execute(&self.pool)
        .await?;
        for (position, strategy) in baseline_strategies().iter().enumerate() {
            sqlx::query(
                "INSERT INTO strategies (id, position, doc, builtin) VALUES ($1, $2, $3, true) \
                 ON CONFLICT (id) DO UPDATE SET position = EXCLUDED.position, doc = EXCLUDED.doc, builtin = true, updated_at = now()",
            )
            .bind(&strategy.id)
            .bind(position as i32)
            .bind(Json(strategy))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    // --- worlds and strategies ---

    pub async fn worlds(&self) -> Result<Vec<World>, sqlx::Error> {
        let rows = sqlx::query("SELECT doc FROM worlds ORDER BY id").fetch_all(&self.pool).await?;
        rows.into_iter().map(|r| r.try_get::<Json<World>, _>("doc").map(|j| j.0)).collect()
    }

    pub async fn world(&self, id: &str) -> Result<Option<World>, sqlx::Error> {
        let row = sqlx::query("SELECT doc FROM worlds WHERE id = $1").bind(id).fetch_optional(&self.pool).await?;
        row.map(|r| r.try_get::<Json<World>, _>("doc").map(|j| j.0)).transpose()
    }

    /// Strategies `viewer` may see: public ones and their own. Built-ins first.
    pub async fn strategies(&self, viewer: Option<i64>) -> Result<Vec<StrategyRow>, sqlx::Error> {
        let sql = format!("{STRATEGY_SELECT} WHERE s.visibility = 'public' OR s.owner_id = $1 ORDER BY s.builtin DESC, s.position, s.created_at, s.id");
        sqlx::query(&sql).bind(viewer).fetch_all(&self.pool).await?.into_iter().map(strategy_row).collect()
    }

    /// One strategy, whoever owns it; the caller decides who may see it.
    pub async fn strategy(&self, id: &str) -> Result<Option<StrategyRow>, sqlx::Error> {
        let sql = format!("{STRATEGY_SELECT} WHERE s.id = $1");
        sqlx::query(&sql).bind(id).fetch_optional(&self.pool).await?.map(strategy_row).transpose()
    }

    pub async fn insert_strategy(&self, doc: &StrategyDoc, owner: i64, visibility: &str, world_id: &str, prompt: Option<&str>) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO strategies (id, position, doc, owner_id, visibility, mode, world_id, prompt) VALUES ($1, 1000, $2, $3, $4, $5, $6, $7)")
            .bind(doc.id())
            .bind(Json(doc))
            .bind(owner)
            .bind(visibility)
            .bind(doc.mode())
            .bind(world_id)
            .bind(prompt)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Replaces a strategy its owner has edited. False if it is not theirs.
    pub async fn update_strategy(&self, doc: &StrategyDoc, owner: i64, visibility: &str, world_id: &str, prompt: Option<&str>) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE strategies SET doc = $2, visibility = $4, world_id = $5, prompt = $6, updated_at = now() \
             WHERE id = $1 AND owner_id = $3 AND NOT builtin",
        )
        .bind(doc.id())
        .bind(Json(doc))
        .bind(owner)
        .bind(visibility)
        .bind(world_id)
        .bind(prompt)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Runs keep their own frozen copy of every strategy, so deleting is safe.
    pub async fn delete_strategy(&self, id: &str, owner: i64) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("DELETE FROM strategies WHERE id = $1 AND owner_id = $2 AND NOT builtin").bind(id).bind(owner).execute(&self.pool).await?;
        Ok(done.rows_affected() == 1)
    }

    // --- accounts ---

    /// Stores a one-time sign-in token. False if this address has asked too often lately.
    pub async fn create_sign_in_token(&self, email: &str, token_hash: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(&format!(
            "INSERT INTO sign_in_tokens (token_hash, email, expires_at) \
             SELECT $1, $2, now() + interval '{SIGN_IN_LINK_MINUTES} minutes' \
             WHERE (SELECT count(*) FROM sign_in_tokens WHERE email = $2 AND created_at > now() - interval '15 minutes') < $3"
        ))
        .bind(token_hash)
        .bind(email)
        .bind(SIGN_IN_LINKS_PER_WINDOW)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Uses up a sign-in token; returns the address it was sent to if it was still good.
    pub async fn consume_sign_in_token(&self, token_hash: &str) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query("UPDATE sign_in_tokens SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING email")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| r.try_get("email")).transpose()
    }

    /// The user with this address, created on first sign-in.
    pub async fn upsert_user(&self, email: &str, default_name: &str) -> Result<User, sqlx::Error> {
        let sql = format!("INSERT INTO users (email, display_name) VALUES ($1, $2) ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email RETURNING {USER_COLUMNS}");
        sqlx::query(&sql).bind(email).bind(default_name).fetch_one(&self.pool).await.and_then(user_row)
    }

    pub async fn create_session(&self, user_id: i64, token_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query(&format!("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '{SESSION_DAYS} days')"))
            .bind(token_hash)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn session_user(&self, token_hash: &str) -> Result<Option<User>, sqlx::Error> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = (SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > now())");
        sqlx::query(&sql).bind(token_hash).fetch_optional(&self.pool).await?.map(user_row).transpose()
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(token_hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn rename_user(&self, id: i64, display_name: &str) -> Result<User, sqlx::Error> {
        let sql = format!("UPDATE users SET display_name = $2 WHERE id = $1 RETURNING {USER_COLUMNS}");
        sqlx::query(&sql).bind(id).bind(display_name).fetch_one(&self.pool).await.and_then(user_row)
    }

    #[cfg(test)]
    pub async fn set_plan(&self, id: i64, plan: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET plan = $2 WHERE id = $1").bind(id).bind(plan).execute(&self.pool).await?;
        Ok(())
    }

    /// The current month (`YYYY-MM`, UTC) and how many runs the user has started in it.
    pub async fn runs_this_month(&self, user_id: i64) -> Result<(String, i64), sqlx::Error> {
        let row = sqlx::query(
            "SELECT to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM') AS period, \
                    (SELECT count(*) FROM runs WHERE user_id = $1 AND created_at >= date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') AS used",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((row.try_get("period")?, row.try_get("used")?))
    }

    /// Notes one plain-English compile, unless the user has reached the hourly limit (then false).
    pub async fn record_llm_call(&self, user_id: i64, hourly_limit: i64) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "INSERT INTO llm_calls (user_id) SELECT $1 \
             WHERE (SELECT count(*) FROM llm_calls WHERE user_id = $1 AND created_at > now() - interval '1 hour') < $2",
        )
        .bind(user_id)
        .bind(hourly_limit)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    // --- runs ---

    /// Adds a run to the queue.
    pub async fn enqueue(&self, input: &RunInput, user_id: i64, private: bool) -> Result<RunRow, sqlx::Error> {
        let sql = format!(
            "INSERT INTO runs (world_id, world_name, seed, iterations, actor_count, input, user_id, private) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {}",
            run_columns()
        );
        let row = sqlx::query(&sql)
            .bind(&input.world.id)
            .bind(&input.world.metadata.name)
            .bind(input.seed as i64)
            .bind(input.world.time.iterations as i32)
            .bind(input.actors.len() as i32)
            .bind(Json(input))
            .bind(user_id)
            .bind(private)
            .fetch_one(&self.pool)
            .await?;
        run_row(row)
    }

    /// Takes the oldest queued run and marks it running. Safe with several
    /// workers: `SKIP LOCKED` means two never take the same run.
    pub async fn claim(&self) -> Result<Option<(i64, RunInput)>, sqlx::Error> {
        let row = sqlx::query(
            "UPDATE runs SET status = 'running', started_at = now() \
             WHERE id = (SELECT id FROM runs WHERE status = 'queued' ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1) \
             RETURNING id, input",
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| Ok((r.try_get("id")?, r.try_get::<Json<RunInput>, _>("input")?.0))).transpose()
    }

    /// Stores a finished run: summary, traces, KPI snapshots and the list of
    /// files written to object storage, all or nothing.
    pub async fn complete(&self, id: i64, out: &RunOutput, artifacts: &[ArtifactMeta]) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE runs SET status = 'completed', result = $2, finished_at = now() WHERE id = $1")
            .bind(id)
            .bind(Json(&out.result))
            .execute(&mut *tx)
            .await?;
        for trace in &out.traces {
            let bytes: Vec<u8> = trace.moves.iter().flatten().copied().collect();
            sqlx::query("INSERT INTO run_traces (run_id, a, b, moves) VALUES ($1, $2, $3, $4)")
                .bind(id)
                .bind(&trace.a)
                .bind(&trace.b)
                .bind(bytes)
                .execute(&mut *tx)
                .await?;
        }
        for kpi in &out.result.kpis {
            sqlx::query("INSERT INTO kpi_snapshots (run_id, world_id, kpi_id, label, value) VALUES ($1, $2, $3, $4, $5)")
                .bind(id)
                .bind(&out.result.world_id)
                .bind(&kpi.id)
                .bind(&kpi.label)
                .bind(kpi.value)
                .execute(&mut *tx)
                .await?;
        }
        for a in artifacts {
            sqlx::query("INSERT INTO run_artifacts (run_id, name, key, content_type, size_bytes, sha256) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(id)
                .bind(&a.name)
                .bind(&a.key)
                .bind(&a.content_type)
                .bind(a.size_bytes)
                .bind(&a.sha256)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    /// The run's manifest: every file it wrote to object storage, by name.
    pub async fn artifacts(&self, run_id: i64) -> Result<Vec<ArtifactMeta>, sqlx::Error> {
        let rows = sqlx::query("SELECT name, key, content_type, size_bytes, sha256 FROM run_artifacts WHERE run_id = $1 ORDER BY name")
            .bind(run_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| {
                Ok(ArtifactMeta {
                    name: r.try_get("name")?,
                    key: r.try_get("key")?,
                    content_type: r.try_get("content_type")?,
                    size_bytes: r.try_get("size_bytes")?,
                    sha256: r.try_get("sha256")?,
                })
            })
            .collect()
    }

    pub async fn fail(&self, id: i64, message: &str, details: &[Issue]) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE runs SET status = 'failed', error = $2, finished_at = now() WHERE id = $1")
            .bind(id)
            .bind(Json(json!({ "message": message, "details": details })))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Puts runs that were mid-flight when the process stopped back in the
    /// queue. Assumes a single API process (true today); with several, use a
    /// heartbeat instead.
    pub async fn requeue_interrupted(&self) -> Result<u64, sqlx::Error> {
        let done = sqlx::query("UPDATE runs SET status = 'queued', started_at = NULL WHERE status = 'running'")
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }

    pub async fn run(&self, id: i64) -> Result<Option<RunRow>, sqlx::Error> {
        let sql = format!("SELECT {} FROM runs WHERE id = $1", run_columns());
        sqlx::query(&sql).bind(id).fetch_optional(&self.pool).await?.map(run_row).transpose()
    }

    /// Newest first: public runs and the viewer's own.
    pub async fn recent_runs(&self, limit: i64, viewer: Option<i64>) -> Result<Vec<RunRow>, sqlx::Error> {
        let sql = format!("SELECT {} FROM runs WHERE NOT private OR user_id = $2 ORDER BY id DESC LIMIT $1", run_columns());
        sqlx::query(&sql).bind(limit).bind(viewer).fetch_all(&self.pool).await?.into_iter().map(run_row).collect()
    }

    /// The latest completed public run of a world, or the completed public run `run_id` if it belongs to that world.
    pub async fn completed_run(&self, world_id: &str, run_id: Option<i64>) -> Result<Option<RunRow>, sqlx::Error> {
        let sql = format!(
            "SELECT {} FROM runs WHERE world_id = $1 AND status = 'completed' AND NOT private AND ($2::bigint IS NULL OR id = $2) \
             ORDER BY id DESC LIMIT 1",
            run_columns()
        );
        sqlx::query(&sql).bind(world_id).bind(run_id).fetch_optional(&self.pool).await?.map(run_row).transpose()
    }

    /// The rounds of the pairing stored as `(a, b)`: one `[a's move, b's move]` per round.
    pub async fn trace(&self, run_id: i64, a: &str, b: &str) -> Result<Option<Vec<[u8; 2]>>, sqlx::Error> {
        let row = sqlx::query("SELECT moves FROM run_traces WHERE run_id = $1 AND a = $2 AND b = $3")
            .bind(run_id)
            .bind(a)
            .bind(b)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| {
            let bytes: Vec<u8> = r.try_get("moves")?;
            Ok(bytes.chunks_exact(2).map(|c| [c[0], c[1]]).collect())
        })
        .transpose()
    }

    /// KPI values of the last `limit` completed public runs of a world (up to and
    /// including `up_to`, if given), oldest run first.
    pub async fn kpi_history(&self, world_id: &str, up_to: Option<i64>, limit: i64) -> Result<Vec<KpiRow>, sqlx::Error> {
        let sql = format!(
            "SELECT k.run_id, r.seed, r.iterations, to_char(r.created_at AT TIME ZONE 'UTC', {TS}) AS created_at, \
                    k.kpi_id, k.label, k.value \
             FROM kpi_snapshots k JOIN runs r ON r.id = k.run_id \
             WHERE k.run_id IN ( \
                 SELECT id FROM runs WHERE world_id = $1 AND status = 'completed' AND NOT private AND ($2::bigint IS NULL OR id <= $2) \
                 ORDER BY id DESC LIMIT $3) \
             ORDER BY k.run_id, k.kpi_id"
        );
        let rows = sqlx::query(&sql).bind(world_id).bind(up_to).bind(limit).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| {
                Ok(KpiRow {
                    run_id: r.try_get("run_id")?,
                    seed: r.try_get("seed")?,
                    iterations: r.try_get("iterations")?,
                    created_at: r.try_get("created_at")?,
                    kpi_id: r.try_get("kpi_id")?,
                    label: r.try_get("label")?,
                    value: r.try_get("value")?,
                })
            })
            .collect()
    }
}
