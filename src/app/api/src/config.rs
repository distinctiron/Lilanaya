//! Settings read from the environment once at startup.

#[derive(Clone, Debug)]
pub struct Config {
    /// Where browsers reach the site; sign-in links point here.
    pub public_url: String,
    /// Runs a user may start per calendar month, by plan.
    pub free_monthly_runs: i64,
    pub subscriber_monthly_runs: i64,
    /// AI requests (the language model writing rules from a description) a free user has per world. They never come back.
    pub free_ai_requests_per_world: i64,
    /// AI requests a subscriber has per calendar month, across all worlds.
    pub subscriber_ai_requests_per_month: i64,
    /// Runs one user may have queued or running at the same time.
    pub max_active_runs: i64,
    /// Strategy checks (previews and saves) a user may ask for per hour.
    pub previews_per_hour: i64,
    /// Plain-English compiles a user may ask for per hour.
    pub compiles_per_hour: i64,
    /// Plain-English compiles all users together may ask for per day (UTC). Reaching it switches compile off until the next day.
    pub compiles_per_day: i64,
    /// Tokens all compiles together may use per day (UTC), screening included. Same effect.
    pub compile_tokens_per_day: i64,
}

/// Descriptions the screen may turn down in 24 hours before compile is closed to that user for the rest of them.
pub const BLOCKED_DESCRIPTIONS_PER_DAY: i64 = 5;

impl Config {
    pub fn from_env() -> Result<Self, String> {
        // Compose passes unset settings as empty strings; treat those as unset.
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let number = |name: &str, default: i64| match var(name) {
            None => Ok(default),
            Some(raw) => raw.parse::<i64>().ok().filter(|n| *n >= 0).ok_or_else(|| format!("{name} must be a whole number, got \"{raw}\"")),
        };
        Ok(Config {
            public_url: var("PUBLIC_URL").unwrap_or_else(|| "http://localhost:3000".into()).trim_end_matches('/').to_string(),
            free_monthly_runs: number("FREE_MONTHLY_RUNS", 100)?,
            subscriber_monthly_runs: number("SUBSCRIBER_MONTHLY_RUNS", 1000)?,
            free_ai_requests_per_world: number("FREE_AI_REQUESTS_PER_WORLD", 3)?,
            subscriber_ai_requests_per_month: number("SUBSCRIBER_AI_REQUESTS_PER_MONTH", 20)?,
            max_active_runs: number("MAX_ACTIVE_RUNS", 2)?,
            previews_per_hour: number("PREVIEWS_PER_HOUR", 120)?,
            compiles_per_hour: number("COMPILES_PER_HOUR", 20)?,
            compiles_per_day: number("COMPILES_PER_DAY", 500)?,
            compile_tokens_per_day: number("COMPILE_TOKENS_PER_DAY", 2_000_000)?,
        })
    }

    /// Session cookies are marked `Secure` when the site is served over HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.public_url.starts_with("https://")
    }

    pub fn monthly_runs(&self, plan: &str) -> i64 {
        if plan == "subscriber" {
            self.subscriber_monthly_runs
        } else {
            self.free_monthly_runs
        }
    }

    /// The plan's allowance of AI requests.
    pub fn ai_allowance(&self, plan: &str) -> Allowance {
        if plan == "subscriber" {
            Allowance::PerMonth(self.subscriber_ai_requests_per_month)
        } else {
            Allowance::PerWorld(self.free_ai_requests_per_world)
        }
    }
}

/// How many AI requests a plan includes, and what the number is counted over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Allowance {
    /// This many in each world, for good.
    PerWorld(i64),
    /// This many per calendar month (UTC), across all worlds.
    PerMonth(i64),
}

#[cfg(test)]
impl Config {
    pub fn for_tests() -> Self {
        Config {
            public_url: "http://localhost:3000".into(),
            free_monthly_runs: 100,
            subscriber_monthly_runs: 1000,
            free_ai_requests_per_world: 3,
            subscriber_ai_requests_per_month: 20,
            max_active_runs: 2,
            previews_per_hour: 120,
            compiles_per_hour: 3,
            compiles_per_day: 500,
            compile_tokens_per_day: 2_000_000,
        }
    }
}
