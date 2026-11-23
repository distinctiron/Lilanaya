//! Postgres persistence: worlds, strategies, runs (the `runs` table is also
//! the job queue), per-pairing traces and KPI snapshots.

use crate::auth::{User, SESSION_DAYS, SIGN_IN_LINK_MINUTES};
use crate::config::Allowance;
use crate::llm::Spent;
use crate::strategies::{RunActor, StrategyDoc};
use lilanaya_engine::{baseline_strategies, prisoners_dilemma, Issue, MatchTrace, RunOutput, RunResult, World};
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

/// What became of a request to queue a run.
#[derive(Debug)]
pub enum Queued {
    Run(RunRow),
    /// The user has started as many runs this month as their plan allows.
    MonthlyLimit,
    /// The user already has as many runs queued or running as one user may.
    TooManyActive,
}

/// The limits a plain-English compile is held to.
#[derive(Clone, Copy, Debug)]
pub struct CompileLimits {
    /// Compiles one user may ask for in an hour.
    pub per_hour: i64,
    /// Descriptions the screen may turn down in 24 hours before the user is shut out.
    pub blocked_per_day: i64,
    /// Compiles all users together may ask for in a day (UTC).
    pub per_day: i64,
    /// Tokens all compiles together may use in a day (UTC).
    pub tokens_per_day: i64,
    /// The AI requests the user's plan includes.
    pub allowance: Allowance,
}

/// Whether a compile may go ahead.
#[derive(Debug, PartialEq)]
pub enum CompileGate {
    /// It may; the id of its row in `llm_calls`, to be completed with [`Store::finish_llm_call`].
    Started(i64),
    /// The plan's AI requests are used up (for this world, or for this month).
    AllowanceUsed,
    HourlyLimit,
    /// The screen has turned this user down too often lately.
    Suspended,
    /// All users together have reached the day's ceiling.
    DailyCeiling,
}

// Advisory-lock namespaces. A limit is "count, then insert"; taking the lock first makes
// that one step, so requests arriving together cannot all pass on the same count.

const LOCK_RUNS: i32 = 1;
const LOCK_PREVIEWS: i32 = 2;
const LOCK_SIGN_IN: i32 = 3;
const LOCK_COMPILE: i32 = 4;
const LOCK_STRATEGIES: i32 = 5;
const TAKE_LOCK: &str = "SELECT pg_advisory_xact_lock($1, $2)";

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

/// True of a row of `users` whose subscription at Stripe is paid for: it is alive, and the period last
/// paid for has not run out. The day's grace covers the hour or so Stripe takes to collect a renewal;
/// during it the old period's allowance still applies, so it opens nothing new.
const PAID_SUBSCRIPTION: &str = "EXISTS (SELECT 1 FROM subscriptions sub WHERE sub.user_id = users.id AND sub.status IN ('active', 'past_due') AND sub.period_end + interval '1 day' > now())";

/// The plan is worked out, not stored: a subscriber is someone whose subscription is paid for
/// (or whose `plan` was set by hand, for accounts given away).
fn user_columns() -> String {
    format!("users.id, users.email, users.display_name, users.roles, CASE WHEN users.plan = 'subscriber' OR {PAID_SUBSCRIPTION} THEN 'subscriber' ELSE 'free' END AS plan")
}

/// When the allowance of user `$1` started to be counted: the start of the paid period for a paying
/// subscriber, of the calendar month (UTC) for everyone else.
const PERIOD_START: &str = "coalesce((SELECT sub.period_start FROM subscriptions sub WHERE sub.user_id = $1 AND sub.status IN ('active', 'past_due') AND sub.period_end + interval '1 day' > now()), date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC')";
/// When it starts again: the end of the paid period, or the first of next month.
const PERIOD_END: &str = "coalesce((SELECT sub.period_end FROM subscriptions sub WHERE sub.user_id = $1 AND sub.status IN ('active', 'past_due') AND sub.period_end + interval '1 day' > now()), (date_trunc('month', now() AT TIME ZONE 'UTC') + interval '1 month') AT TIME ZONE 'UTC')";

/// What is left of the packs user `$1` has bought: what they held, less what was taken back with a refund,
/// less what was used. What was used never comes back, and the balance never goes below nothing.
const PACK_STRATEGIES_LEFT: &str = "greatest(0, (SELECT coalesce(sum(strategies - refunded_strategies), 0) FROM purchases WHERE user_id = $1) - (SELECT count(*) FROM llm_calls WHERE user_id = $1 AND from_pack AND counted))::bigint";
const PACK_RUNS_LEFT: &str = "greatest(0, (SELECT coalesce(sum(runs - refunded_runs), 0) FROM purchases WHERE user_id = $1) - (SELECT count(*) FROM runs WHERE user_id = $1 AND from_pack))::bigint";

/// AI-written strategies used in world `$2` by accounts that were since deleted under the address of user `$1`.
/// Deleting an account and signing up again does not bring the free ones back.
const CARRIED_OVER: &str = "(SELECT coalesce(sum(used), 0) FROM spent_free_strategies WHERE world_id = $2 AND email_hash = (SELECT encode(sha256(convert_to(email, 'UTF8')), 'hex') FROM users WHERE id = $1))::bigint";

/// A user's subscription as last read from Stripe.
#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionRow {
    pub stripe_id: String,
    pub status: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub cancel_at_period_end: bool,
    /// It is paid for right now: this is what makes the user a subscriber.
    pub paid: bool,
}

/// A pack a user bought.
#[derive(Clone, Debug)]
pub struct PurchaseRow {
    pub pack: String,
    pub strategies: i64,
    pub runs: i64,
    pub amount: i64,
    pub currency: String,
    pub created_at: String,
    pub refunded: bool,
}

/// The window a user's allowance is counted over, and what they have used in it.
#[derive(Clone, Debug)]
pub struct Usage {
    /// The calendar month (`YYYY-MM`, UTC) for those counted by month.
    pub period: String,
    pub period_start: String,
    pub period_end: String,
    /// Runs started in the window on the plan's allowance.
    pub runs_used: i64,
    pub pack_strategies_left: i64,
    pub pack_runs_left: i64,
}

fn user_row(row: PgRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        roles: row.try_get("roles")?,
        plan: row.try_get("plan")?,
    })
}

fn subscription_row(row: PgRow) -> Result<SubscriptionRow, sqlx::Error> {
    Ok(SubscriptionRow {
        stripe_id: row.try_get("stripe_subscription_id")?,
        status: row.try_get("status")?,
        period_start: row.try_get("period_start")?,
        period_end: row.try_get("period_end")?,
        cancel_at_period_end: row.try_get("cancel_at_period_end")?,
        paid: row.try_get("paid")?,
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
        // Postgres cannot hold the NUL character, so nothing is stored under a name with one in it.
        if id.contains('\0') {
            return Ok(None);
        }
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
        if id.contains('\0') {
            return Ok(None);
        }
        let sql = format!("{STRATEGY_SELECT} WHERE s.id = $1");
        sqlx::query(&sql).bind(id).fetch_optional(&self.pool).await?.map(strategy_row).transpose()
    }

    /// Saves a new strategy. False if its owner already has `limit` of them.
    pub async fn insert_strategy(&self, doc: &StrategyDoc, owner: i64, visibility: &str, world_id: &str, prompt: Option<&str>, limit: i64) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(TAKE_LOCK).bind(LOCK_STRATEGIES).bind(owner as i32).execute(&mut *tx).await?;
        let done = sqlx::query(
            "INSERT INTO strategies (id, position, doc, owner_id, visibility, mode, world_id, prompt) \
             SELECT $1, 1000, $2, $3, $4, $5, $6, $7 WHERE (SELECT count(*) FROM strategies WHERE owner_id = $3) < $8",
        )
        .bind(doc.id())
        .bind(Json(doc))
        .bind(owner)
        .bind(visibility)
        .bind(doc.mode())
        .bind(world_id)
        .bind(prompt)
        .bind(limit)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(done.rows_affected() == 1)
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
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2))").bind(LOCK_SIGN_IN).bind(email).execute(&mut *tx).await?;
        let done = sqlx::query(&format!(
            "INSERT INTO sign_in_tokens (token_hash, email, expires_at) \
             SELECT $1, $2, now() + interval '{SIGN_IN_LINK_MINUTES} minutes' \
             WHERE (SELECT count(*) FROM sign_in_tokens WHERE email = $2 AND created_at > now() - interval '15 minutes') < $3"
        ))
        .bind(token_hash)
        .bind(email)
        .bind(SIGN_IN_LINKS_PER_WINDOW)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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
    /// Whether this address has an account.
    pub async fn user_exists(&self, email: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE email = $1)").bind(email).fetch_one(&self.pool).await
    }

    pub async fn upsert_user(&self, email: &str, default_name: &str) -> Result<User, sqlx::Error> {
        let sql = format!("INSERT INTO users (email, display_name) VALUES ($1, $2) ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email RETURNING {}", user_columns());
        sqlx::query(&sql).bind(email).bind(default_name).fetch_one(&self.pool).await.and_then(user_row)
    }

    /// The user this outside account (say, Google account 123) signs in as, if it has signed in before.
    pub async fn identity_user(&self, provider: &str, subject: &str) -> Result<Option<User>, sqlx::Error> {
        let sql = format!("SELECT {} FROM users WHERE id = (SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2)", user_columns());
        sqlx::query(&sql).bind(provider).bind(subject).fetch_optional(&self.pool).await?.map(user_row).transpose()
    }

    /// From now on this outside account signs in as this user.
    pub async fn link_identity(&self, provider: &str, subject: &str, user_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO user_identities (provider, subject, user_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING").bind(provider).bind(subject).bind(user_id).execute(&self.pool).await?;
        Ok(())
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
        let sql = format!("SELECT {} FROM users WHERE id = (SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > now())", user_columns());
        sqlx::query(&sql).bind(token_hash).fetch_optional(&self.pool).await?.map(user_row).transpose()
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(token_hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn rename_user(&self, id: i64, display_name: &str) -> Result<User, sqlx::Error> {
        let sql = format!("UPDATE users SET display_name = $2 WHERE id = $1 RETURNING {}", user_columns());
        sqlx::query(&sql).bind(id).bind(display_name).fetch_one(&self.pool).await.and_then(user_row)
    }

    /// Makes every row of `table` look `interval` older (for example `"2 hours"`), so a test
    /// can stand on the far side of a time limit without waiting for it.
    #[cfg(test)]
    pub async fn backdate(&self, table: &str, interval: &str) {
        let expires = if matches!(table, "sessions" | "sign_in_tokens") { format!(", expires_at = expires_at - interval '{interval}'") } else { String::new() };
        sqlx::query(&format!("UPDATE {table} SET created_at = created_at - interval '{interval}'{expires}")).execute(&self.pool).await.unwrap_or_else(|e| panic!("backdate {table}: {e}"));
    }

    #[cfg(test)]
    pub async fn set_plan(&self, id: i64, plan: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET plan = $2 WHERE id = $1").bind(id).bind(plan).execute(&self.pool).await?;
        Ok(())
    }

    /// The window the user's allowance is counted over, the runs used in it, and what is left of their packs.
    pub async fn usage(&self, user_id: i64) -> Result<Usage, sqlx::Error> {
        let row = sqlx::query(&format!(
            "SELECT to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM') AS period, \
                    to_char({PERIOD_START} AT TIME ZONE 'UTC', {TS}) AS period_start, \
                    to_char({PERIOD_END} AT TIME ZONE 'UTC', {TS}) AS period_end, \
                    (SELECT count(*) FROM runs WHERE user_id = $1 AND NOT from_pack AND created_at >= {PERIOD_START}) AS used, \
                    {PACK_STRATEGIES_LEFT} AS pack_strategies, {PACK_RUNS_LEFT} AS pack_runs"
        ))
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(Usage {
            period: row.try_get("period")?,
            period_start: row.try_get("period_start")?,
            period_end: row.try_get("period_end")?,
            runs_used: row.try_get("used")?,
            pack_strategies_left: row.try_get("pack_strategies")?,
            pack_runs_left: row.try_get("pack_runs")?,
        })
    }

    /// Notes one strategy check (a preview or a save), unless the user has reached the hourly limit (then false).
    pub async fn record_preview(&self, user_id: i64, hourly_limit: i64) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(TAKE_LOCK).bind(LOCK_PREVIEWS).bind(user_id as i32).execute(&mut *tx).await?;
        let done = sqlx::query(
            "INSERT INTO preview_calls (user_id) SELECT $1 \
             WHERE (SELECT count(*) FROM preview_calls WHERE user_id = $1 AND created_at > now() - interval '1 hour') < $2",
        )
        .bind(user_id)
        .bind(hourly_limit)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(done.rows_affected() == 1)
    }

    /// Notes the start of one plain-English compile, unless a limit says no. A compile
    /// that is let through counts against the limits whatever comes of it.
    pub async fn begin_llm_call(&self, user_id: i64, world_id: &str, limits: CompileLimits) -> Result<CompileGate, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        // One lock for everyone: the daily ceiling is shared by all users.
        sqlx::query(TAKE_LOCK).bind(LOCK_COMPILE).bind(0i32).execute(&mut *tx).await?;
        let today = "created_at >= date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'";
        let row = sqlx::query(&format!(
            "SELECT (SELECT count(*) FROM llm_calls WHERE user_id = $1 AND created_at > now() - interval '1 hour') AS hour, \
                    (SELECT count(*) FROM llm_calls WHERE user_id = $1 AND blocked AND created_at > now() - interval '24 hours') AS blocked, \
                    (SELECT count(*) FROM llm_calls WHERE {today}) AS day, \
                    (SELECT coalesce(sum(input_tokens + output_tokens), 0)::bigint FROM llm_calls WHERE {today}) AS tokens, \
                    (SELECT count(*) FROM llm_calls WHERE user_id = $1 AND counted AND NOT from_pack AND world_id = $2) + {CARRIED_OVER} AS world, \
                    (SELECT count(*) FROM llm_calls WHERE user_id = $1 AND counted AND NOT from_pack AND created_at >= {PERIOD_START}) AS month, \
                    {PACK_STRATEGIES_LEFT} AS pack"
        ))
        .bind(user_id)
        .bind(world_id)
        .fetch_one(&mut *tx)
        .await?;
        let count = |name: &str| row.try_get::<i64, _>(name);
        if count("blocked")? >= limits.blocked_per_day {
            return Ok(CompileGate::Suspended);
        }
        // A request still under way is counted too, so requests arriving together cannot share one place.
        let used_up = match limits.allowance {
            Allowance::PerWorld(n) => count("world")? >= n,
            Allowance::PerMonth(n) => count("month")? >= n,
        };
        // When the plan's own are used up, a pack pays, if the user has one with anything left.
        let from_pack = used_up && count("pack")? > 0;
        if used_up && !from_pack {
            return Ok(CompileGate::AllowanceUsed);
        }
        if count("hour")? >= limits.per_hour {
            return Ok(CompileGate::HourlyLimit);
        }
        if count("day")? >= limits.per_day || count("tokens")? >= limits.tokens_per_day {
            return Ok(CompileGate::DailyCeiling);
        }
        let id: i64 = sqlx::query("INSERT INTO llm_calls (user_id, world_id, from_pack) VALUES ($1, $2, $3) RETURNING id").bind(user_id).bind(world_id).bind(from_pack).fetch_one(&mut *tx).await?.try_get("id")?;
        tx.commit().await?;
        Ok(CompileGate::Started(id))
    }

    /// Completes the record of a compile: what the screen said and what the calls used.
    /// `counted` is false when the language model was never reached, which gives the AI request back.
    pub async fn finish_llm_call(&self, id: i64, spent: &Spent, counted: bool) -> Result<(), sqlx::Error> {
        let verdict = spent.verdict.as_ref();
        sqlx::query("UPDATE llm_calls SET verdict = $2, confidence = $3, blocked = $4, input_tokens = $5, output_tokens = $6, counted = $7 WHERE id = $1")
            .bind(id)
            .bind(verdict.map(|v| v.label.as_str()))
            .bind(verdict.map(|v| v.confidence))
            .bind(verdict.is_some_and(|v| !v.allowed))
            .bind(spent.input_tokens)
            .bind(spent.output_tokens)
            .bind(counted)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Strategies the AI has written for the user on the plan's own allowance (those a pack paid for are
    /// left out): in each world for good, and in the current period across all worlds.
    pub async fn ai_requests_used(&self, user_id: i64) -> Result<(std::collections::BTreeMap<String, i64>, i64), sqlx::Error> {
        let rows = sqlx::query(&format!(
            "SELECT world_id, count(*) AS used, count(*) FILTER (WHERE created_at >= {PERIOD_START}) AS month FROM llm_calls WHERE user_id = $1 AND counted AND NOT from_pack GROUP BY world_id"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        let mut by_world = std::collections::BTreeMap::new();
        let mut month = 0;
        for row in rows {
            month += row.try_get::<i64, _>("month")?;
            if let Some(world) = row.try_get::<Option<String>, _>("world_id")? {
                by_world.insert(world, row.try_get::<i64, _>("used")?);
            }
        }
        // What an earlier, deleted account under the same address had used.
        let carried = sqlx::query("SELECT world_id, used FROM spent_free_strategies WHERE email_hash = (SELECT encode(sha256(convert_to(email, 'UTF8')), 'hex') FROM users WHERE id = $1)").bind(user_id).fetch_all(&self.pool).await?;
        for row in carried {
            *by_world.entry(row.try_get("world_id")?).or_insert(0) += row.try_get::<i64, _>("used")?;
        }
        Ok((by_world, month))
    }

    // --- payment ---

    /// The user's customer record at Stripe, if they have one.
    pub async fn customer(&self, user_id: i64) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT stripe_customer_id FROM users WHERE id = $1").bind(user_id).fetch_optional(&self.pool).await.map(Option::flatten)
    }

    /// Notes the customer record made for a user, unless they have one already; returns the one that stands.
    pub async fn keep_customer(&self, user_id: i64, customer: &str) -> Result<String, sqlx::Error> {
        sqlx::query_scalar("UPDATE users SET stripe_customer_id = coalesce(stripe_customer_id, $2) WHERE id = $1 RETURNING stripe_customer_id").bind(user_id).bind(customer).fetch_one(&self.pool).await
    }

    /// Whose customer record this is.
    pub async fn customer_user(&self, customer: &str) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar("SELECT id FROM users WHERE stripe_customer_id = $1").bind(customer).fetch_optional(&self.pool).await
    }

    /// The user agreed to the terms, and to the service starting at once, on their way to pay.
    pub async fn accept_terms(&self, user_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET terms_accepted_at = now() WHERE id = $1").bind(user_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Stores a subscription as Stripe has it. The paid period moves only when `sub.paid`: a renewal
    /// Stripe has not collected yet opens nothing. A user has one subscription; another one takes its
    /// place only when the one we know has ended, so a late message about an old one changes nothing.
    pub async fn save_subscription(&self, user_id: i64, sub: &crate::billing::Subscription) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO subscriptions AS s (user_id, stripe_subscription_id, status, period_start, period_end, cancel_at_period_end) \
             VALUES ($1, $2, $3, CASE WHEN $7 THEN to_timestamp($4) END, CASE WHEN $7 THEN to_timestamp($5) END, $6) \
             ON CONFLICT (user_id) DO UPDATE SET \
                 stripe_subscription_id = EXCLUDED.stripe_subscription_id, status = EXCLUDED.status, cancel_at_period_end = EXCLUDED.cancel_at_period_end, updated_at = now(), \
                 period_start = CASE WHEN $7 THEN EXCLUDED.period_start WHEN s.stripe_subscription_id = EXCLUDED.stripe_subscription_id THEN s.period_start END, \
                 period_end = CASE WHEN $7 THEN EXCLUDED.period_end WHEN s.stripe_subscription_id = EXCLUDED.stripe_subscription_id THEN s.period_end END \
             WHERE s.stripe_subscription_id = EXCLUDED.stripe_subscription_id OR s.status NOT IN ('active', 'past_due', 'trialing', 'incomplete')",
        )
        .bind(user_id)
        .bind(&sub.id)
        .bind(&sub.status)
        .bind(sub.period_start as f64)
        .bind(sub.period_end as f64)
        .bind(sub.cancel_at_period_end)
        .bind(sub.paid)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn subscription(&self, user_id: i64) -> Result<Option<SubscriptionRow>, sqlx::Error> {
        let sql = format!(
            "SELECT stripe_subscription_id, status, cancel_at_period_end, \
                    to_char(period_start AT TIME ZONE 'UTC', {TS}) AS period_start, to_char(period_end AT TIME ZONE 'UTC', {TS}) AS period_end, \
                    (status IN ('active', 'past_due') AND period_end + interval '1 day' > now()) IS TRUE AS paid \
             FROM subscriptions WHERE user_id = $1"
        );
        sqlx::query(&sql).bind(user_id).fetch_optional(&self.pool).await?.map(subscription_row).transpose()
    }

    /// Notes a pack that was paid for. False if this payment was noted before (nothing is added twice).
    pub async fn record_purchase(&self, user_id: i64, pack: &crate::billing::Pack, session: &crate::billing::Session) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "INSERT INTO purchases (user_id, pack, strategies, runs, amount, currency, stripe_session_id, stripe_payment_intent) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (stripe_session_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(pack.id)
        .bind(pack.strategies)
        .bind(pack.runs)
        .bind(session.amount)
        .bind(&session.currency)
        .bind(&session.id)
        .bind(&session.payment_intent)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Money was paid back for a pack (or the payment was disputed): `share` of the price, from 0 to 1, in all.
    /// The same share of the pack is taken back, rounded up. Told twice, it is taken once; a later, larger
    /// refund takes the difference. False if we know of no such payment.
    pub async fn refund_purchase(&self, payment_intent: &str, share: f64) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE purchases SET refunded_at = coalesce(refunded_at, now()), \
                 refunded_strategies = greatest(refunded_strategies, least(strategies, ceil(strategies * $2)::bigint)), \
                 refunded_runs = greatest(refunded_runs, least(runs, ceil(runs * $2)::bigint)) \
             WHERE stripe_payment_intent = $1",
        )
        .bind(payment_intent)
        .bind(share.clamp(0.0, 1.0))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// The packs a user has bought, newest first.
    pub async fn purchases(&self, user_id: i64) -> Result<Vec<PurchaseRow>, sqlx::Error> {
        let sql = format!("SELECT pack, strategies, runs, amount, currency, to_char(created_at AT TIME ZONE 'UTC', {TS}) AS created_at, (refunded_strategies >= strategies AND refunded_runs >= runs AND refunded_at IS NOT NULL) AS refunded FROM purchases WHERE user_id = $1 ORDER BY id DESC");
        let rows = sqlx::query(&sql).bind(user_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| {
                Ok(PurchaseRow {
                    pack: r.try_get("pack")?,
                    strategies: r.try_get("strategies")?,
                    runs: r.try_get("runs")?,
                    amount: r.try_get("amount")?,
                    currency: r.try_get("currency")?,
                    created_at: r.try_get("created_at")?,
                    refunded: r.try_get("refunded")?,
                })
            })
            .collect()
    }

    /// Whether this message from Stripe has been acted on before.
    pub async fn event_seen(&self, id: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stripe_events WHERE id = $1)").bind(id).fetch_one(&self.pool).await
    }

    pub async fn record_event(&self, id: &str, kind: &str) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO stripe_events (id, kind) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(id).bind(kind).execute(&self.pool).await?;
        Ok(())
    }

    /// Moves a user's paid period (and when it was read) into the past, so a test can stand after its end.
    #[cfg(test)]
    pub async fn age_subscription(&self, user_id: i64, interval: &str) {
        sqlx::query(&format!("UPDATE subscriptions SET period_start = period_start - interval '{interval}', period_end = period_end - interval '{interval}' WHERE user_id = $1")).bind(user_id).execute(&self.pool).await.unwrap();
    }

    // --- leaving ---

    /// Everything we hold about a user, for them to take away.
    pub async fn export_user(&self, user_id: i64) -> Result<Value, sqlx::Error> {
        let one = |sql: &'static str| async move { sqlx::query_scalar::<_, Option<Value>>(sql).bind(user_id).fetch_one(&self.pool).await.map(|v| v.unwrap_or(Value::Null)) };
        Ok(json!({
            "account": one("SELECT (SELECT to_jsonb(u) FROM (SELECT id, email, display_name, roles, plan, created_at, terms_accepted_at, stripe_customer_id FROM users WHERE id = $1) u)").await?,
            "sign_in_providers": one("SELECT coalesce(jsonb_agg(to_jsonb(i)), '[]') FROM (SELECT provider, created_at FROM user_identities WHERE user_id = $1 ORDER BY created_at) i").await?,
            "strategies": one("SELECT coalesce(jsonb_agg(to_jsonb(s)), '[]') FROM (SELECT id, visibility, mode, world_id, prompt, doc, created_at, updated_at FROM strategies WHERE owner_id = $1 ORDER BY created_at) s").await?,
            "runs": one("SELECT coalesce(jsonb_agg(to_jsonb(r)), '[]') FROM (SELECT id, world_id, status, seed, iterations, actor_count, private, from_pack, created_at, finished_at, result FROM runs WHERE user_id = $1 ORDER BY id) r").await?,
            "ai_written_strategies": one("SELECT coalesce(jsonb_agg(to_jsonb(c)), '[]') FROM (SELECT created_at, world_id, counted, from_pack, verdict FROM llm_calls WHERE user_id = $1 ORDER BY id) c").await?,
            "subscription": one("SELECT (SELECT to_jsonb(s) FROM (SELECT status, period_start, period_end, cancel_at_period_end FROM subscriptions WHERE user_id = $1) s)").await?,
            "purchases": one("SELECT coalesce(jsonb_agg(to_jsonb(p)), '[]') FROM (SELECT pack, strategies, runs, amount, currency, created_at, refunded_strategies, refunded_runs, refunded_at FROM purchases WHERE user_id = $1 ORDER BY id) p").await?,
        }))
    }

    /// Deletes a user and what is theirs: the account, sessions, strategies, private runs, what they
    /// bought. Public runs stay, as part of their world's history, with nobody's name on them. What
    /// they had used of the free AI-written strategies is kept under a hash of the address.
    pub async fn delete_user(&self, user_id: i64) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO spent_free_strategies (email_hash, world_id, used) \
             SELECT encode(sha256(convert_to(u.email, 'UTF8')), 'hex'), c.world_id, count(*) FROM llm_calls c JOIN users u ON u.id = c.user_id \
             WHERE c.user_id = $1 AND c.counted AND NOT c.from_pack AND c.world_id IS NOT NULL GROUP BY u.email, c.world_id \
             ON CONFLICT (email_hash, world_id) DO UPDATE SET used = spent_free_strategies.used + EXCLUDED.used",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM sign_in_tokens WHERE email = (SELECT email FROM users WHERE id = $1)").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM runs WHERE user_id = $1 AND (private OR status IN ('queued', 'running'))").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE runs SET user_id = NULL WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&mut *tx).await?;
        tx.commit().await
    }

    // --- runs ---

    /// Adds a run to the queue, unless the user has used up the month's runs
    /// (`monthly_limit`) or already has `active_limit` runs queued or running.
    pub async fn enqueue(&self, input: &RunInput, user_id: i64, private: bool, monthly_limit: i64, active_limit: i64) -> Result<Queued, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(TAKE_LOCK).bind(LOCK_RUNS).bind(user_id as i32).execute(&mut *tx).await?;
        let row = sqlx::query(&format!(
            "SELECT (SELECT count(*) FROM runs WHERE user_id = $1 AND NOT from_pack AND created_at >= {PERIOD_START}) AS month, \
                    (SELECT count(*) FROM runs WHERE user_id = $1 AND status IN ('queued', 'running')) AS active, \
                    {PACK_RUNS_LEFT} AS pack"
        ))
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        // When the plan's own runs are used up, a pack pays, if the user has one with anything left.
        let used_up = row.try_get::<i64, _>("month")? >= monthly_limit;
        let from_pack = used_up && row.try_get::<i64, _>("pack")? > 0;
        if used_up && !from_pack {
            return Ok(Queued::MonthlyLimit);
        }
        if row.try_get::<i64, _>("active")? >= active_limit {
            return Ok(Queued::TooManyActive);
        }
        let sql = format!(
            "INSERT INTO runs (world_id, world_name, seed, iterations, actor_count, input, user_id, private, from_pack) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING {}",
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
            .bind(from_pack)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Queued::Run(run_row(row)?))
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

    /// Stores a finished run: summary, traces and KPI snapshots, all or nothing.
    pub async fn complete(&self, id: i64, out: &RunOutput) -> Result<(), sqlx::Error> {
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
        tx.commit().await
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

    /// A finished run as the engine gave it: the summary and every pairing's rounds.
    pub async fn run_output(&self, run_id: i64, result: &RunResult) -> Result<RunOutput, sqlx::Error> {
        let traces = self.traces(run_id).await?.into_iter().map(|(a, b, moves)| MatchTrace { a, b, moves }).collect();
        Ok(RunOutput { result: result.clone(), traces })
    }

    /// Every pairing of a run with its rounds: `(a, b, moves)`, one `[a's move, b's move]` per round.
    pub async fn traces(&self, run_id: i64) -> Result<Vec<(String, String, Vec<[u8; 2]>)>, sqlx::Error> {
        let rows = sqlx::query("SELECT a, b, moves FROM run_traces WHERE run_id = $1 ORDER BY a, b").bind(run_id).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| {
                let bytes: Vec<u8> = r.try_get("moves")?;
                Ok((r.try_get("a")?, r.try_get("b")?, bytes.chunks_exact(2).map(|c| [c[0], c[1]]).collect()))
            })
            .collect()
    }

    /// The world a run was played in, as frozen when the run was queued.
    pub async fn run_world(&self, run_id: i64) -> Result<Option<World>, sqlx::Error> {
        let row = sqlx::query("SELECT input FROM runs WHERE id = $1").bind(run_id).fetch_optional(&self.pool).await?;
        row.map(|r| Ok(r.try_get::<Json<RunInput>, _>("input")?.0.world)).transpose()
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
