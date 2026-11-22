import { getJson } from '../api.js';
import { avatar, avatarName } from '../components/avatar.js';
import { barRow } from '../components/bar-row.js';
import { button, linkButton } from '../components/button.js';
import { matrixGrid } from '../components/matrix-grid.js';
import { raceChart } from '../components/race.js';
import { roundStrip, stripLegend } from '../components/round-strip.js';
import { screen, screenGroup, screens } from '../components/screen.js';
import { sectionHeading } from '../components/section-heading.js';
import { fmt, fmtValue, h, prefersReducedMotion } from '../dom.js';

const MAX_SHOWN_ROUNDS = 40;
const ROUND_MS = 70; // one round of the strip
const BAR_STEPS = 20; // the bars grow over 20 × 30ms = 600ms
const BAR_MS = 30;
const POLL_MS = 500; // while a run is queued or running

const SCHEDULE = { shuffle_all: 'Random order, shuffled once', sample_per_round: 'Random order, reshuffled every round' };

// What has already played its build-in, so going back to a result does not replay it.
// Kept for the browser session; falls back to memory if storage is unavailable.
const seen = (() => {
	const KEY = 'lilanaya.seen';
	let set = new Set();
	try {
		set = new Set(JSON.parse(sessionStorage.getItem(KEY) ?? '[]'));
	} catch {
		// Storage blocked or corrupt: start empty.
	}
	return {
		has: (key) => set.has(key),
		add: (key) => {
			set.add(key);
			try {
				sessionStorage.setItem(KEY, JSON.stringify([...set].slice(-200)));
			} catch {
				// Memory only.
			}
		}
	};
})();

/** Words for the two decisions of a cooperate/defect world; other worlds keep their own names. */
function decisionNames(decisions) {
	const isCD = decisions.length === 2 && decisions[0] === 'C' && decisions[1] === 'D';
	return {
		isCD,
		solid: isCD ? 'Cooperate' : decisions[0],
		hollow: isCD ? 'Defect' : decisions.length === 2 ? decisions[1] : 'Any other decision',
		word: (d) => (isCD ? { C: 'cooperate', D: 'defect' }[d] : d)
	};
}

/** The match view: two players across the centre line, the round strip and its legend. */
function buildMatch({ trace, world, names, roundLabel, faces }) {
	const total = trace.iterations;
	const shownCount = Math.min(MAX_SHOWN_ROUNDS, total);
	const d = trace.decisions;
	const cumA = [];
	const cumB = [];
	let a = 0;
	let b = 0;
	for (let i = 0; i < total; i++) {
		const [pa, pb] = world.payoff.matrix[d[trace.moves_a[i]]][d[trace.moves_b[i]]];
		cumA.push((a += pa));
		cumB.push((b += pb));
	}

	const movesA = trace.moves_a.slice(0, shownCount);
	const movesB = trace.moves_b.slice(0, shownCount);
	const tally = (moves) => d.map((name, i) => `${moves.filter((m) => m === i).length} ${names.word(name)}`).join(', ');
	const strip = roundStrip({
		movesA,
		movesB,
		label: `First ${shownCount} of ${total} rounds. ${trace.a.name}: ${tally(movesA)}. ${trace.b.name}: ${tally(movesB)}.`
	});

	const scoreA = h('div', { class: 'match-score' });
	const scoreB = h('div', { class: 'match-score' });
	const node = [
		h('div', { class: 'row spread' }, [sectionHeading(`Match: ${trace.a.name} versus ${trace.b.name}`), h('div', { class: 'meta' }, roundLabel)]),
		h('p', { class: 'meta match-note' }, 'One of the matches of this run, round by round · The big numbers are the points each side won in this match alone'),
		h('div', { class: 'match' }, [
			h('div', { class: 'match-side match-left' }, [avatar(faces[0], { size: 48 }), h('div', { class: 'match-name' }, trace.a.name), scoreA]),
			h('div', { class: 'centre-line', 'aria-hidden': 'true' }),
			h('div', { class: 'match-side match-right' }, [avatar(faces[1], { size: 48 }), h('div', { class: 'match-name' }, trace.b.name), scoreB])
		]),
		strip.node,
		stripLegend({ top: trace.a.name, bottom: trace.b.name, solid: names.solid, hollow: names.hollow }),
		shownCount < total ? h('p', { class: 'meta' }, `Scores are totals over all ${total} rounds`) : null
	];

	/** Shows the first `k` rounds of the strip; the scores count up to the final totals in step. */
	const show = (k) => {
		strip.show(k);
		const round = k >= shownCount ? total : Math.ceil((k * total) / shownCount);
		scoreA.textContent = String(round ? cumA[round - 1] : 0);
		scoreB.textContent = String(round ? cumB[round - 1] : 0);
		roundLabel.textContent = `Round ${k} / ${shownCount} shown of ${total}`;
	};
	return { node, show, steps: shownCount };
}

/** Shown while the worker has not finished the run; checks again until it has, then redraws the page. */
function waitingView(run, { refresh, onLeave }) {
	let timer = 0;
	let left = false;
	const poll = async () => {
		try {
			const now = await getJson(`/api/runs/${run.id}`);
			if (left) return;
			if (now.status !== 'queued' && now.status !== 'running') return refresh();
		} catch {
			// Keep trying: the server may be restarting.
		}
		if (!left) timer = setTimeout(poll, POLL_MS);
	};
	timer = setTimeout(poll, POLL_MS);
	onLeave(() => {
		left = true;
		clearTimeout(timer);
	});
	return {
		title: `Run ${run.id}`,
		node: h('section', { class: 'stack-md' }, [
			sectionHeading(`Run ${run.id}`, { level: 1 }),
			h('p', { class: 'meta' }, `${run.world_name} · Seed ${run.seed} · ${run.actor_count} actors · ${run.iterations} rounds per match`),
			h('p', { class: 'section-heading', role: 'status' }, run.status === 'queued' ? 'Waiting in the queue…' : 'Running…'),
			h('p', { class: 'muted' }, 'The results appear here as soon as the run finishes.')
		])
	};
}

function failedView(run) {
	const details = run.error?.details ?? [];
	return {
		title: `Run ${run.id}`,
		node: h('section', { class: 'stack-md' }, [
			sectionHeading(`Run ${run.id}`, { level: 1 }),
			h('p', { class: 'meta' }, `${run.world_name} · Seed ${run.seed} · ${run.actor_count} actors`),
			h('div', { class: 'error-box', role: 'alert' }, [
				h('p', null, `This run failed. ${run.error?.message ?? ''}`),
				details.length ? h('ul', null, details.map((d) => h('li', null, `${d.path}: ${d.message}`))) : null
			]),
			h('p', null, linkButton('New run', `/runs/new?world=${run.world_id}`, { primary: true }))
		])
	};
}

function kpiSection(run) {
	if (!run.kpis?.length) return null;
	return h('div', { class: 'stack' }, [
		sectionHeading('Indicators'),
		h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Indicators' }, [
			h('table', { class: 'grid-table' }, [
				h('thead', null, h('tr', null, [h('th', { scope: 'col' }, 'Indicator'), h('th', { scope: 'col' }, 'This run')])),
				h('tbody', null, run.kpis.map((k) => h('tr', null, [h('th', { scope: 'row' }, k.label), h('td', { class: 'num' }, fmtValue(k.value))])))
			])
		])
	]);
}

/** Points as a whole number with thousands separators. */
const points = (n) => Math.round(n).toLocaleString('en');

const kb = (bytes) => (bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`);

/** The run's files in object storage. Each link goes through the API, which redirects to a short-lived signed URL. */
function downloadsSection(files) {
	if (!files.length) return null;
	const link = (f, label) => h('li', null, [h('a', { href: f.href }, label ?? f.name), h('span', { class: 'muted' }, `  ${kb(f.size_bytes)}`)]);
	const main = ['run_summary.json', 'pairwise_matrix.json', 'manifest.json'].map((name) => files.find((f) => f.name === name)).filter(Boolean);
	const traces = files.filter((f) => f.name.startsWith('traces/'));
	return h('div', { class: 'stack' }, [
		sectionHeading('Downloads'),
		h('ul', { class: 'plain-list tight' }, main.map((f) => link(f))),
		traces.length
			? h('details', null, [
					h('summary', null, `Round-by-round traces (${traces.length} matches)`),
					h('ul', { class: 'plain-list tight' }, traces.map((f) => link(f, f.name.slice('traces/'.length))))
				])
			: null,
		h('p', { class: 'meta' }, 'JSON files · The manifest lists every file with its size and SHA-256 checksum')
	]);
}

export async function resultsPage(ctx) {
	const { params, query, replaceUrl, onLeave } = ctx;
	const run = await getJson(`/api/runs/${encodeURIComponent(params.id)}`);
	if (run.status === 'queued' || run.status === 'running') return waitingView(run, ctx);
	if (run.status === 'failed') return failedView(run);
	const [world, { artifacts }, raceData] = await Promise.all([
		getJson(`/api/worlds/${encodeURIComponent(run.world_id)}`),
		getJson(`/api/runs/${run.id}/artifacts`),
		getJson(`/api/runs/${run.id}/race`)
	]);
	const names = decisionNames(run.decisions);
	const face = (actor) => actor.strategy_id ?? actor.id; // avatars belong to strategies
	const byId = new Map(run.actors.map((actor, index) => [actor.id, { ...actor, index }]));
	const ranked = run.ranking.map((id) => byId.get(id));

	// --- the race: the run as a season of match days, total points after each ---
	const race = raceChart({
		seed: run.seed,
		rounds: raceData.rounds,
		iterations: raceData.iterations,
		matches: raceData.matches,
		actors: ranked.map((actor) => ({ id: actor.id, name: actor.name, face: face(actor) }))
	});
	onLeave(() => race.stop());

	// --- high scores ---
	const best = Math.max(...ranked.map((actor) => actor.total));
	const bars = ranked.map((actor, i) => {
		return barRow({
			rank: i + 1,
			name: avatarName(face(actor), actor.name),
			score: points(actor.total),
			share: best > 0 ? Math.max(0, actor.total) / best : 0
		});
	});
	const growBars = (g) => bars.forEach((bar) => bar.grow(g));
	const highScores = screen(
		[
		sectionHeading('High scores'),
		h('div', null, [
			h('table', { class: 'bars' }, [
				h(
					'thead',
					null,
					h('tr', { class: 'meta' }, [
						h('th', { scope: 'col' }, h('span', { class: 'visually-hidden' }, 'Rank')),
						h('th', { scope: 'col' }, 'Actor'),
						h('td', null),
						h('th', { scope: 'col', class: 'score-head' }, 'Total')
					])
				),
				h('tbody', null, bars.map((bar) => bar.row))
			])
		]),
		h('p', { class: 'meta' }, 'Total points over all matches')
		],
		{ class: 'stack high-scores' }
	);

	// --- match view ---
	const roundLabel = h('div', null);
	// The match view stays away until a match is picked from the head-to-head table (or the address names one).
	const matchSection = screen([sectionHeading('Match'), h('p', { class: 'muted prose' }, 'Pick a cell in the head-to-head table to watch that match here, round by round.')], { class: 'stack-md is-empty' });
	let match = null;
	let timer = 0;
	let request = 0;
	onLeave(() => {
		clearTimeout(timer);
		request = -1;
	});

	/** Plays the build-in: rounds fill in one at a time, then the bars grow. Either part can be skipped. */
	const play = ({ strip, grow }) => {
		clearTimeout(timer);
		if (!match) return;
		let k = strip ? 0 : match.steps;
		let g = grow ? 0 : BAR_STEPS;
		match.show(k);
		growBars(g / BAR_STEPS);
		const tick = () => {
			if (k < match.steps) {
				match.show(++k);
				timer = setTimeout(tick, k < match.steps ? ROUND_MS : BAR_MS);
			} else if (g < BAR_STEPS) {
				growBars(++g / BAR_STEPS);
				timer = setTimeout(tick, BAR_MS);
			}
		};
		tick();
	};

	const pairHref = (a, b) => `/runs/${run.id}?a=${encodeURIComponent(a)}&b=${encodeURIComponent(b)}`;
	const matrix = matrixGrid({
		actors: ranked.map((actor) => ({ ...actor, face: face(actor) })),
		values: ranked.map((row) => ranked.map((col) => run.pairwise[row.index][col.index])),
		caption: 'Head to head: average points per round of the row actor against the column actor',
		format: fmt,
		unit: 'points per round',
		hrefFor: (row, col) => pairHref(row.id, col.id),
		onPick: (row, col, e) => {
			e.preventDefault();
			replaceUrl(pairHref(row.id, col.id));
			showMatch(row.id, col.id).then(() => matchSection.scrollIntoView({ block: 'nearest', behavior: prefersReducedMotion() ? 'auto' : 'smooth' }));
		}
	});

	async function showMatch(a, b) {
		const mine = ++request;
		const trace = await getJson(`/api/runs/${run.id}/matches/${encodeURIComponent(a)}/${encodeURIComponent(b)}`);
		if (mine !== request) return; // a newer pick, or the page was left
		match = buildMatch({ trace, world, names, roundLabel, faces: [face(byId.get(a)), face(byId.get(b))] });
		matchSection.replaceChildren(...match.node.filter(Boolean));
		matchSection.classList.remove('is-empty');
		matrix.select(a, b);

		// Its rounds fill in one at a time the first time it is shown.
		const matchKey = `run:${run.id}:${a}:${b}`;
		play({ strip: !prefersReducedMotion() && !seen.has(matchKey), grow: false });
		seen.add(matchKey);
	}

	// A match is on show from the start only when the address names a real pairing.
	const [a, b] = [query.get('a'), query.get('b')];
	const named = byId.has(a) && byId.has(b) && a !== b;
	if (named) await showMatch(a, b);

	// The race plays the first time a result is opened; after that it is shown finished, with Replay.
	const runKey = `run:${run.id}`;
	if (!seen.has(runKey) && !named) requestAnimationFrame(() => race.play());
	seen.add(runKey);

	const extras = [kpiSection(run), downloadsSection(artifacts)].filter(Boolean);
	return {
		title: `Run ${run.id}`,
		// The race has a screen to itself; the rest share screens where the window is wide.
		node: screens([
			screen(
				[
					h('div', { class: 'row spread' }, [
						sectionHeading(`Run ${run.id}`, { level: 1 }),
						button('Replay', {
							onclick: () => {
								window.scrollTo({ top: 0 });
								race.play();
							}
						})
					]),
					h('div', { class: 'meta' }, [h('a', { href: `/worlds/${run.world_id}` }, run.world_name), ` · Seed ${run.seed} · ${run.actors.length} actors · ${points(run.iterations)} rounds per match`, run.schedule_mode ? ` · ${SCHEDULE[run.schedule_mode] ?? run.schedule_mode}` : '']),
					// The race takes whatever height the window has left, so the fixtures, the chart and its caption share one screen.
					race.node,
					h('p', { class: 'meta' }, 'Total points after each match day · Everyone plays one match per match day · The order is drawn from the seed and does not change the final totals')
				],
				{ class: 'stack race-screen' }
			),
			// On a laptop these share screens: the final table beside the small print, and head to head beside the match picked from it.
			extras.length ? screenGroup([highScores, screen(extras, { class: 'stack-lg' })]) : highScores,
			screenGroup([
				screen([sectionHeading('Head to head'), matrix.node, h('p', { class: 'meta' }, 'Row’s average points per round against column · Pick a cell to watch that match')], { class: 'stack head-to-head' }),
				matchSection
			])
		])
	};
}
