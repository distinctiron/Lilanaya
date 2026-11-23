# Lilanaya

Strategy-simulation platform: users define game-theory "worlds" (first: repeated Prisoner's Dilemma), author strategies, and run seeded, reproducible simulations. Product scope lives in `spec/docs/` (PRD, architecture, stories).

**What to build next: `spec/docs/implementation-plan.md`.** When asked to "implement", take the first unchecked item there, work down the list, and tick items off as they are verified.

## Stack (decided — overrides anything in `spec/docs` that says otherwise)

- Back end: Rust, axum on tokio. One Cargo workspace at the repo root; crates under `src/`.
- Front end: vanilla JavaScript, native ES modules, no framework, no bundler, no Node. Files in `src/app/web/lilanaya-client/public/` are served as-is by nginx.
- Everything runs in containers. There is no Rust toolchain on the host: run cargo through Docker.

## Commands

- Dev: `docker compose up --build` — web http://localhost:3000, API http://localhost:8080/api/health, caught emails (sign-up and new-password links) http://localhost:8025
- Production-like: `docker compose -f compose.prod.yaml up --build` — http://localhost:3000 (needs `POSTGRES_PASSWORD`)
- Production: Render, described by `render.yaml`. The API and the worker are the same image with `ROLE=api` and `ROLE=worker`; `APP_ENV=production` makes it refuse to start without https and a mail server.
- Tests: `./test` runs everything (lint, unit, API, safety, UI) in Docker; `./test unit`, `./test api`, `./test safety`, `./test ui` run one kind. What is tested where, and how to add a test: `tests/README.md`.
- Golden files: if the engine's output changes on purpose, regenerate with `docker compose run --rm -e UPDATE_GOLDEN=1 api cargo test -p lilanaya-engine --test golden` and review the diff

## Layout

- `src/engine` (`lilanaya-engine`): pure simulation engine, no I/O. World model, FSM strategies, schedules, KPIs.
- `src/app/api` (`lilanaya-api`): axum routes, Postgres store (sqlx, runtime queries, migrations in `migrations/`), background run worker (the `runs` table is the job queue), a run's downloadable files built on request from the database (`artifacts.rs`; nothing is stored).
  - Accounts: `auth.rs` (session cookie, password hashing with Argon2id, `User` / `Viewer` extractors), `mail.rs`, `routes/account.rs`. The emailed link is for signing up and for choosing a new password (`POST /api/auth/email/confirm` takes the token and the password, and ends the user's other sessions); logging in is address and password (`POST /api/auth/log-in`, ten wrong passwords per address in 15 minutes).
  - Sign-in through Google, Apple, Facebook, GitHub: `oauth.rs` (the providers; each is behind its own flag, `GOOGLE_SIGN_IN`, `APPLE_SIGN_IN`, `FACEBOOK_SIGN_IN`, `GITHUB_SIGN_IN`, off unless set to `on`, and one that is on must have its credentials in the environment, see `.env.example`), `routes/federated.rs` (browser redirects under `/api/auth/{provider}/…`; failures land on `/sign-in?error=<code>`). An outside account is tied to a user in `user_identities`; a new one joins the user with the same verified address.
  - Strategies: `strategies.rs` (rules or code documents, preview, plain-English compile), `sandbox.rs` (JavaScript in QuickJS), `isolate.rs` (runs previews and runs in a child process with a time limit), `llm.rs` (raw HTTP to Vercel AI Gateway, Chat Completions), `guard.rs` (Jev, the screen in front of every model call, through the gateway's Decisions API), `routes/authoring.rs`.
  - Payment: `billing.rs` (Stripe over raw HTTP: Checkout, customer portal, webhook signature, the pack catalog; off unless `STRIPE_SECRET_KEY` is set), `routes/billing.rs`. Amounts live in Stripe, not in code. A user is a subscriber while `subscriptions` holds a paid period that has not run out (`PAID_SUBSCRIPTION` in `store.rs`); the period moves only when Stripe says its invoice is paid. Packs (`purchases`) are spent after the plan's own allowance (`from_pack` on `llm_calls` and `runs`). Whatever a webhook says is read again from Stripe before it is believed.
  - Limits: `config.rs` (the numbers, from the environment), `store.rs` (each limit is counted and written under an advisory lock, so requests arriving together cannot all pass).
- `tests/`: `web/` (unit tests of front-end modules, Node's own test runner), `ui/` (Playwright; `compose.test.yaml` starts the whole site for it, with a stand-in AI gateway in `ui/fake-ai/`). Rust tests live with the code: `src/engine/tests/`, `mod tests` in each module, and `src/app/api/src/routes/tests/` (one file per area, `access.rs`, `contract.rs`, `safety/`). The front end itself still has no Node and no build step: Node is used only to run tests, in containers.
- `src/app/web/lilanaya-client/public`: `pages/` (one module per screen; `landing.js` is the home page at `/`, where a visitor plays five rounds by hand before anything is explained; `how-it-works.js` holds the narrated films), `components/` (`avatar.js` draws a strategy's avatar from its id, `film.js` plays the narrated step-by-step films, `scene.js` is their pausable clock, `pocket-dilemma.js` the small Prisoner's Dilemma both pages play in the browser), `router.js`, `api.js`, `session.js` (who is signed in), `dom.js`, `style.css`.

## Conventions

- Every feature and every fix comes with tests, at the lowest layer that can show it works (`tests/README.md`, "Adding tests"). A new API route needs a row in `src/app/api/src/routes/tests/access.rs` saying who may call it; anything a user must not be able to do needs a test under `routes/tests/safety/`. Work is done, and an item in the implementation plan is ticked, only when `./test` passes.

- Look and feel: the "Arcade" direction (black and white, pixel display type, dashed dividers, solid versus hollow blocks). `spec/docs/design/visual-direction.md` is the source of truth; follow it for every screen.
- All API routes are under `/api`; nginx proxies `/api` to the backend, so the front end uses relative URLs.
- API errors always have the shape `{"error": {"code", "message", "details"?}}`. `POST /api/runs` answers 202 with a queued run; clients poll `GET /api/runs/{id}`.
- Anything a user creates has an owner. A private strategy or run answers 404 (not 403) to everyone else and is left out of lists and world analytics. Handlers that write take `User` (401 when signed out); handlers that read take `Viewer`.
- Settings read from the environment treat an empty value as unset (compose passes unset variables as empty strings). Secrets go in `.env`, never in the compose files.
- Code strategies must stay deterministic: the sandbox has no clock or randomness, and its limits are counted in interpreter steps, not time. The one clock is outside it: `isolate.rs` stops a child process that runs too long. That fails the job ("stopped by the server"); it never decides a match.
- Nothing a user wrote is executed in the API process: previews, saves and runs go through `isolate.rs`.
- No text reaches the language model without passing the screen: `Llm::complete` is the only way to the model and screens on every call. If the screen cannot be reached, the call is refused.
- Determinism is non-negotiable: every run is reproducible from its recorded seed. No wall-clock time, unseeded randomness, or hash-map iteration order in the engine.
- The name is "Lilanaya" (not "Lilanyaya").
- On the site, say "strategies" (the AI writes N strategies for you), never "AI requests". The settings and JSON fields keep the older name (`ai_requests`).
- The terms, privacy and refund pages (`pages/legal.js`) describe what the code does. A change to what is counted, kept or refunded changes them too.
