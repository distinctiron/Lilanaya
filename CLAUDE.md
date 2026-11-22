# Lilanaya

Strategy-simulation platform: users define game-theory "worlds" (first: repeated Prisoner's Dilemma), author strategies, and run seeded, reproducible simulations. Product scope lives in `spec/docs/` (PRD, architecture, stories).

**What to build next: `spec/docs/implementation-plan.md`.** When asked to "implement", take the first unchecked item there, work down the list, and tick items off as they are verified.

## Stack (decided — overrides anything in `spec/docs` that says otherwise)

- Back end: Rust, axum on tokio. One Cargo workspace at the repo root; crates under `src/`.
- Front end: vanilla JavaScript, native ES modules, no framework, no bundler, no Node. Files in `src/app/web/lilanaya-client/public/` are served as-is by nginx.
- Everything runs in containers. There is no Rust toolchain on the host: run cargo through Docker.

## Commands

- Dev: `docker compose up --build` — web http://localhost:3000, API http://localhost:8080/api/health, caught emails (sign-in links) http://localhost:8025
- Production-like: `docker compose -f compose.prod.yaml up --build` — http://localhost:3000
- Rust tests: `docker compose run --rm api cargo test` (starts Postgres and the S3 store; each API test uses its own schema and key prefix)
- Golden files: if the engine's output changes on purpose, regenerate with `docker compose run --rm -e UPDATE_GOLDEN=1 api cargo test -p lilanaya-engine --test golden` and review the diff

## Layout

- `src/engine` (`lilanaya-engine`): pure simulation engine, no I/O. World model, FSM strategies, schedules, KPIs.
- `src/app/api` (`lilanaya-api`): axum routes, Postgres store (sqlx, runtime queries, migrations in `migrations/`), background run worker (the `runs` table is the job queue), run artifacts in S3-compatible storage (`artifacts.rs`, `object_store` crate; configured by `S3_*` in the compose files).
  - Accounts: `auth.rs` (session cookie, `User` / `Viewer` extractors), `mail.rs`, `routes/account.rs`.
  - Strategies: `strategies.rs` (rules or code documents, preview, plain-English compile), `sandbox.rs` (JavaScript in QuickJS), `llm.rs` (raw HTTP; Claude through the Claude API for development, an Azure OpenAI deployment through the Responses API for production), `routes/authoring.rs`.
- `src/app/web/lilanaya-client/public`: `pages/` (one module per screen; `landing.js` is the home page at `/`, where a visitor plays five rounds by hand before anything is explained; `how-it-works.js` holds the narrated films), `components/` (`avatar.js` draws a strategy's avatar from its id, `film.js` plays the narrated step-by-step films, `scene.js` is their pausable clock, `pocket-dilemma.js` the small Prisoner's Dilemma both pages play in the browser), `router.js`, `api.js`, `session.js` (who is signed in), `dom.js`, `style.css`.

## Conventions

- Look and feel: the "Arcade" direction (black and white, pixel display type, dashed dividers, solid versus hollow blocks). `spec/docs/design/visual-direction.md` is the source of truth; follow it for every screen.
- All API routes are under `/api`; nginx proxies `/api` to the backend, so the front end uses relative URLs.
- API errors always have the shape `{"error": {"code", "message", "details"?}}`. `POST /api/runs` answers 202 with a queued run; clients poll `GET /api/runs/{id}`.
- Anything a user creates has an owner. A private strategy or run answers 404 (not 403) to everyone else and is left out of lists and world analytics. Handlers that write take `User` (401 when signed out); handlers that read take `Viewer`.
- Settings read from the environment treat an empty value as unset (compose passes unset variables as empty strings). Secrets go in `.env`, never in the compose files.
- Code strategies must stay deterministic: the sandbox has no clock or randomness, and its limits are counted in interpreter steps, not time.
- Determinism is non-negotiable: every run is reproducible from its recorded seed. No wall-clock time, unseeded randomness, or hash-map iteration order in the engine.
- The name is "Lilanaya" (not "Lilanyaya").
