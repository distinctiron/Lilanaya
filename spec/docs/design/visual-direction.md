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

- No colour and no gradients, shadows or rounded corners. Meaning is carried by solid versus hollow and by shades of grey. If a later world needs more than two categories, ask the user before adding an accent colour.
- Borders: 3px on small squares and bars, 4px on buttons, 6px dashed under the header, 8px dashed for the centre line.
- Display type sizes: logo 18px, nav and buttons 11px, section headings 13px, ranks and scores 12px, match scores `clamp(36px, 8vw, 88px)`. All display text is upper case. Never set body copy in the display font.
- Fonts: both are open-licensed (SIL OFL). Self-host the woff2 files under `public/fonts/` rather than linking to Google Fonts, so the app works offline in its containers.

## Components

- **Header:** logo, nav links, action buttons, with the 6px dashed rule beneath. The active nav link is inverted (white block, black text). Primary button is a solid white block with black text; secondary is black with a 4px white border. Minimum control height 44px.
- **Match view:** two halves split by the dashed centre line. Player name in display type above a huge score; left side right-aligned, right side left-aligned, so both hug the line.
- **Round strip:** one column per round, top square for the left player and bottom for the right. Solid white = cooperate, hollow (3px white border) = defect, ghost outline = not yet played. Sits in a horizontally scrolling box below 720px width. Always show a legend.
- **High scores (leaderboard):** rank, name, a 20px-high bar, score. Bar width is score as a share of the maximum. Solid bar = never defects first; hollow bar = defects first.
- **Head to head (pairwise matrix):** grid of 44px cells with 5px gaps. Cell background is a grey from `rgb(30,30,30)` (lowest score) to white (highest); text flips to black once the grey is lighter than 140.
- **Section headings:** display font, 13px, upper case, no decoration.

## Motion

The page builds itself when a result is shown: rounds fill in one at a time (about 70ms each) while the match scores count up, then the leaderboard bars grow (about 600ms). Details and timings are in the mock-up's `tick()` and `renderVals()`.

- Provide a Replay control.
- Under `prefers-reduced-motion: reduce`, show the finished screen with no animation.
- Play the full sequence when a result is first shown; do not replay it on every navigation back to the same result.

## Accessibility

- All text meets 4.5:1 on black (`--muted` is about 8:1). Do not use `--ghost` for text.
- Solid versus hollow must always be explained by a visible legend and must not be the only cue in tables: keep the text label too.
- Use real `<a>`, `<button>`, `<table>` (for the leaderboard and matrix) and headings; decorative blocks get `aria-hidden="true"`.
- Layout must work at phone width: header wraps, columns stack, strips and matrices scroll inside their own box.
