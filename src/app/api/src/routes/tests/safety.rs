//! Safety tests: what must hold however the API is used, and by whom.
//! Run on their own with `./test safety`.
//!
//! - `sandbox`: code a user wrote reaches nothing but its input, and is stopped when it must be.
//! - `screen`: nothing a user wrote reaches the language model unscreened.
//! - `limits`: every limit holds, also against requests arriving together and across its time window.
//! - `privacy`: what is private stays invisible, and nothing personal is handed out.
//! - `sessions`: cookies, sign-in links and the round trip through an outside provider.
//! - `requests`: hostile, malformed and oversized requests get a plain refusal.
//! - `payments`: nothing is given that was not paid for, and nobody but Stripe can say what was paid.

use super::*;

mod limits;
mod payments;
mod privacy;
mod requests;
mod sandbox;
mod screen;
mod sessions;
