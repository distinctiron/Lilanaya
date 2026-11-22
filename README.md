# Lilanaya

Lilanaya is a technical project for exploring strategies across configurable worlds via deterministic (seeded) simulations with explainable outcomes.

The codebase defines:
- A world schema (rules, decision/outcome spaces, interaction policy, iterations, payoff/scoring, state/KPIs).
- Strategy authoring models:
  - Natural language that compiles to a safe Finite State Machine (FSM).
  - Optional code strategies executed via a constrained interface in a sandboxed runtime.
- A deterministic simulation engine (seeded scheduling, reproducible runs) that produces artifacts (summaries, pairwise matrices, optional traces).
- Basic analytics surfaces (leaderboards, KPIs, pairwise drilldowns) derived from simulation artifacts.

Initial target world: Prisoner’s Dilemma (repeated). The system is world‑agnostic and supports additional game types and parameterized variants.

## Core capabilities

- Define and validate worlds with seedable baseline actors (e.g., Always Cooperate/Defect, Tit‑For‑Tat, Win‑Stay/Lose‑Shift).
- Compile natural‑language strategies to FSMs; optionally run code strategies with strict interfaces and limits.
- Run deterministic simulations with configurable scheduling (all_pairs, round_robin, random via seeds).
- Generate artifacts: summaries, pairwise payoff/outcome matrices, optional per‑round traces.
- Surface leaderboards and analytics (KPI snapshots, pairwise metrics).

## Non‑functional characteristics

- Deterministic and reproducible results from recorded seeds.
- Sandboxed execution for code strategies with enforced time/memory limits and allowed decision types.
- Supports batching of simulations and caching of hot aggregates.

## Tech stack

- Back end: Rust (axum on tokio), one Cargo workspace rooted at the repository root.
- Front end: vanilla JavaScript (native ES modules, no framework or build step), served by nginx.
- Storage: Postgres only (worlds, strategies, runs, traces, KPI snapshots; the `runs` table doubles as the job queue). A run's downloadable files are built on request from what the database holds.
- Accounts: sign-in by emailed one-time link, or through Google, Apple, Facebook or GitHub (each behind its own flag, such as `GOOGLE_SIGN_IN=on`, all off by default; see `.env.example`); session cookie. Strategies and runs have an owner; private ones are visible to their owner only.
- Code strategies: JavaScript, run in an embedded QuickJS interpreter with a 1 MB memory cap and a step budget, no clock, no randomness, no I/O. Everything that executes a strategy (previews, saves, runs) happens in a child process with a time limit, so a slow strategy is stopped instead of holding the server.
- Plain-English strategy compile: a language model reached through Vercel AI Gateway. Every description is first screened by Jev (a classifier, through the same gateway); one that is not a strategy description never reaches the model. Compile is limited per user per hour and for all users together per day.
- Usage limits: runs per month by plan, runs waiting at once, strategy checks and compiles per hour (see `.env.example`); request limits per client address in nginx.
- Everything runs in containers; no local Rust toolchain is required.

## Running locally

Requires Docker with Compose.

```bash
# Development: API rebuilds on change (cargo-watch); refresh the browser for front-end edits
docker compose up --build
# web  http://localhost:3000
# api  http://localhost:8080/api/health
# mail http://localhost:8025   (every email the app sends, including sign-in links, is caught here)

# Production-like: release binary + front end baked into the nginx image
docker compose -f compose.prod.yaml up --build
# app  http://localhost:3000
```

Both stacks include Postgres; data lives in Docker volumes (`docker compose down -v` wipes them). The production build needs an up-to-date `Cargo.lock`.

Settings that are secret or differ per deployment go in a `.env` file next to the compose files (git-ignored; start from `.env.example`):

- Plain-English compile is off until `AI_GATEWAY_API_KEY` (a Vercel AI Gateway API key) is set. `LLM_MODEL` picks the model (default `anthropic/claude-opus-5.5`).
- The production-like stack needs `PUBLIC_URL` and `SMTP_URL` for sign-in emails. Without `SMTP_URL` the links are only printed to the API log.
- A user's plan (`free` or `subscriber`) is a column in `users`; there is no billing yet, so change it with SQL.

Tests (lint, unit, API, safety and UI tests, all in Docker; `tests/README.md` says what is tested where):

```bash
./test          # everything
./test api      # one kind: lint, unit, api, safety or ui
```

## API

All routes are under `/api` and return JSON. Errors look like `{"error": {"code", "message", "details"?}}`.

| Route | What it does |
|---|---|
| `GET /api/worlds`, `GET /api/worlds/{id}` | World templates |
| `POST /api/worlds/validate` | Checks a world document: `{"valid", "errors": [{path, message}]}` |
| `POST /api/auth/email`, `POST /api/auth/email/confirm`, `POST /api/auth/sign-out` | Email a one-time link (`intent`: `sign_up` makes the account if there is none, `log_in` never does and says so in the email); trade its token for a session cookie; end the session |
| `GET /api/plans` | What each plan includes (AI requests, runs), for the sign-up page. Public |
| `GET /api/session` | `{"user": …}` for the signed-in user, `{"user": null}` otherwise |
| `GET /api/me`, `PATCH /api/me`, `GET /api/me/entitlements`, `GET /api/usage` | Profile, display name, plan flags, runs used this month (401 when signed out) |
| `GET /api/strategies`, `GET /api/strategies/{id}` | Built-in, public and own strategies: rules (state machines as data) or code |
| `POST /api/strategies`, `PUT` / `DELETE /api/strategies/{id}` | Create, replace, delete own strategies (signed in). `mode` is `rules` or `code` and cannot change |
| `POST /api/strategies/preview` | Checks a draft and plays two sample matches: `{valid, errors, warnings, samples}` (signed in; no run quota, limited per hour) |
| `POST /api/strategies/compile` | Plain English to rules through the language model (signed in; screened first, 422 `prompt_blocked` if turned down). Each call that reaches the model uses one of the plan's AI requests: 3 per world for good on the free plan, 20 a month for subscribers; 429 `ai_limit_reached` when they are used up. Also limited per hour and, for all users together, per day |
| `POST /api/runs` | Queues a run (202; signed in). Body: `world_id`, optional `actors`, `iterations`, `seed`, `policy`, `schedule_mode`. 429 `run_limit_reached` when the month's runs are used up, `too_many_runs` when too many are waiting |
| `GET /api/runs`, `GET /api/runs/{id}` | Run list; one run with status and, once completed, its results. Private runs answer 404 to everyone but their owner |
| `GET /api/runs/{id}/matches/{a}/{b}` | Round-by-round moves of one pairing |
| `GET /api/runs/{id}/race` | For every match, both sides' points so far at up to 60 sampled rounds; the results page draws the race from it |
| `GET /api/runs/{id}/artifacts`, `GET /api/runs/{id}/artifacts/{name}` | The run's files (manifest with sizes and SHA-256); the second answers with the file, built on request |
| `GET /api/worlds/{id}/leaderboard` | Ranking from the latest completed run |
| `GET /api/worlds/{id}/pairwise?run_id=` | Pairwise metrics: average payoff, outcome mix, first defection, punish duration |
| `GET /api/worlds/{id}/kpis?limit=&run_id=` | KPI snapshots per run, oldest first, with change from the run before |

## Repository structure

- `src/engine` — Rust simulation engine (pure, deterministic, no I/O)
- `src/app/api` — Rust API service, Postgres store and run worker
- `src/app/web/lilanaya-client` — vanilla JS front end (`public/` is served as-is)
- `compose.yaml`, `compose.prod.yaml` — container setup (development / production-like)
- `render.yaml` — the production setup on Render (website public; API, worker and database private)
- `spec/docs` — Technical specifications (PRD, architecture, design, stories)
- `spec/bmad` — Agentic workflow specifications and guides

## Technical references

- `spec/docs/PRD.md` — Functional scope and requirements
- `spec/docs/architecture/mvp-architecture.md` — System architecture overview
- `spec/docs/architecture/world-schema-and-kpis.md` — World schema and KPI concepts
- `spec/docs/architecture/strategy-dsl-and-sandbox.md` — Strategy model and sandbox
- `spec/docs/architecture/simulation-engine-and-scheduling.md` — Engine and scheduling
- `spec/docs/architecture/services-apis-and-deployment.md` — Services and deployment
- `spec/docs/design/user-flows-spec.md`, `spec/docs/design/wireframes.md` — UX references
