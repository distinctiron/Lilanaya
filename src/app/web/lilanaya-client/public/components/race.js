// The race: a run told as a league season. Every match day each competitor plays one whole
// match against one opponent (with an odd field, one sits on the bench); the chart shows the
// points gathered over the match days so far. The film goes match day by match day: the
// fixtures appear above the chart, their scores tally up while the lines move, the standings
// are gone through one competitor at a time, and the fixtures are wiped for the next match
// day. When the last one is done the fixtures go and the chart stays.
//
// On the finished chart, pointing at a line (or its label) makes it stand out so its path can be
// followed, and picking a match day (in the chart or from the buttons under it) brings back that
// day's fixtures with their final scores.
//
// The order of the fixtures is drawn from the run's seed, so a run always tells the same
// story; the final totals do not depend on it. Lines are told apart by the avatar and name at
// their end and by the roll-call, not by colour: the direction keeps colour for moves and flags.

import { h, prefersReducedMotion } from '../dom.js';
import { button } from './button.js';
import { avatar, avatarName } from './avatar.js';

const SVG = 'http://www.w3.org/2000/svg';
const FILM_MS = 30_000; // the whole film, for a field of ordinary size
const DAY_MS = [2_800, 9_000]; // the shortest and longest a match day may take
const INTRO_MS = 700; // the fixtures on show before they start
const WIPE_MS = 350; // the fixtures being wiped
const CALL_MS = 700; // one competitor in a roll-call, at most
const HEIGHT = 380; // of the chart, when its frame has not been laid out yet
const MIN_HEIGHT = 180;
const LABEL_GAP = 28; // least distance between two end labels
const NAME_CHARS = 24; // of a name beside its line; 12px monospace, so this fits the label's width

const svg = (tag, attrs = {}, children = []) => {
	const el = document.createElementNS(SVG, tag);
	for (const [key, value] of Object.entries(attrs)) if (value != null) el.setAttribute(key, String(value));
	el.append(...children);
	return el;
};
const whole = (n) => Math.round(n).toLocaleString('en');
const place = (n) => `${n}${n % 100 >= 11 && n % 100 <= 13 ? 'th' : (['th', 'st', 'nd', 'rd'][n % 10] ?? 'th')}`;

/** An axis step: about `count` steps between 0 and `max`. */
function niceStep(max, count) {
	const raw = max / count;
	const power = 10 ** Math.floor(Math.log10(raw));
	return [1, 2, 5, 10].map((m) => m * power).find((step) => step >= raw) ?? raw;
}

/** mulberry32: the fixture order comes from the run's seed, never from Math.random. */
function rng(seed) {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

/**
 * A round-robin season (the circle method) over a seeded shuffle of the field: every pair meets
 * once, everyone plays at most once a match day. Returns one list of pairs per match day; a pair
 * with `null` in it is the bench.
 */
function season(actors, seed) {
	const next = rng(Number(BigInt.asUintN(32, BigInt(seed))));
	const field = [...actors];
	for (let i = field.length - 1; i > 0; i--) {
		const j = Math.floor(next() * (i + 1));
		[field[i], field[j]] = [field[j], field[i]];
	}
	if (field.length % 2) field.push(null);
	const days = [];
	for (let day = 0; day < field.length - 1; day++) {
		days.push(Array.from({ length: field.length / 2 }, (_, i) => [field[i], field[field.length - 1 - i]]));
		field.splice(1, 0, field.pop());
	}
	return days;
}

/**
 * `actors`: [{ id, name, face }]; `matches`: [{ a, b, totals: [a's points, b's points] }], each a list with one
 * entry per sampled round in `rounds`; `seed`: the run's seed.
 * Returns { node, play(), finish(), stop() }. Until `play` is called the finished chart is on show.
 */
export function raceChart({ seed, iterations, rounds, actors, matches }) {
	const pairKey = (a, b) => (a < b ? `${a}|${b}` : `${b}|${a}`);
	const byPair = new Map(matches.map((m) => [pairKey(m.a, m.b), m]));
	/** One side's points through a match, as sampled; empty if the match is missing. */
	const series = (own, other) => {
		const m = byPair.get(pairKey(own.id, other.id));
		return m ? m.totals[m.a === own.id ? 0 : 1] : [];
	};

	// The season: per match day the fixtures and the bench, and per competitor its match that day.
	const days = season(actors, seed).map((pairs) => {
		const fixtures = pairs.filter(([a, b]) => a && b).map(([a, b]) => ({ home: a, away: b, homePoints: series(a, b), awayPoints: series(b, a) }));
		const bench = pairs.find(([a, b]) => !a || !b)?.find(Boolean) ?? null;
		return { fixtures, bench };
	});
	const D = days.length;
	const own = new Map(actors.map((actor) => [actor, days.map((day) => day.fixtures.find((f) => f.home === actor)?.homePoints ?? day.fixtures.find((f) => f.away === actor)?.awayPoints ?? null)]));
	const after = new Map(
		actors.map((actor) => {
			const sums = [0];
			for (const points of own.get(actor)) sums.push(sums.at(-1) + (points?.at(-1) ?? 0));
			return [actor, sums];
		})
	);

	/** Points in one match after the fraction `f` of its rounds, read off the line between two samples. */
	const within = (points, f) => {
		if (!points?.length || f <= 0) return 0;
		const round = f * iterations;
		let k = rounds.findIndex((r) => r >= round);
		if (k < 0) k = rounds.length - 1;
		const [r0, p0] = k === 0 ? [0, 0] : [rounds[k - 1], points[k - 1]];
		const span = rounds[k] - r0;
		return span <= 0 ? points[k] : p0 + ((points[k] - p0) * (round - r0)) / span;
	};
	/** A competitor's total at time `t`, measured in match days (2.5 is halfway through the third). */
	const totalAt = (actor, t) => {
		if (t >= D) return after.get(actor)[D];
		const day = Math.max(0, Math.floor(t));
		return after.get(actor)[day] + within(own.get(actor)[day], t - day);
	};
	const standings = (t) => [...actors].sort((a, b) => totalAt(b, t) - totalAt(a, t));
	// Every point of every line, in match days: drawn up to the moment on show.
	const trail = new Map(
		actors.map((actor) => [
			actor,
			own.get(actor).flatMap((points, day) => (points ? rounds.map((r, k) => [day + r / iterations, after.get(actor)[day] + points[k]]) : [[day + 1, after.get(actor)[day]]]))
		])
	);

	const final = standings(D);
	const lowest = Math.min(0, ...actors.flatMap((a) => trail.get(a).map(([, v]) => v)));
	const highest = Math.max(1, ...actors.flatMap((a) => trail.get(a).map(([, v]) => v)));

	const board = h('div', { class: 'fixtures', hidden: true });
	const frame = h('div', { class: 'race-frame' });
	const caption = h('p', { class: 'race-caption', 'aria-hidden': 'true' });
	// One button per match day: picking one shows how that day's matches ended.
	const dayButtons = days.map((_, day) => button(String(day + 1), { class: 'btn race-day-button', 'aria-label': `Show match day ${day + 1}`, 'aria-pressed': 'false', onclick: () => select(selected === day ? null : day) }));
	const dayRow = h('div', { class: 'race-days' }, [h('span', { class: 'meta' }, 'Match day'), ...dayButtons]);
	const node = h('figure', { class: 'race stack-sm' }, [
		board,
		frame,
		caption,
		dayRow,
		h(
			'figcaption',
			{ class: 'visually-hidden' },
			`Total points after each of ${D} match days; every competitor plays one match per match day. Final standings: ${final.map((a, i) => `${place(i + 1)} ${a.name} with ${whole(totalAt(a, D))} points`).join(', ')}.`
		)
	]);
	const say = (text) => (caption.textContent = text);

	// --- the fixtures board ---

	/**
	 * Puts match day `day` on the board; returns `score(f)`, which shows the scores after the fraction `f` of the rounds.
	 * `picked`: the day was picked from the finished chart, so the board says the scores are final and can be closed.
	 */
	const showFixtures = (day, picked = false) => {
		const { fixtures, bench } = days[day];
		const rows = fixtures.map((fixture) => {
			const home = h('span', { class: 'fixture-score' }, '0');
			const away = h('span', { class: 'fixture-score' }, '0');
			const node = h('li', { class: 'fixture' }, [
				h('span', { class: 'fixture-side fixture-home' }, avatarName(fixture.home.face, fixture.home.name, { size: 24 })),
				home,
				h('span', { class: 'fixture-dash', 'aria-hidden': 'true' }, '–'),
				away,
				h('span', { class: 'fixture-side fixture-away' }, avatarName(fixture.away.face, fixture.away.name, { size: 24 }))
			]);
			return { fixture, home, away, node };
		});
		board.replaceChildren(
			h('div', { class: 'row spread' }, [
				h('p', { class: 'fixtures-title' }, `Match day ${day + 1} of ${D}${picked ? ' · Final scores' : ''}`),
				picked ? button('Close', { 'aria-label': `Close match day ${day + 1}`, onclick: () => select(null) }) : null
			]),
			h('ul', { class: 'fixtures-list' }, rows.map((r) => r.node)),
			bench ? h('p', { class: 'fixtures-bench meta' }, ['On the bench · ', avatarName(bench.face, bench.name, { size: 24 })]) : ''
		);
		board.classList.remove('is-wiping');
		board.hidden = false;
		return (f) => {
			for (const { fixture, home, away, node } of rows) {
				const [a, b] = [within(fixture.homePoints, f), within(fixture.awayPoints, f)];
				home.textContent = whole(a);
				away.textContent = whole(b);
				// At the whistle the side with more points is marked; the name says who, not a colour.
				node.classList.toggle('home-won', f >= 1 && a > b);
				node.classList.toggle('away-won', f >= 1 && b > a);
			}
		};
	};

	// --- the chart ---

	let width = 0;
	let height = HEIGHT; // the frame's: whatever the screen has left over for the chart
	let parts = null; // the SVG pieces of the current size
	let shown = D; // the moment on show, in match days
	let called = null; // the competitor a roll-call is naming
	let hovered = null; // the competitor whose line or label is pointed at
	let selected = null; // the match day picked from the finished chart

	/** Builds the chart for the frame's width; `draw` then moves the lines. */
	const build = () => {
		width = Math.max(280, frame.clientWidth || 720);
		height = Math.max(MIN_HEIGHT, frame.clientHeight || HEIGHT);
		const wide = width >= 620;
		const box = { left: 52, right: width - (wide ? 230 : 100), top: 14, bottom: height - 30 };
		const x = (t) => box.left + ((box.right - box.left) * t) / D;
		const y = (total) => box.bottom - ((box.bottom - box.top) * (total - lowest)) / (highest - lowest);

		const grid = [];
		const step = niceStep(highest - lowest, 4);
		for (let v = Math.ceil(lowest / step) * step; v <= highest; v += step) {
			grid.push(svg('line', { class: v === 0 ? 'race-axis' : 'race-grid', x1: box.left, x2: box.right, y1: y(v), y2: y(v) }));
			grid.push(svg('text', { class: 'race-tick', x: box.left - 8, y: y(v) + 4, 'text-anchor': 'end' }, [whole(v)]));
		}
		// One mark per match day. A narrow or crowded axis names only some of them.
		const every = wide ? Math.ceil(D / 12) : D;
		for (let day = 0; day <= D; day++) {
			if (day > 0) grid.push(svg('line', { class: 'race-grid', x1: x(day), x2: x(day), y1: box.top, y2: box.bottom }));
			if (day !== 0 && day !== D && day % every) continue;
			const text = day === 0 ? 'Start' : day === D || day === every ? `Match day ${day}` : String(day);
			grid.push(svg('text', { class: 'race-tick', x: x(day), y: box.bottom + 18, 'text-anchor': day === D ? 'end' : day === 0 ? 'start' : 'middle' }, [text]));
		}

		// The column of the match day that was picked.
		const column = svg('rect', { class: 'race-day', y: box.top, height: box.bottom - box.top, visibility: 'hidden' });
		const point = (actor) => {
			if (playing || hovered === actor) return;
			hovered = actor;
			draw(shown);
		};
		const unpoint = (actor) => {
			if (hovered !== actor) return;
			hovered = null;
			draw(shown);
		};
		const lines = actors.map((actor) => {
			const path = svg('polyline', { class: 'race-line', fill: 'none' });
			// A wide, unseen copy of the line, so pointing at it does not take a steady hand.
			const hit = svg('polyline', { class: 'race-hit', fill: 'none' });
			const dot = svg('rect', { class: 'race-dot', width: 8, height: 8 });
			const face = avatar(actor.face, { size: 24 });
			// Names are cut to fit beside the chart; on a narrow screen the avatar and the total stand alone.
			const short = actor.name.length > NAME_CHARS ? `${actor.name.slice(0, NAME_CHARS - 1).trimEnd()}…` : actor.name;
			const name = wide ? svg('text', { class: 'race-name', x: 34, y: 10 }, [short]) : null;
			const total = svg('text', { class: 'race-total', x: 34, y: wide ? 24 : 16 });
			const plate = svg('rect', { class: 'race-plate', x: -4, y: -4, width: (wide ? 222 : 92) - 4, height: 32 });
			const label = svg('g', { class: 'race-label' }, [plate, face, ...(name ? [name] : []), total]);
			for (const el of [hit, label]) {
				el.addEventListener('pointerenter', () => point(actor));
				el.addEventListener('pointerleave', () => unpoint(actor));
			}
			const pixels = trail.get(actor).map(([t, v]) => [t, `${x(t).toFixed(1)},${y(v).toFixed(1)}`]);
			return { actor, path, hit, dot, label, total, pixels };
		});
		const cursor = svg('line', { class: 'race-cursor', y1: box.top, y2: box.bottom });
		const chart = svg('svg', { class: 'race-chart', viewBox: `0 0 ${width} ${height}`, width, height, 'aria-hidden': 'true' }, [
			column,
			...grid,
			cursor,
			...lines.map((l) => l.path),
			...lines.map((l) => l.hit),
			...lines.map((l) => l.dot),
			...lines.map((l) => l.label)
		]);
		// Picking a match day in the finished chart shows how its matches ended; picking it again closes it.
		chart.addEventListener('click', (e) => {
			if (playing) return;
			const at = ((e.clientX - chart.getBoundingClientRect().left - box.left) / (box.right - box.left)) * D;
			if (at < 0 || at > D) return;
			const day = Math.min(D - 1, Math.floor(at));
			select(selected === day ? null : day);
		});
		frame.replaceChildren(chart);
		parts = { box, x, y, lines, cursor, column };
	};

	/** Moves every line, dot and label to where the race stood at `t` match days. */
	const draw = (t) => {
		shown = t;
		const { box, x, y, lines, cursor, column } = parts;
		const standing = lines.map((l) => ({ l, value: totalAt(l.actor, t) })).sort((a, b) => b.value - a.value);
		// End labels sit beside their line's end, pushed apart where lines run close together.
		let floor = -Infinity;
		const slots = new Map();
		for (const { l, value } of standing) {
			const top = Math.max(y(value) - 12, floor);
			slots.set(l, top);
			floor = top + LABEL_GAP;
		}
		const overflow = Math.max(0, floor - LABEL_GAP + 24 - (box.bottom + 8));
		standing.forEach(({ l, value }, rank) => {
			const done = [];
			for (const [at, point] of l.pixels) {
				if (at >= t) break;
				done.push(point);
			}
			const points = [`${x(0).toFixed(1)},${y(0).toFixed(1)}`, ...done, `${x(t).toFixed(1)},${y(value).toFixed(1)}`].join(' ');
			l.path.setAttribute('points', points);
			l.hit.setAttribute('points', points);
			l.dot.setAttribute('x', (x(t) - 4).toFixed(1));
			l.dot.setAttribute('y', (y(value) - 4).toFixed(1));
			l.label.setAttribute('transform', `translate(${(box.right + 16).toFixed(1)},${(slots.get(l) - overflow).toFixed(1)})`);
			l.total.textContent = whole(value);
			// A roll-call names one competitor; otherwise the one pointed at stands out; otherwise the leader is white.
			const focus = called ?? hovered;
			const state = focus ? (focus !== l.actor ? 'is-dimmed' : called ? 'is-called' : 'is-pointed') : rank === 0 && t > 0 ? 'is-leading' : '';
			for (const el of [l.path, l.dot, l.label]) el.setAttribute('class', `${el.getAttribute('class').split(' ')[0]} ${state}`);
		});
		cursor.setAttribute('x1', x(t));
		cursor.setAttribute('x2', x(t));
		cursor.setAttribute('visibility', t < D ? 'visible' : 'hidden');
		if (selected == null) column.setAttribute('visibility', 'hidden');
		else {
			column.setAttribute('x', x(selected));
			column.setAttribute('width', x(selected + 1) - x(selected));
			column.setAttribute('visibility', 'visible');
		}
	};

	/** Shows the fixtures of a match day of the finished season with their final scores; `null` puts them away. */
	const select = (day) => {
		if (playing) return;
		selected = day;
		dayButtons.forEach((b, i) => b.setAttribute('aria-pressed', String(i === day)));
		if (day == null) {
			board.hidden = true;
			board.replaceChildren();
			say(closing());
		} else {
			showFixtures(day, true)(1);
			const leader = standings(day + 1)[0];
			say(`After match day ${day + 1} of ${D} · ${leader.name} leads with ${whole(totalAt(leader, day + 1))} points`);
		}
		draw(D);
	};
	const closing = () => `All ${D} match days played · ${final[0].name} wins with ${whole(totalAt(final[0], D))} points`;

	// --- the film ---

	let playing = false;
	let run = 0; // a newer play, or stop, ends an older one
	const pause = (ms, mine) => new Promise((resolve) => setTimeout(() => resolve(mine === run), ms));
	const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));

	/** The finished chart on its own, with the winner named. */
	const finish = () => {
		run += 1;
		playing = false;
		called = null;
		hovered = null;
		node.classList.remove('is-playing');
		dayRow.hidden = false;
		select(null);
	};

	/** What a competitor did in the standings over a match day, in words. */
	const movement = (before, now) => {
		if (before == null) return now === 1 ? 'leads' : '';
		if (now === 1 && before !== 1) return 'takes the lead';
		if (now < before) return `up ${before - now}`;
		if (now > before) return `down ${now - before}`;
		return now === 1 ? 'still leads' : 'holds';
	};

	/** Plays the film, match day by match day. Resolves when it has ended or been stopped. */
	const play = async () => {
		if (prefersReducedMotion()) return finish();
		const mine = ++run;
		select(null);
		playing = true;
		called = null;
		hovered = null;
		dayRow.hidden = true;
		node.classList.add('is-playing');
		const dayMs = Math.min(DAY_MS[1], Math.max(DAY_MS[0], FILM_MS / D));
		// A long season names only the top of the table after each match day.
		const calls = Math.min(actors.length, D > 8 ? 3 : 6);
		const callMs = Math.min(CALL_MS, (dayMs * 0.35) / calls);
		const playMs = dayMs - INTRO_MS - WIPE_MS - calls * callMs;
		draw(0);
		for (let day = 0; day < D; day++) {
			const before = day === 0 ? null : standings(day);
			const score = showFixtures(day);
			score(0);
			say(`Match day ${day + 1} of ${D}`);
			if (!(await pause(INTRO_MS, mine))) return;

			const started = performance.now();
			for (;;) {
				const f = Math.min(1, (performance.now() - started) / playMs);
				draw(day + f);
				score(f);
				say(`Match day ${day + 1} of ${D} · ${standings(day + f)[0].name} leads`);
				if (f >= 1) break;
				await nextFrame();
				if (mine !== run) return;
			}

			// The roll-call: who stands where after this match day, and who moved.
			const order = standings(day + 1);
			for (const [i, actor] of order.slice(0, calls).entries()) {
				called = actor;
				draw(day + 1);
				const moved = movement(before ? before.indexOf(actor) + 1 : null, i + 1);
				say(`After match day ${day + 1} · ${place(i + 1)} · ${actor.name} · ${whole(totalAt(actor, day + 1))} points${moved ? ` · ${moved}` : ''}`);
				if (!(await pause(callMs, mine))) return;
			}
			called = null;
			draw(day + 1);
			// The fixtures are wiped before the next match day; after the last they go for good.
			board.classList.add('is-wiping');
			if (!(await pause(WIPE_MS, mine))) return;
		}
		if (mine === run) finish();
	};

	const observer = new ResizeObserver(() => {
		if (Math.abs((frame.clientWidth || width) - width) < 8 && Math.abs(Math.max(MIN_HEIGHT, frame.clientHeight || height) - height) < 8) return;
		build();
		draw(shown);
	});
	observer.observe(frame);
	build();
	finish();

	return {
		node,
		play,
		finish,
		stop: () => {
			run += 1;
			playing = false;
			observer.disconnect();
		}
	};
}
