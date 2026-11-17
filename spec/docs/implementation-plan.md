# Implementation plan

Status as of 2026-11-17. Work top to bottom; tick an item only once it is verified. Story numbers refer to `spec/docs/stories/epic-*.md` (detail and acceptance criteria in `spec/docs/stories/issues/`). Stack decisions are in `/CLAUDE.md`.

## Where things stand

- Done: project renamed to Lilanaya (files, folder, GitHub repo, `origin` remote).
- Done and verified in Docker: Rust API skeleton (`src/app/api`, routes `/api/health` and `/api/hello`), vanilla JS front end (`src/app/web/lilanaya-client`), `compose.yaml`, `compose.prod.yaml`.
- Committed and pushed on branch `rust-arcade-foundation` (not yet merged to `main`).
- Docker note: this machine's cached Debian bookworm base layer is corrupted ("exec format error"), so the API images use Debian trixie. Avoid bookworm-based images here unless Docker's data is reset.

## Phase 0 — Make the foundation real

- [x] `docker compose up --build` starts both services; http://localhost:3000 shows "Hello from Lilanaya!"
- [x] `docker compose run --rm api cargo test` passes
- [x] `Cargo.lock` exists and `docker compose -f compose.prod.yaml up --build` works
- [x] Editing a `.rs` file triggers a rebuild in the dev container
- [x] Untrack `.DS_Store` files (`git rm --cached`)
- [ ] Ask the user: delete the obsolete TypeScript scaffold under `spec/` (see `/CLAUDE.md`)? It is untracked, so deletion is permanent. `spec/packages/engine/src/rng.ts` and `fsm.ts` are useful reference for Phase 1.
- [x] Commit (pushed on branch `rust-arcade-foundation`; the old scaffold under `spec/` was left out and is still untracked)

## Phase 1 — Engine crate (pure Rust, no I/O)

New crate `src/engine` (`lilanaya-engine`), added to the workspace. Spec: `architecture/world-schema-and-kpis.md`, `architecture/simulation-engine-and-scheduling.md`, `architecture/strategy-dsl-and-sandbox.md`.

- [ ] World model with serde: decisions, outcomes, payoff matrix, iterations, interaction policy; built-in repeated Prisoner's Dilemma template (stories 1.1, 1.2, 1.4)
- [ ] World validation with readable errors (1.2)
- [ ] Strategy as a finite state machine in a serialisable form — transitions are data, not closures (2.2, engine side only)
- [ ] Baseline actors: Always Cooperate, Always Defect, Tit-for-Tat, Win-Stay/Lose-Shift (1.7)
- [ ] Seeded RNG with derived sub-seeds per pairing (3.1)
- [ ] All-pairs scheduler (3.2)
- [ ] Pairwise execution loop producing per-actor scores, pairwise matrix, outcome distribution (3.4)
- [ ] Golden-run test: fixed seed + baseline actors gives a byte-identical result on every run (3.7, minimal form)

## Phase 2 — API over the engine (in memory, no database yet)

- [ ] `GET /api/worlds`, `GET /api/worlds/{id}` (templates), `POST /api/worlds/validate`
- [ ] `POST /api/runs` runs a simulation synchronously and returns the result; `GET /api/runs/{id}`
- [ ] `GET /api/worlds/{id}/leaderboard` from the latest run (4.1)
- [ ] Consistent JSON error shape; handler tests

## Phase 3 — Front end for the first end-to-end demo

Follow `design/user-flows-spec.md`, `design/wireframes.md`, `ux-design-specification.md`. The look and feel is settled: the "Arcade" direction in `design/visual-direction.md` (tokens, components, motion, accessibility), with the approved mock-up source in `design/reference/arcade-mockup.dc.html`. Read both before writing any front-end code.

- [ ] Design foundation: self-hosted Press Start 2P and IBM Plex Mono in `public/fonts/`, CSS custom properties for the tokens, base styles in `style.css`
- [ ] Shared components as ES modules: header with nav, buttons, section heading, bar row, matrix grid, round strip
- [ ] Client-side router and page layout
- [ ] World page: rules and payoff matrix
- [ ] Run form: pick actors, iterations, seed; submit
- [ ] Results: match view (two strategies across the dashed centre line, round strip, scores), high scores, head-to-head matrix — port of the mock-up, fed by the Phase 2 API instead of sample data
- [ ] Build-in animation on results with a Replay control; finished screen under `prefers-reduced-motion`
- [ ] Check at phone width and with keyboard only

The match view needs per-round moves for a chosen pairing, which the Phase 2 endpoints above do not list. Add that to the run result (or a `GET /api/runs/{id}/matches/{a}/{b}` route) when you reach this phase.

## Phase 4 — Persistence and background runs

- [ ] Postgres service in both compose files; migrations; persist worlds, strategies, runs
- [ ] Run queue + worker (start with a Postgres-backed job table); run status polling
- [ ] Seeded random scheduling (3.3), artifact generation (3.6), KPI snapshots and pairwise API (4.2, 4.3)

## Phase 5 — Rest of the MVP

Follow `stories/milestones-and-priorities.md` for order: strategy authoring UI and natural-language-to-FSM compile (2.1, 2.2, 2.4–2.6), world creator UI (1.3, 1.5, 1.6, 1.8, 1.9), replays and gating (4.4, 4.5), limits (3.5), auth/quotas/billing (epic 5), code strategies and sandbox (2.3, 2.7, 2.8).

## Open decisions (ask, do not assume)

- The milestones doc puts sign-in and entitlements (M0) first. This plan defers them to Phase 5 to reach a working simulation sooner — an assumption made when writing this plan, not confirmed by the user.
- How natural language compiles to an FSM (2.2) — implies an LLM call; provider and where it runs are undecided.
- Sandbox technology for code strategies (2.3).
- Object storage for artifacts (the spec says S3-compatible) versus Postgres only for the MVP.
