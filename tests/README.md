# Tests

Everything runs in Docker through one command at the repository root. Nothing is installed on your machine.

```
./test            everything, as CI runs it
./test lint       clippy over the Rust code
./test unit       unit tests: engine, API modules, front-end modules
./test api        API tests: every route through the real router, on Postgres
./test safety     safety tests: sandbox, AI screen, limits, privacy, sessions, headers
./test ui         UI tests: Playwright against the whole site
```

Anything after the mode goes to the tool behind it:

```
./test api kpi                          API tests with "kpi" in their name
./test ui --grep "sign up"              UI tests whose title matches
./test ui --project=phone               only the iPhone project
./test ui strategies                    only tests/ui/specs/strategies.spec.js
./test ui --update-snapshots screenshots   take the screenshots again
```

`./test all` takes about five minutes once the images are built. The first `./test ui` builds the release image of the API, which takes a few minutes more.

## What is tested where

| Layer | Where | What it is for |
|---|---|---|
| Engine, unit | `src/engine/src/*.rs` (`mod tests`) | Each part of the simulation on its own |
| Engine, properties | `src/engine/tests/properties.rs` | Rules that hold for every world, strategy and seed, over 300 generated cases |
| Engine, golden files | `src/engine/tests/golden.rs`, `tests/golden/` | The engine's output, byte for byte, for four fixed runs |
| API modules, unit | `src/app/api/src/*.rs` (`mod tests`) | Config, tokens and cookies, the error shape, run files, the sandbox, the AI client |
| API routes | `src/app/api/src/routes/tests/` | Every route through the real router, one file per area |
| Who may call what | `routes/tests/access.rs` | One table: every route, asked by a visitor, the owner and another user |
| What the pages read | `routes/tests/contract.rs` | The fields the front end reads from each answer |
| Safety | `routes/tests/safety/`, and UI tests tagged `@safety` | See below |
| Front end, unit | `tests/web/*.test.mjs` | The modules that are pure logic, in Node's own test runner |
| UI | `tests/ui/specs/*.spec.js` | The journeys, in a real browser against the whole site |

### Safety

`./test safety` runs these on their own:

- `safety/sandbox.rs`: code a user wrote reaches no clock, chance, network or file; a strategy that breaks fails the run and never decides a match; slow code is stopped while the API keeps answering.
- `safety/screen.rs`: a description the screen turns down never reaches the language model; the model is shown the description as data; only a valid machine or a short refusal comes back.
- `safety/limits.rs`: every limit, with requests one at a time, arriving together, and on both sides of its time window.
- `safety/privacy.rs`: private strategies and runs are invisible to everyone else; nobody's email address or description is handed out.
- `safety/sessions.rs`: cookie attributes, forged cookies, links that work once and expire, the round trip through an outside provider.
- `safety/payments.rs`: a message Stripe did not sign just now gives nothing; what a message claims is checked with Stripe; nobody gets what somebody else paid for; a pack cannot be spent twice.
- `safety/requests.rs`: oversized, malformed and hostile requests get a plain refusal and break nothing.
- UI tests tagged `@safety`: security headers on every answer, nothing loaded from elsewhere, markup shown as text, the rate limit on sign-in links, where a visitor can be redirected.

## The UI tests

`compose.test.yaml` starts the site as production runs it: the nginx image, the release build of the API, the API and the worker as two services, and Postgres (in memory, empty at every start). Two things stand in:

- **Email**: Mailpit catches every message; the tests read the sign-up and new-password links from it.
- **Stripe**: `tests/ui/fake-stripe/server.mjs` is Stripe's API, its Checkout page and its customer portal, as plain pages with buttons ("Pay", "Pay, message arrives late", "Go back without paying"). It signs its messages to the site as Stripe does. No money moves.
- **The AI**: `tests/ui/fake-ai/server.mjs` answers for the language model and for Jev, the screen. What it answers depends on words in the description; the list is at the top of that file. The tests cost nothing and never call a real model.

The test stack gives a free account 3 runs a month, so a test can reach the limit.

Every test makes its own user (`account` in `tests/ui/fixtures.js`), so tests run side by side. They share one database: do not assert that a public list is empty or has an exact length.

Two projects run: `desktop` (Chromium, every spec but the phone's) and `phone` (an iPhone 13 in Safari's engine, `phone.spec.js`).

Motion is reduced by default, which makes the site show every animation finished and the tests steady. The tests of the animations themselves turn motion on (`test.describe('with motion on', …)`).

When a test fails, open `tests/ui/report/index.html`: each failure has a screenshot and a trace that replays the test step by step.

To look at the site the tests see: `docker compose -f compose.test.yaml up web`, then http://localhost:3100 (caught email at http://localhost:8125).

### Screenshots

`screenshots.spec.js` keeps four pictures of the Arcade look in `tests/ui/screenshots/`. After a change of design that is meant, run `./test ui --update-snapshots screenshots`, look at the new pictures, and commit them. They were taken on an Apple-silicon machine; if CI's pictures differ by more than the 1% allowed, take them again from CI's report.

## Adding tests

A feature or a fix comes with tests at the lowest layer that can show it works:

- A rule of the simulation: the engine. If it must hold for every input, add it to `properties.rs`.
- A new route: a test in the file for its area, **and a row in `access.rs`** (the suite fails without one), and its fields in `contract.rs` if a page reads them.
- A new limit, or anything a user must not be able to do: `routes/tests/safety/`.
- Logic in a front-end module: `tests/web/`.
- A new page or journey: a spec in `tests/ui/specs/`. Find elements the way a person does (role, label, text), not by CSS class where a role will do.

If the engine's output changes on purpose, regenerate the golden files and review the diff:

```
docker compose run --rm -e UPDATE_GOLDEN=1 api cargo test -p lilanaya-engine --test golden
```

API tests use `TestApp` (`routes/tests.rs`): the real router with its own database schema, an in-memory mailbox, and stand-ins for the AI gateway and the sign-in providers. `store.backdate(table, "2 hours")` makes rows older, so a test can stand on the far side of a time limit.

## CI

`.github/workflows/test.yml` runs everything on each push to `main` and each pull request, plus `cargo audit` on the dependencies. A failed UI run uploads its report.
