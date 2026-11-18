# Implementation plan

Status as of 2026-11-17. Work top to bottom; tick an item only once it is verified. Story numbers refer to `spec/docs/stories/epic-*.md` (detail and acceptance criteria in `spec/docs/stories/issues/`). Stack decisions are in `/CLAUDE.md`.

## Where things stand

- Done: project renamed to Lilanaya (files, folder, GitHub repo, `origin` remote).
- Done and verified in Docker (2026-11-17): Phases 1 to 4 below. Engine crate `src/engine`; API `src/app/api` on Postgres with a background run worker; front end `src/app/web/lilanaya-client` (worlds, world page, strategies, new run, runs, results).
- Not committed: everything since the `rust-arcade-foundation` commit `4b8e7b4` is in the working tree only.
- Docker note: this machine's cached Debian bookworm base layer is corrupted ("exec format error"), so the API images use Debian trixie and Postgres uses the alpine image. Avoid bookworm-based images here unless Docker's data is reset.

## Phase 0 — Make the foundation real

- [x] `docker compose up --build` starts both services; http://localhost:3000 shows "Hello from Lilanaya!"
- [x] `docker compose run --rm api cargo test` passes
- [x] `Cargo.lock` exists and `docker compose -f compose.prod.yaml up --build` works
- [x] Editing a `.rs` file triggers a rebuild in the dev container
- [x] Untrack `.DS_Store` files (`git rm --cached`)
- [x] Obsolete TypeScript scaffold under `spec/` deleted on the user's say-so (2026-11-18)
- [x] Commit (pushed on branch `rust-arcade-foundation`)

## Phase 1 — Engine crate (pure Rust, no I/O)

New crate `src/engine` (`lilanaya-engine`), added to the workspace. Spec: `architecture/world-schema-and-kpis.md`, `architecture/simulation-engine-and-scheduling.md`, `architecture/strategy-dsl-and-sandbox.md`.

- [x] World model with serde: decisions, outcomes, payoff matrix, iterations, interaction policy; built-in repeated Prisoner's Dilemma template (stories 1.1, 1.2, 1.4)
- [x] World validation with readable errors (1.2)
- [x] Strategy as a finite state machine in a serialisable form — transitions are data, not closures (2.2, engine side only)
- [x] Baseline actors: Always Cooperate, Always Defect, Tit-for-Tat, Win-Stay/Lose-Shift (1.7)
- [x] Seeded RNG with derived sub-seeds per pairing (3.1)
- [x] All-pairs scheduler (3.2)
- [x] Pairwise execution loop producing per-actor scores, pairwise matrix, outcome distribution (3.4)
- [x] Golden-run test: fixed seed + baseline actors gives a byte-identical result on every run (3.7, minimal form)

## Phase 2 — API over the engine (in memory, no database yet)

- [x] `GET /api/worlds`, `GET /api/worlds/{id}` (templates), `POST /api/worlds/validate`; also `GET /api/strategies` (the built-in baselines, needed by the run form)
- [x] `POST /api/runs` runs a simulation synchronously and returns the result; `GET /api/runs/{id}`; also `GET /api/runs` (list, newest first)
- [x] `GET /api/worlds/{id}/leaderboard` from the latest run (4.1)
- [x] Consistent JSON error shape; handler tests

## Phase 3 — Front end for the first end-to-end demo

Follow `design/user-flows-spec.md`, `design/wireframes.md`, `ux-design-specification.md`. The look and feel is settled: the "Arcade" direction in `design/visual-direction.md` (tokens, components, motion, accessibility), with the approved mock-up source in `design/reference/arcade-mockup.dc.html`. Read both before writing any front-end code.

- [x] Design foundation: self-hosted Press Start 2P and IBM Plex Mono in `public/fonts/`, CSS custom properties for the tokens, base styles in `style.css`
- [x] Shared components as ES modules: header with nav, buttons, section heading, bar row, matrix grid, round strip
- [x] Client-side router and page layout
- [x] World page: rules and payoff matrix
- [x] Run form: pick actors, iterations, seed; submit
- [x] Results: match view (two strategies across the dashed centre line, round strip, scores), high scores, head-to-head matrix — port of the mock-up, fed by the Phase 2 API instead of sample data
- [x] Build-in animation on results with a Replay control; finished screen under `prefers-reduced-motion`
- [x] Check at phone width and with keyboard only

Done: the match view reads per-round moves from `GET /api/runs/{id}/matches/{a}/{b}`.

## Phase 4 — Persistence and background runs

- [x] Postgres service in both compose files; migrations; persist worlds, strategies, runs
- [x] Run queue + worker (start with a Postgres-backed job table); run status polling
- [x] Seeded random scheduling (3.3): `policy: random` with `shuffle_all` and `sample_per_round`, recorded on the run
- [x] KPI snapshots (4.2) and pairwise API (4.3): `GET /api/worlds/{id}/kpis`, `GET /api/worlds/{id}/pairwise`
- [x] Artifact generation (3.6): each completed run writes `run_summary.json`, `pairwise_matrix.json`, `traces/{a}__{b}.json` and `manifest.json` (sizes, SHA-256) to S3-compatible storage under `runs/{run_id}/`; `GET /api/runs/{id}/artifacts` lists them and `GET /api/runs/{id}/artifacts/{name}` redirects to a 5-minute signed URL. Locally the storage is Versity S3 Gateway (MinIO's images are no longer published). Traces are also still kept in Postgres, which is what the match view reads.

Not done in this phase, by choice: the pairwise API's cohort and round-window filters (4.3 marks them subscriber-only, so they belong with gating in Phase 5), and the KPI "compare last N runs" screen (the API returns the history with deltas; the world page shows the latest run against the one before).

## Phase 5 — Rest of the MVP

Order follows `stories/milestones-and-priorities.md`, with accounts first (user, 2026-11-18).

### 5a — Accounts (M0: 5.1, 5.2 stub, 5.4 readout, 5.5)

- [x] Sign in by emailed link; session cookie; sign out; profile page (email, display name, roles, plan, runs left this month).
- [x] Entitlement flags from the user's plan (`/api/me/entitlements`), no billing yet.
- [x] Quota readout (`/api/usage`): runs used this month against the plan's allowance. Not enforced yet.
- [x] Strategies and runs have an owner; private ones answer 404 to everyone else and are left out of lists and world analytics.
- [ ] Sign in with Google and GitHub — waiting for OAuth app credentials from the user.

### 5b — Strategy authoring (2.1, 2.2, 2.4)

- [x] Create, edit and delete own strategies; mode (rules or code) is chosen once and then fixed.
- [x] Rules editor: states, the decision each plays, and conditions for moving between them.
- [ ] Plain English to rules through an LLM (Claude API key for dev, Azure-hosted Claude endpoint in production); the result is checked by the engine's validator and can be corrected in the editor. Built and tested against a stand-in server for both providers; not ticked until it has run against the real Claude API (needs the user's key in `.env`).
- [x] Static preview: problems, warnings (unreachable states, dead ends) and two sample matches (v Always Cooperate, v Always Defect); uses no quota.
- [x] Own strategies can be entered in runs.

### 5c — Code strategies (2.3)

- [x] JavaScript `decide(input)` strategies run in a sandbox with a step budget and a memory cap, no clock, no randomness, no I/O.
- [x] Save runs the static checks (size, compiles, defines `decide`, returns allowed decisions in a dry run).

### Later, in this order

- [ ] World creator (1.3, 1.5, 1.6, 1.8, 1.9).
- [ ] Quick test (2.5), replays and subscriber gating (4.4, 4.5, pairwise filters), limits (3.5).
- [ ] Billing, usage packs, enforced quotas with rollover (5.2 full, 5.3, 5.4), webhooks (5.7), artifact access audit (5.6).
- [ ] Strategy versioning and public strategy pages (2.7, 2.8).

## Decided

- Artifacts go to S3-compatible object storage (user, 2026-11-18).
- Accounts come before authoring (user, 2026-11-18).
- Plain-English compile uses Claude: an API key in dev/test, an Azure-provisioned endpoint in production; both must work (user, 2026-11-18). Built on the assumption that the Azure endpoint is Claude in Microsoft Foundry (same Messages API) — confirm with the user.
- Code strategies are written in JavaScript (user, 2026-11-18).

## Open decisions (ask, do not assume)

- OAuth app credentials for Google and GitHub sign-in, and the SMTP service for sign-in emails in production.
- Payment provider for subscriptions (needed for the billing item).
- Whether the embedded JavaScript interpreter is isolation enough for public launch, or code strategies must move to a separate process or WebAssembly first.
