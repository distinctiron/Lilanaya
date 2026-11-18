import { getJson } from '../api.js';
import { barRow } from '../components/bar-row.js';
import { button, linkButton } from '../components/button.js';
import { matrixGrid } from '../components/matrix-grid.js';
import { roundStrip, stripLegend } from '../components/round-strip.js';
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

/** True if the actor ever played D no later than its opponent did (it was not provoked). */
function defectsFirst(run, actorId) {
	return run.matches.some((m) => {
		const side = m.a === actorId ? 0 : m.b === actorId ? 1 : -1;
		if (side < 0) return false;
		const mine = m.first_played.D[side];
		const theirs = m.first_played.D[1 - side];
		return mine != null && (theirs == null || mine <= theirs);
	});
}

/** The match view: two players across the centre line, the round strip and its legend. */
function buildMatch({ trace, world, names, roundLabel }) {
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
		h('h2', { class: 'visually-hidden' }, `Match: ${trace.a.name} versus ${trace.b.name}`),
		h('div', { class: 'match' }, [
			h('div', { class: 'match-side match-left' }, [h('div', { class: 'match-name' }, trace.a.name), scoreA]),
			h('div', { class: 'centre-line', 'aria-hidden': 'true' }),
			h('div', { class: 'match-side match-right' }, [h('div', { class: 'match-name' }, trace.b.name), scoreB])
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
	return h('section', { class: 'stack' }, [
		sectionHeading('Indicators'),
		h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Indicators' }, [
			h('table', { class: 'grid-table' }, [
				h('thead', null, h('tr', null, [h('th', { scope: 'col' }, 'Indicator'), h('th', { scope: 'col' }, 'This run')])),
				h('tbody', null, run.kpis.map((k) => h('tr', null, [h('th', { scope: 'row' }, k.label), h('td', { class: 'num' }, fmtValue(k.value))])))
			])
		])
	]);
}

const kb = (bytes) => (bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`);

/** The run's files in object storage. Each link goes through the API, which redirects to a short-lived signed URL. */
function downloadsSection(files) {
	if (!files.length) return null;
	const link = (f, label) => h('li', null, [h('a', { href: f.href }, label ?? f.name), h('span', { class: 'muted' }, `  ${kb(f.size_bytes)}`)]);
	const main = ['run_summary.json', 'pairwise_matrix.json', 'manifest.json'].map((name) => files.find((f) => f.name === name)).filter(Boolean);
	const traces = files.filter((f) => f.name.startsWith('traces/'));
	return h('section', { class: 'stack' }, [
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
	const [world, { artifacts }] = await Promise.all([
		getJson(`/api/worlds/${encodeURIComponent(run.world_id)}`),
		getJson(`/api/runs/${run.id}/artifacts`)
	]);
	const names = decisionNames(run.decisions);
	const byId = new Map(run.actors.map((actor, index) => [actor.id, { ...actor, index }]));
	const ranked = run.ranking.map((id) => byId.get(id));

	// --- high scores ---
	const best = Math.max(...ranked.map((actor) => actor.average));
	const bars = ranked.map((actor, i) => {
		const first = names.isCD ? defectsFirst(run, actor.id) : null;
		return barRow({
			rank: i + 1,
			name: actor.name,
			score: fmt(actor.average),
			share: best > 0 ? Math.max(0, actor.average) / best : 0,
			solid: first !== true,
			label: first == null ? null : first ? 'Yes' : 'No'
		});
	});
	const growBars = (g) => bars.forEach((bar) => bar.grow(g));
	const highScores = h('section', { class: 'stack high-scores' }, [
		sectionHeading('High scores'),
		h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'High scores' }, [
			h('table', { class: 'bars' }, [
				h(
					'thead',
					null,
					h('tr', { class: 'meta' }, [
						h('th', { scope: 'col' }, h('span', { class: 'visually-hidden' }, 'Rank')),
						h('th', { scope: 'col' }, 'Actor'),
						h('td', null),
						h('th', { scope: 'col', class: 'score-head' }, 'Avg'),
						names.isCD ? h('th', { scope: 'col' }, 'Defects first') : null
					])
				),
				h('tbody', null, bars.map((bar) => bar.row))
			])
		]),
		h('p', { class: 'meta' }, [
			'Average payoff per round',
			names.isCD ? ' · Solid bar never defects first · Hollow bar defects first' : ''
		])
	]);

	// --- match view ---
	const roundLabel = h('div', null);
	const matchSection = h('section', { class: 'stack-md', 'aria-busy': 'true' });
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
		actors: ranked,
		values: ranked.map((row) => ranked.map((col) => run.pairwise[row.index][col.index])),
		caption: 'Head to head: average payoff per round of the row actor against the column actor',
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
		match = buildMatch({ trace, world, names, roundLabel });
		matchSection.replaceChildren(...match.node.filter(Boolean));
		matchSection.removeAttribute('aria-busy');
		matrix.select(a, b);

		const runKey = `run:${run.id}`;
		const matchKey = `${runKey}:${a}:${b}`;
		const moving = !prefersReducedMotion();
		play({ strip: moving && !seen.has(matchKey), grow: moving && !seen.has(runKey) });
		seen.add(runKey);
		seen.add(matchKey);
	}

	// The match on show: from the URL if it names a real pairing, otherwise first against second.
	let [a, b] = [query.get('a'), query.get('b')];
	if (!byId.has(a) || !byId.has(b) || a === b) [a, b] = [ranked[0].id, ranked[1].id];
	await showMatch(a, b);

	return {
		title: `Run ${run.id}`,
		node: h('div', { class: 'stack-lg' }, [
			h('section', { class: 'stack-md' }, [
				h('div', { class: 'row spread' }, [
					sectionHeading(`Run ${run.id}`, { level: 1 }),
					button('Replay', { onclick: () => play({ strip: true, grow: true }) })
				]),
				h('div', { class: 'row spread meta' }, [
					h('div', null, [h('a', { href: `/worlds/${run.world_id}` }, run.world_name), ` · Seed ${run.seed} · ${run.actors.length} actors`, run.schedule_mode ? ` · ${SCHEDULE[run.schedule_mode] ?? run.schedule_mode}` : '']),
					roundLabel
				])
			]),
			matchSection,
			h('div', { class: 'columns' }, [
				highScores,
				h('section', { class: 'stack head-to-head' }, [
					sectionHeading('Head to head'),
					matrix.node,
					h('p', { class: 'meta' }, 'Row’s average per round against column · Pick a cell to watch that match')
				])
			]),
			kpiSection(run),
			downloadsSection(artifacts)
		])
	};
}
