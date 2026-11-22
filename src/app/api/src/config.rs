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
        Self::read(|name| std::env::var(name).ok())
    }

    /// Reads the settings from `lookup` (the environment, or a stand-in for it in tests).
    fn read(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        // Compose passes unset settings as empty strings; treat those as unset.
        let var = |name: &str| lookup(name).filter(|v| !v.trim().is_empty());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn read(settings: &[(&str, &str)]) -> Result<Config, String> {
        Config::read(|name| settings.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn with_nothing_set_the_limits_are_the_agreed_defaults() {
        let config = read(&[]).unwrap();
        assert_eq!(config.public_url, "http://localhost:3000");
        assert_eq!((config.free_monthly_runs, config.subscriber_monthly_runs), (100, 1000));
        assert_eq!((config.free_ai_requests_per_world, config.subscriber_ai_requests_per_month), (3, 20));
        assert_eq!((config.max_active_runs, config.previews_per_hour, config.compiles_per_hour), (2, 120, 20));
        assert_eq!((config.compiles_per_day, config.compile_tokens_per_day), (500, 2_000_000));
        assert_eq!(BLOCKED_DESCRIPTIONS_PER_DAY, 5);
    }

    #[test]
    fn an_empty_setting_counts_as_not_set() {
        // Compose passes every setting it knows of, empty when it has no value.
        let names = ["PUBLIC_URL", "FREE_MONTHLY_RUNS", "SUBSCRIBER_MONTHLY_RUNS", "FREE_AI_REQUESTS_PER_WORLD", "SUBSCRIBER_AI_REQUESTS_PER_MONTH", "MAX_ACTIVE_RUNS", "PREVIEWS_PER_HOUR", "COMPILES_PER_HOUR", "COMPILES_PER_DAY", "COMPILE_TOKENS_PER_DAY"];
        for blank in ["", " ", "\t"] {
            let settings: Vec<(&str, &str)> = names.iter().map(|name| (*name, blank)).collect();
            assert_eq!(format!("{:?}", read(&settings).unwrap()), format!("{:?}", read(&[]).unwrap()), "{blank:?}");
        }
    }

    #[test]
    fn every_limit_can_be_set_and_goes_where_its_name_says() {
        let config = read(&[
            ("PUBLIC_URL", "https://www.lilanaya.example/"),
            ("FREE_MONTHLY_RUNS", "1"),
            ("SUBSCRIBER_MONTHLY_RUNS", "2"),
            ("FREE_AI_REQUESTS_PER_WORLD", "3"),
            ("SUBSCRIBER_AI_REQUESTS_PER_MONTH", "4"),
            ("MAX_ACTIVE_RUNS", "5"),
            ("PREVIEWS_PER_HOUR", "6"),
            ("COMPILES_PER_HOUR", "7"),
            ("COMPILES_PER_DAY", "8"),
            ("COMPILE_TOKENS_PER_DAY", "9"),
        ])
        .unwrap();
        // Sign-in links are built from this address: no doubled slash.
        assert_eq!(config.public_url, "https://www.lilanaya.example");
        assert_eq!((config.free_monthly_runs, config.subscriber_monthly_runs, config.free_ai_requests_per_world, config.subscriber_ai_requests_per_month), (1, 2, 3, 4));
        assert_eq!((config.max_active_runs, config.previews_per_hour, config.compiles_per_hour, config.compiles_per_day, config.compile_tokens_per_day), (5, 6, 7, 8, 9));
        // Nought is a number too: it switches the thing off.
        assert_eq!(read(&[("FREE_MONTHLY_RUNS", "0")]).unwrap().free_monthly_runs, 0);
    }

    #[test]
    fn a_limit_that_is_not_a_whole_number_stops_the_program_and_names_the_setting() {
        for (name, value) in [("FREE_MONTHLY_RUNS", "many"), ("MAX_ACTIVE_RUNS", "-1"), ("PREVIEWS_PER_HOUR", "1.5"), ("COMPILES_PER_DAY", "1e3"), ("COMPILE_TOKENS_PER_DAY", "2,000,000"), ("SUBSCRIBER_AI_REQUESTS_PER_MONTH", "20 a month")] {
            let error = read(&[(name, value)]).unwrap_err();
            assert_eq!(error, format!("{name} must be a whole number, got \"{value}\""));
        }
    }

    #[test]
    fn the_plan_decides_the_allowance_and_anything_but_subscriber_is_free() {
        let config = read(&[]).unwrap();
        assert_eq!((config.monthly_runs("subscriber"), config.ai_allowance("subscriber")), (1000, Allowance::PerMonth(20)));
        for plan in ["free", "", "Subscriber", "admin", "subscriber "] {
            assert_eq!((config.monthly_runs(plan), config.ai_allowance(plan)), (100, Allowance::PerWorld(3)), "{plan:?}");
        }
    }

    #[test]
    fn cookies_are_marked_secure_exactly_when_the_site_is_on_https() {
        let at = |url: &str| read(&[("PUBLIC_URL", url)]).unwrap().secure_cookies();
        assert!(at("https://www.lilanaya.com"));
        assert!(!at("http://localhost:3000"));
        assert!(!at("http://https.example"));
    }

    #[test]
    fn the_settings_the_tests_use_are_the_defaults_except_where_they_say() {
        let (tests, defaults) = (Config::for_tests(), read(&[]).unwrap());
        // Three compiles an hour in tests, so the limit is quick to reach.
        assert_eq!(format!("{:?}", Config { compiles_per_hour: defaults.compiles_per_hour, ..tests }), format!("{defaults:?}"));
    }
}
