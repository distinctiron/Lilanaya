# Lilanaya

Strategy-simulation platform: users define game-theory "worlds" (first: repeated Prisoner's Dilemma), author strategies, and run seeded, reproducible simulations. Product scope lives in `spec/docs/` (PRD, architecture, stories).

**What to build next: `spec/docs/implementation-plan.md`.** When asked to "implement", take the first unchecked item there, work down the list, and tick items off as they are verified.

## Stack (decided — overrides anything in `spec/docs` that says otherwise)

- Back end: Rust, axum on tokio. One Cargo workspace at the repo root; crates under `src/`.
- Front end: vanilla JavaScript, native ES modules, no framework, no bundler, no Node. Files in `src/app/web/lilanaya-client/public/` are served as-is by nginx.
- Everything runs in containers. There is no Rust toolchain on the host: run cargo through Docker.

## Commands

- Dev: `docker compose up --build` — web http://localhost:3000, API http://localhost:8080/api/health
- Production-like: `docker compose -f compose.prod.yaml up --build` — http://localhost:3000
- Rust tests: `docker compose run --rm api cargo test`

## Conventions

- Look and feel: the "Arcade" direction (black and white, pixel display type, dashed dividers, solid versus hollow blocks). `spec/docs/design/visual-direction.md` is the source of truth; follow it for every screen.
- All API routes are under `/api`; nginx proxies `/api` to the backend, so the front end uses relative URLs.
- Determinism is non-negotiable: every run is reproducible from its recorded seed. No wall-clock time, unseeded randomness, or hash-map iteration order in the engine.
- The name is "Lilanaya" (not "Lilanyaya").
- `spec/apps`, `spec/packages`, `spec/infra`, `spec/scripts`, `spec/README.md` are an obsolete TypeScript scaffold — reference only, do not build on it.
