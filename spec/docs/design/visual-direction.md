# Visual direction: Arcade

Chosen by the user on 2026-11-17 after comparing five directions. This is the look and feel for the whole application. Where it conflicts with visual details in `wireframes.md`, this document wins; the wireframes still govern what goes on each screen.

- Mock-up canvas (private to the user): https://claude.ai/artifact/6WaH2Rjr5XcBLw4TZjfPX2, artboard "E · Arcade".
- Mock-up source: `reference/arcade-mockup.dc.html`. It is written in a design-canvas component format (`<x-dc>`, `<sc-for>`, `{{holes}}`), so it does not run on its own. Use it as the exact reference for markup structure, sizes and the animation logic, and port it to plain HTML, CSS and ES modules.

## The idea

The feel of an early two-player arcade cabinet, inspired by Pong without copying it: a black field, white blocks, chunky pixel lettering, a thick dashed line as the divider. Two strategies facing each other across a centre line is the core image, which suits head-to-head game-theory worlds. Do not build a playable ball-and-paddle game or reuse that game's name or assets.

## Tokens

| Token | Value | Use |
|---|---|---|
| `--bg` | `#000000` | Page ground |
| `--fg` | `#FFFFFF` | Text, blocks, borders |
| `--muted` | `#A6A6A6` | Secondary text, labels, table headers |
| `--ghost` | `#2E2E2E` | Outline of squares not yet played |
| `--font-display` | `'Press Start 2P', monospace` | Logo, nav, headings, scores, ranks, buttons |
| `--font-body` | `'IBM Plex Mono', ui-monospace, monospace` | Everything else (14px, weights 400 and 600) |

- No gradients, blurred shadows or rounded corners. Two signal colours only, defined as tokens in `style.css`: blue `--coop` (#3fc5f0) = cooperate, magenta `--defect` (#e25bd8) = defect. Neither colour may read as a verdict (no red, no green, no slash): defecting is a move, not a mistake. They mark moves and are never used for decoration, body text or buttons; everything else stays black, white and grey. A move square is always filled, so that an empty-looking square cannot be read as "no points". If a later world needs more than two categories, ask the user before adding a colour.
- **Winners are never coloured:** the square that won a round gets a white outline (its border turns white). Whoever leads on the total or won a match is raised: nudged up and left over a hard-edged block shadow (`--raise`), so it stands out from the field. A leading total is also shown as a white block with a text tag ("Leads", "Wins"). The shadow is never blurred.
- Borders: 3px on small squares and bars, 4px on buttons, 6px dashed under the header, 8px dashed for the centre line.
- Display type sizes: logo 18px, nav and buttons 11px, section headings 13px, ranks and scores 12px, match scores `clamp(36px, 8vw, 88px)`. All display text is upper case. Never set body copy in the display font.
- Fonts: both are open-licensed (SIL OFL). Self-host the woff2 files under `public/fonts/` rather than linking to Google Fonts, so the app works offline in its containers.

## Components

- **Header:** logo, nav links, action buttons, with the 6px dashed rule beneath. The active nav link is inverted (white block, black text). Primary button is a solid white block with black text; secondary is black with a 4px white border. Minimum control height 44px.
- **Match view:** two halves split by the dashed centre line. Player name in display type above a huge score; left side right-aligned, right side left-aligned, so both hug the line.
- **Round strip:** one column per round, top square for the left player and bottom for the right. Blue square = cooperate, magenta square = defect, ghost outline = not yet played. Sits in a horizontally scrolling box below 720px width. Always shown with its legend.
- **High scores (leaderboard):** rank, name, a 20px-high bar, score. Bar width is score as a share of the maximum. Blue bar = never defects first; magenta bar = defects first.
- **Head to head (pairwise matrix):** grid of 44px cells with 5px gaps. Cell background is a grey from `rgb(30,30,30)` (lowest score) to white (highest); text flips to black once the grey is lighter than 140.
- **Section headings:** display font, 13px, upper case, no decoration.

- **Avatars:** every strategy has a pixel figure, 8 × 8 white blocks (`components/avatar.js`). The four built-in classics are hand-drawn; every other strategy's figure is drawn from its id, so it is the same on every screen and nothing is stored. Show the avatar wherever a strategy is named (cards, run form, match view, high scores, head to head, world page, runs list), at a multiple of 8px, always next to the name. A new strategy's avatar is built block by block the first time it appears in the strategies list.
- **Front page:** play before explanation, one idea per step (`pages/landing.js`): a hero with one promise and one action; five rounds played by hand against an opponent that has a face and talks in a speech bubble; the opponent then names itself and its rule (what a strategy is); strategies play each other and are ranked; three steps and one closing action. Explanations in depth live on `/how-it-works`. An opponent's speech bubble is a bordered box with a blocky tail towards its avatar, and reacts to what the visitor just did. Each step fills the window (`.landing-screen`); the hero shows one button and nothing of the next step. In the hand-played game each round shows the points both sides won, with the round's winner outlined in white, and the total of whoever leads raised and tagged. Squares large enough carry the letter of their move (C, D). The two sides are told apart by place, not colour: the visitor's lane and the panel with their move buttons sit on a grey band (`--lane`) with a white edge and a white "You" tag; the opponent's lane is plain, with its avatar and name. The two move buttons look the same, so neither reads as the right choice.
- **Films (How it works page):** the pictures that explain something are short films (`components/film.js`, chapters in `pages/how-it-works.js`): a framed screen with a chapter bar on top, the picture in the middle and a caption beneath, next to the list of chapters. Chapters play one after another and each is narrated by captions. A film starts by itself the first time most of its screen is in view, stops where it is when scrolled away and carries on when it is back; it plays once, then offers Replay. Pause and the chapter list (jump to a chapter) are always available. A playing film must look like it is running: a blinking light and PLAYING in its top bar, a progress bar that fills chapter by chapter, a title card before each chapter, captions typed out behind a blinking caret, and avatars that hop on the spot. All of it stops when the film is paused, and the picture dims. Films use only the parts above: avatars, move squares, the centre line, bars, pixel lettering.
- **How it works page, one screen per section:** like the front page, every section fills the window and fits inside it (`.how-screen`): three films and a closing screen. The film's picture is sized from the window's height (300px at least, 440px at most), only the chapter on show keeps its description, and anything a film points at (the points table) sits under its chapter list, not in a section of its own. Below 900px width the film and its chapters stack, and a section may be taller than the window.
- **Magnetic screens:** on the front page and on How it works the page scroll snaps to the screens (CSS scroll snap on `html:has(.landing)`): the transition between two screens is visible while scrolling, and letting go settles on one of them. Mandatory from 901px width, where every screen fits the window; only a gentle pull below that. Every section there has an `id`, so it can be linked (`/#ranking`, `/how-it-works#prisoners-dilemma`); the router scrolls to it.
- **Folding cards (strategies, worlds):** lists of strategies and worlds use `components/fold.js`, a native `<details>` in a bordered card. Folded, a card is one line: a +/- marker, the avatar and name, and its tags on the right, so many fit on a screen. Folded out, it shows the description, how the strategy works (or the world's guide and points table) and its actions. A newly made strategy starts folded out, and so does a world when it is the only one.
- **Sound (films only):** off until the viewer presses a film's Sound button; one setting for all films, remembered in the browser (`components/sound.js`). Effects and music are square and triangle waves made in the browser, no files. The voice is one recorded clip per caption under `public/audio/voice/en/`, listed in `manifest.json` (`{"clips": {"<clip name>": "<file>"}}`); the clip name is a hash of the caption's exact text, so a changed caption simply has no voice until it is recorded again. `/how-it-works?voice-script` lists every caption with its clip name. A spoken caption holds the film until the clip ends; pausing the film pauses the voice and the music. A film must stay fully understandable with sound off.

## Motion

The page builds itself when a result is shown: rounds fill in one at a time (about 70ms each) while the match scores count up, then the leaderboard bars grow (about 600ms). Details and timings are in the mock-up's `tick()` and `renderVals()`.

- Provide a Replay control.
- Under `prefers-reduced-motion: reduce`, show the finished screen with no animation. Landing-page films then show each chapter as its finished picture with its last caption, one step at a time; the hero scene shows its finished picture.
- Play the full sequence when a result is first shown; do not replay it on every navigation back to the same result.

## Accessibility

- All text meets 4.5:1 on black (`--muted` is about 8:1). Do not use `--ghost` for text.
- Colour must never be the only cue: large move squares carry their letter, the legend is always visible, and tables keep the text label too.
- Use real `<a>`, `<button>`, `<table>` (for the leaderboard and matrix) and headings; decorative blocks get `aria-hidden="true"`.
- Layout must work at phone width: header wraps, columns stack, strips and matrices scroll inside their own box.
