# Open items

Noted 2026-11-22. To be tackled one by one. Each item says what was asked, what is true today, and what has to be decided or built.

## 1. Define the limits in one place

**Asked:** the plan limits should be defined somewhere.

**Today:** the numbers live as defaults in `src/app/api/src/config.rs` and can be overridden by settings on Render. The site shows them on the sign-up page, the profile page and in the strategy editor, all read from the API (`GET /api/plans`, `GET /api/usage`). There is no single written definition, and no page a logged-out visitor can open to read them apart from sign-up.

| Limit | Free | Subscriber | Agreed? |
|---|---|---|---|
| AI requests (the AI writes or rewrites rules) | 3 per world, for good | 20 per month, all worlds | Yes |
| Strategies built by hand or in code | No limit | No limit | Yes |
| Simulation runs per month | 100 | 1,000 | 100 is in the PRD; 1,000 is not confirmed |
| Runs at the same time | 2 | 2 | Not confirmed |
| Strategy checks (preview or save) per hour | 120 | 120 | Not confirmed |
| AI requests per hour (brake on bursts) | 20 | 20 | Not confirmed; rarely reached now |
| AI requests per day, all users together | 500 | (shared) | Not confirmed |
| AI tokens per day, all users together | 2,000,000 | (shared) | Not confirmed |
| Rejected descriptions before the AI is paused for a day | 5 | 5 | Not confirmed |
| Stored strategies per user (stop against filling the database) | 1,000 | 1,000 | Not confirmed |

**To decide:**
- Where the definition should live: this table in the repo, a public "Plans and limits" page on the site, or both.
- The numbers marked "not confirmed".
- From the PRD and not built: rollover of unused runs (capped at three times the monthly amount) and "one actor per world" for free accounts.

## 2. Sharing strategies between users

**Asked:** a strategy one user creates cannot be seen by another user in the Prisoner's Dilemma.

**Today:** sharing exists, but it is off for each new strategy. A strategy is saved as "Private: only you" unless its author changes the Visibility field in the editor to "Public: everyone". A public strategy is listed for everyone and can be picked in anyone's run. There is nothing in between: no sharing with one named person, and no link to share.

**To decide:**
- Whether new strategies should be public by default.
- Whether the editor should make the choice more visible when saving.
- Whether sharing with chosen people (or by link) is wanted, as opposed to public for everyone.

**Related finding from the security audit:** a public strategy shows its author's name, which is taken from the first part of their email address until they change it.

## 3. Creating worlds

**Asked:** creating worlds is not supported yet.

**Today:** correct. The only world is the built-in Prisoner's Dilemma. The API can check a world definition (`POST /api/worlds/validate`) but cannot save one, and the site has no page for it. The profile page already lists "private worlds" as a subscriber feature, which promises something that does not exist.

**To decide and build:**
- Who may create worlds (the PRD says paid accounts only).
- Storage, an editor page, visibility (private or public), and limits per plan.
- Security work that comes with it, already noted in the audit: a world's name and description must go through the same Jev screening as strategy descriptions before reaching the AI.
- The free AI allowance is per world, so every new public world gives each free user 3 more AI requests. This needs a rule before worlds can be created by users.

## 4. iPhone

**Asked:** the app renders badly on an iPhone and should be easy to use there, with scroll snapping that works and screens that are not overloaded, without changing how it looks on a computer.

**Done 2026-11-22** (all rules are in the `PHONE` block at the end of `style.css`, windows up to 720px wide):
- Header: one line (logo and a Menu button) instead of a third of the screen.
- Pages to read (front page, How it works, a world, results, strategies): one screen per phone window, and a scroll settles firmly on the next screen. Screens that were too tall were made smaller or divided (a world's introduction, rules and payoff matrix are a screen each; the points table of the third film is a screen of its own).
- Pages to type on (editor, new run, profile, sign up, log in) and single lists (worlds, runs): scroll freely, no snapping, so an opening keyboard cannot move the page.
- Fields are 16px, so iOS does not zoom in when one is tapped.
- Lists of runs and high scores: one short card per row.
- A favicon and home-screen icon.

**Checked:** in Chrome set up as an iPhone 14 (390 wide): every page measured and looked at, finger flicks settle on each screen in turn, the menu opens and closes, no page scrolls sideways. At computer size the pages are pixel-for-pixel the same as before, apart from pages that were changed on purpose (profile, sign up, log in).

**Not checked, still to do:**
- A real iPhone or Safari. The check ran in Chrome's engine; Safari's could not be installed here. Safari's moving toolbars and its snapping need a look on the device.
- Small phones (iPhone SE, 320 wide): two front-page screens and one How it works screen are 10 to 25 percent taller than the window. They can be scrolled, but they do not fit.
- The strategy editor is usable but long on a phone (two to three windows for a rules strategy). It has not been redesigned.
- Phone held sideways.
