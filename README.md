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
- Everything runs in containers; no local Rust toolchain is required.

## Running locally

Requires Docker with Compose.

```bash
# Development: API rebuilds on change (cargo-watch); refresh the browser for front-end edits
docker compose up --build
# web  http://localhost:3000
# api  http://localhost:8080/api/health

# Production-like: release binary + front end baked into the nginx image
docker compose -f compose.prod.yaml up --build
# app  http://localhost:3000
```

The first development run creates `Cargo.lock`; commit it. The production build requires it.

API tests:

```bash
docker compose run --rm api cargo test
```

## Repository structure

- `src/app/api` — Rust API service
- `src/app/web/lilanaya-client` — vanilla JS front end (`public/` is served as-is)
- `compose.yaml`, `compose.prod.yaml` — container setup (development / production-like)
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
