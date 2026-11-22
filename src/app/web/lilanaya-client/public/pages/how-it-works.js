import { avatar, revealAvatar } from '../components/avatar.js';
import { linkButton } from '../components/button.js';
import { film, filmCaptions } from '../components/film.js';
import { CLASSICS, PAYOFF, copyThem, playAll, playMatch } from '../components/pocket-dilemma.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { getUser, signInUrl } from '../session.js';

// The visitor's own strategy in the "how it works" pictures: version 1 trusts everyone, version 2 copies.
const YOU = 'challenger';
const RIVAL = 'rival';
const YOU_V1 = { name: 'You', first: 'C', next: () => 'C' };
const YOU_V2 = { name: 'You', first: 'C', next: copyThem };

// --- small parts the pictures are made of ---

const ROUNDS = 10; // rounds per match in the pictures

/** Pixel lettering inside a scene. */
const px = (text, extra = '') => h('span', { class: `px ${extra}`.trim() }, text);

/** A move square: blue = cooperate, magenta = defect, ghost outline = not played yet. */
const chip = (move = '', extra = '') => h('span', { class: `chip ${extra}`.trim(), 'data-move': move });
const setChip = (el, move) => el.setAttribute('data-move', move);
const show = (el, on = true) => el.classList.toggle('off', !on);

/** A sentence of words and move squares: line('They play', 'C', '>', 'I play', 'C'). */
function line(...parts) {
	return h('div', { class: 'rule-line' }, parts.map((part) => (part === 'C' || part === 'D' ? chip(part) : px(part))));
}

/** Two rows of move squares, one column per round. */
function miniStrip(rounds = ROUNDS) {
	const cols = Array.from({ length: rounds }, () => [chip(), chip()]);
	return {
		node: h('div', { class: 'mini-strip' }, cols.map((col) => h('span', { class: 'mini-round' }, col))),
		set(i, a, b) {
			setChip(cols[i][0], a);
			setChip(cols[i][1], b);
		},
		clear() {
			for (const col of cols) for (const el of col) setChip(el, '');
		}
	};
}

/** A ranked row: place, avatar, bar, score. */
function boardRow(id, { hollow = false, you = false } = {}) {
	const rank = px('', 'board-rank');
	const bar = h('span', { class: 'bar' });
	const score = px('0', 'board-score');
	const node = h('div', { class: you ? 'board-row is-you' : 'board-row' }, [rank, avatar(id, { size: 24 }), h('span', { class: 'board-track' }, bar), score]);
	return {
		id,
		node,
		set(total, max, place = '') {
			rank.textContent = place;
			score.textContent = String(total);
			bar.style.width = `${Math.max(3, (total / max) * 100).toFixed(1)}%`;
		}
	};
}

/** Puts board rows in order of score, best first; equal scores share a place. */
function rankRows(list, rows, totals, max) {
	const sorted = [...rows].sort((a, b) => totals[b.id] - totals[a.id] || (b.id === YOU) - (a.id === YOU));
	list.replaceChildren(...sorted.map((row) => row.node));
	sorted.forEach((row, i) => {
		const place = sorted.findIndex((other) => totals[other.id] === totals[row.id]) + 1;
		row.set(totals[row.id], max, String(place));
		row.node.classList.toggle('is-top', i === 0);
	});
}

/** A blocky arrow pointing down. */
function downArrow() {
	const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
	for (const [name, value] of Object.entries({ class: 'arrow', viewBox: '0 0 7 7', width: '28', height: '28', 'shape-rendering': 'crispEdges' })) svg.setAttribute(name, value);
	const path = document.createElementNS(svg.namespaceURI, 'path');
	path.setAttribute('d', 'M2 0h3v3h2v1H0V3h2zM1 4h5v1H1zM2 5h3v1H2zM3 6h1v1H3z');
	svg.append(path);
	return svg;
}

const legend = () =>
	h('div', { class: 'legend meta' }, [
		h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-solid', 'aria-hidden': 'true' }), 'Cooperate']),
		h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-hollow', 'aria-hidden': 'true' }), 'Defect'])
	]);

// --- two players across the centre line ---

/** One side of a duel: name, avatar, score, and next to the line either a move on its way or a face-down card. */
function duelSide(where, { card = false, face = 64 } = {}) {
	const name = px('', 'duel-name');
	const holder = h('div', { class: 'duel-face' });
	const score = h('div', { class: 'duel-score' });
	const shot = card ? chip('', 'card') : chip('', 'shot');
	const mark = card ? px('?', 'card-mark') : null;
	const info = h('div', { class: 'duel-info' }, [name, holder, score]);
	const slot = card ? h('div', { class: 'card-slot' }, [shot, mark]) : shot;
	return {
		name,
		score,
		shot,
		mark,
		node: h('div', { class: `duel-side duel-${where}` }, where === 'left' ? [info, slot] : [slot, info]),
		set(id, label) {
			name.textContent = label;
			name.classList.remove('is-winner');
			holder.replaceChildren(avatar(id, { size: face }));
			score.textContent = card ? '' : '0';
			shot.classList.remove('fly');
		}
	};
}

function duel(options) {
	const left = duelSide('left', options);
	const right = duelSide('right', options);
	return { left, right, node: h('div', { class: 'duel' }, [left.node, h('div', { class: 'centre-line' }), right.node]) };
}

/** Plays round `i` of `match` on a duel and its strip: both moves fly to the line, then land in the strip and pay. */
async function playRound(t, { left, right }, strip, match, i, { flight = 300, rest = 260 } = {}) {
	setChip(left.shot, match.a[i]);
	setChip(right.shot, match.b[i]);
	left.shot.classList.add('fly');
	right.shot.classList.add('fly');
	t.sfx?.('move');
	await t.wait(flight);
	t.sfx?.('point');
	strip.set(i, match.a[i], match.b[i]);
	left.score.textContent = String(match.scoreA[i]);
	right.score.textContent = String(match.scoreB[i]);
	await t.wait(rest);
	left.shot.classList.remove('fly');
	right.shot.classList.remove('fly');
	await t.wait(90);
}

// --- film 1: what it is ---

const tftRules = () => [line('First move', '>', 'C'), line('They play', 'C', '>', 'C'), line('They play', 'D', '>', 'D')];

const WHAT_IT_IS = [
	{
		title: 'A world',
		text: 'A world is a game with fixed rules: who plays, which moves are allowed, how many rounds, and what each outcome pays.',
		async play(stage, t) {
			const field = h('div', { class: 'mini-field' }, [avatar(YOU, { size: 24 }), h('span', { class: 'mini-line' }), avatar(RIVAL, { size: 24 })]);
			const cells = ['3 3', '0 5', '5 0', '1 1'].map((text) => px(text, 'pay-cell'));
			const table = h('div', { class: 'pay-grid' }, [h('span'), chip('C'), chip('D'), chip('C'), cells[0], cells[1], chip('D'), cells[2], cells[3]]);
			const rows = [
				h('div', { class: 'rule-line' }, [px('Players'), px('2', 'strong')]),
				h('div', { class: 'rule-line' }, [px('Moves'), chip('C'), chip('D')]),
				h('div', { class: 'rule-line' }, [px('Rounds'), px('1000', 'strong')]),
				h('div', { class: 'rule-line top' }, [px('Points'), table])
			];
			stage.append(field, h('div', { class: 'rule-card' }, rows));
			for (const el of [...rows, ...cells]) show(el, false);
			await t.say('This is a world: a game for two players, with fixed rules.');
			await t.hold();
			await t.say('The rules say how many play, which moves are allowed, and for how many rounds.');
			for (const row of rows.slice(0, 3)) {
				show(row);
				t.sfx('step');
				await t.wait(700);
			}
			await t.hold();
			await t.say('They also say how many points each pair of moves pays.');
			show(rows[3]);
			for (const el of cells) {
				show(el);
				t.sfx('step');
				await t.wait(450);
			}
			await t.hold();
		}
	},
	{
		title: 'A strategy',
		text: 'A strategy is a plan for how to play. Each round it looks at what has happened so far and picks a move.',
		async play(stage, t) {
			const theirs = ['C', 'D', 'D', 'C', 'D', 'C'];
			const rules = tftRules();
			const them = theirs.map(() => chip());
			const me = theirs.map(() => chip());
			const io = h('div', { class: 'io' }, [px('They'), h('span', { class: 'io-row' }, them), px('Me'), h('span', { class: 'io-row' }, me)]);
			stage.append(h('div', { class: 'strategy-head' }, [avatar('tit_for_tat', { size: 56 }), h('div', { class: 'rule-card' }, rules)]), io);
			const light = (index) => rules.forEach((rule, i) => rule.classList.toggle('is-on', i === index));
			for (const el of [...rules, io]) show(el, false);
			await t.say('A strategy is a plan for how to play. This one is called Tit for Tat.');
			await t.hold();
			await t.say('Its whole plan is three rules.');
			for (const rule of rules) {
				show(rule);
				t.sfx('step');
				await t.wait(600);
			}
			await t.hold();
			show(io);
			await t.say('Its first move is always to cooperate.');
			light(0);
			await t.wait(500);
			setChip(me[0], 'C');
			await t.wait(500);
			setChip(them[0], theirs[0]);
			await t.hold();
			await t.say('After that, it plays whatever the other player played last.');
			for (let i = 1; i < theirs.length; i++) {
				light(theirs[i - 1] === 'C' ? 1 : 2);
				await t.wait(550);
				setChip(me[i], theirs[i - 1]);
				await t.wait(400);
				setChip(them[i], theirs[i]);
				await t.wait(700);
			}
			light(-1);
			await t.hold();
		}
	},
	{
		title: 'A match',
		text: 'Two strategies meet and play round by round. Both move at the same time, and every round pays points.',
		async play(stage, t) {
			const [a, b] = ['tit_for_tat', 'always_defect'];
			const match = playMatch(CLASSICS[a], CLASSICS[b], ROUNDS);
			const court = duel();
			const strip = miniStrip();
			stage.append(court.node, strip.node);
			court.left.set(a, CLASSICS[a].name);
			court.right.set(b, CLASSICS[b].name);
			await t.say('Two strategies meet in a match: Tit for Tat against Always Defect.');
			await t.hold();
			await t.say('Each round, both pick a move at the same time.');
			await playRound(t, court, strip, match, 0, { flight: 500, rest: 900 });
			await t.say('Tit for Tat cooperated and got 0. Always Defect defected and took 5.');
			await t.hold();
			await t.say('From now on Tit for Tat defects back. Each of these rounds pays 1 point each.');
			for (let i = 1; i < ROUNDS; i++) await playRound(t, court, strip, match, i);
			await t.hold();
			await t.say('After the last round the higher score wins: Always Defect, 14 to 9.');
			court.right.name.classList.add('is-winner');
			await t.hold();
		}
	},
	{
		title: 'The ranking',
		text: 'Every strategy plays every other strategy. The points add up, and the best total tops the ranking.',
		async play(stage, t) {
			const ids = ['always_cooperate', 'always_defect', 'tit_for_tat', 'win_stay_lose_shift'];
			const pairs = ids.flatMap((a, i) => ids.slice(i + 1).map((b) => [a, b]));
			const max = Math.max(...Object.values(playAll(ids.map((id) => ({ id, rule: CLASSICS[id] })), ROUNDS)));
			const rows = ids.map((id) => boardRow(id, { hollow: id === 'always_defect' }));
			const byId = Object.fromEntries(rows.map((row) => [row.id, row]));
			const totals = Object.fromEntries(ids.map((id) => [id, 0]));
			const list = h('div', { class: 'board' }, rows.map((row) => row.node));
			const strip = miniStrip();
			const status = px('', 'scene-status');
			stage.append(status, strip.node, list);
			for (const row of rows) row.set(0, max);
			await t.say('One match is not enough. Every strategy plays every other strategy.');
			await t.hold();
			await t.say('Four strategies make six matches. The points from every match add up.');
			for (const [n, [a, b]] of pairs.entries()) {
				const match = playMatch(CLASSICS[a], CLASSICS[b], ROUNDS);
				strip.clear();
				status.textContent = `Match ${n + 1} / ${pairs.length}`;
				byId[a].node.classList.add('is-playing');
				byId[b].node.classList.add('is-playing');
				await t.wait(300);
				for (let i = 0; i < ROUNDS; i++) {
					strip.set(i, match.a[i], match.b[i]);
					await t.wait(60);
				}
				totals[a] += match.scoreA.at(-1);
				totals[b] += match.scoreB.at(-1);
				byId[a].set(totals[a], max);
				byId[b].set(totals[b], max);
				await t.wait(550);
				byId[a].node.classList.remove('is-playing');
				byId[b].node.classList.remove('is-playing');
			}
			await t.say('The best total tops the ranking. In this small group, that is Always Defect.');
			status.textContent = 'Final ranking';
			rankRows(list, rows, totals, max);
			await t.hold();
		}
	}
];

// --- film 2: how it works ---

const SENTENCE = 'Cooperate first. Then copy my opponent’s last move.';
const FIELD = ['always_defect', 'tit_for_tat', 'win_stay_lose_shift']; // who your strategy meets in the pictures
const totalsWith = (rule) => playAll([...FIELD.map((id) => ({ id, rule: CLASSICS[id] })), { id: YOU, rule }], ROUNDS);

const HOW_IT_WORKS = [
	{
		title: 'Pick a world',
		text: 'Start with the Prisoner’s Dilemma. Read the rules and see who is on top. More worlds will follow.',
		async play(stage, t) {
			const tile = (glyph, caption, soon) => h('div', { class: soon ? 'world-tile is-soon' : 'world-tile' }, [h('div', { class: 'world-glyph' }, glyph), px(caption)]);
			const tiles = [
				tile(h('div', { class: 'mini-field' }, [chip('C', 'big'), h('span', { class: 'mini-line' }), chip('D', 'big')]), 'World 1', false),
				tile(h('div', { class: 'glyph-three' }, [chip('D'), chip('D'), chip('D')]), 'Soon', true),
				tile(h('div', { class: 'glyph-crowd' }, Array.from({ length: 9 }, () => h('span', { class: 'dot' }))), 'Soon', true)
			];
			const status = px('', 'scene-status');
			stage.append(h('div', { class: 'world-tiles' }, tiles), status);
			const point = (index) => tiles.forEach((el, i) => el.classList.toggle('has-cursor', i === index));
			await t.say('First, pick a world. Each world is a different game.');
			for (const index of [2, 1, 0]) {
				point(index);
				t.sfx('step');
				await t.wait(800);
			}
			await t.say('Start with the Prisoner’s Dilemma. More worlds are on the way.');
			tiles[0].classList.add('is-picked');
			status.textContent = 'Prisoner’s Dilemma';
			await t.hold();
		}
	},
	{
		title: 'Write a strategy',
		text: 'Describe it in plain English. Lilanaya turns it into rules you can read and check, and gives it an avatar. You can also write it in JavaScript.',
		async play(stage, t) {
			const typed = h('span');
			const caret = h('span', { class: 'caret' });
			const arrow = downArrow();
			const rules = tftRules();
			const face = avatar(YOU, { size: 56, build: true });
			stage.append(h('div', { class: 'type-box' }, [typed, caret]), arrow, h('div', { class: 'strategy-head' }, [face, h('div', { class: 'rule-card' }, rules)]));
			for (const el of [arrow, ...rules]) show(el, false);
			await t.say('Describe how you want to play, in plain English.');
			for (let i = 1; i <= SENTENCE.length; i++) {
				typed.textContent = SENTENCE.slice(0, i);
				await t.wait(40);
			}
			show(caret, false);
			await t.wait(600);
			await t.say('Lilanaya turns your words into rules you can read and check.');
			show(arrow);
			await t.wait(400);
			for (const rule of rules) {
				show(rule);
				t.sfx('step');
				await t.wait(600);
			}
			await t.hold();
			await t.say('Your strategy also gets its own avatar.');
			const blocks = revealAvatar(face, 0);
			for (let i = 1; i <= blocks; i++) {
				revealAvatar(face, i);
				await t.wait(35);
			}
			await t.hold();
		}
	},
	{
		title: 'Run it',
		text: 'Your strategy plays the built-in classics and other players’ strategies. Every match is played out round by round.',
		async play(stage, t) {
			const rows = FIELD.map((id) => {
				const strip = miniStrip();
				const score = px('', 'run-score');
				return { id, strip, score, node: h('div', { class: 'run-row' }, [avatar(id, { size: 24 }), strip.node, score]) };
			});
			const status = px('', 'scene-status');
			stage.append(h('div', { class: 'run-you' }, [avatar(YOU, { size: 40 }), px('You'), px('top row', 'dim')]), h('div', { class: 'run-rows' }, rows.map((row) => row.node)), status);
			await t.say('Now run it. Your strategy plays every other strategy in the world.');
			await t.hold();
			await t.say('Each match is played out round by round. Your moves are the top row.');
			let total = 0;
			for (const [n, row] of rows.entries()) {
				const match = playMatch(YOU_V2, CLASSICS[row.id], ROUNDS);
				row.node.classList.add('is-playing');
				status.textContent = `Match ${n + 1} / ${rows.length}`;
				for (let i = 0; i < ROUNDS; i++) {
					row.strip.set(i, match.a[i], match.b[i]);
					row.score.textContent = `${match.scoreA[i]}-${match.scoreB[i]}`;
					await t.wait(120);
				}
				total += match.scoreA.at(-1);
				await t.wait(600);
				row.node.classList.remove('is-playing');
			}
			await t.say(`Done. Three matches, ${total} points, and every move is there to look at.`);
			status.textContent = `Done · ${total} points`;
			await t.hold();
		}
	},
	{
		title: 'Learn and improve',
		text: 'See your rank, your score against each opponent, and every move of every match. Change your strategy and run it again.',
		async play(stage, t) {
			const [v1, v2] = [totalsWith(YOU_V1), totalsWith(YOU_V2)];
			const max = Math.max(...Object.values(v1), ...Object.values(v2));
			const rows = [...FIELD.map((id) => boardRow(id, { hollow: id === 'always_defect' })), boardRow(YOU, { you: true })];
			const list = h('div', { class: 'board' }, rows.map((row) => row.node));
			const answer = chip('C');
			const rule = h('div', { class: 'rule-line' }, [px('They play'), chip('D'), px('>'), answer]);
			const status = px('Version 1', 'scene-status');
			stage.append(status, h('div', { class: 'rule-card' }, rule), list);
			for (const row of rows) row.set(0, max);
			await t.say('The ranking shows where you stand. Say your first version always cooperates.');
			for (const row of rows) row.set(v1[row.id], max);
			await t.wait(800);
			rankRows(list, rows, v1, max);
			await t.hold();
			await t.say('It ends last: Always Defect took 5 points from it in every round.');
			await t.hold();
			await t.say('So change one rule: when they defect, defect back.');
			status.textContent = 'Change one rule';
			rule.classList.add('is-on');
			await t.wait(900);
			setChip(answer, 'D');
			await t.hold();
			rule.classList.remove('is-on');
			await t.say('Run it again. Version 2 shares first place.');
			status.textContent = 'Version 2';
			for (const row of rows) row.set(v2[row.id], max, '');
			await t.wait(800);
			rankRows(list, rows, v2, max);
			await t.hold();
		}
	}
];

// --- film 3: the Prisoner's Dilemma ---

/** `cells` are the cells of the points table on the page; the film lights up the one it is talking about. */
function dilemmaChapters(cells) {
	const mark = (key) => {
		for (const [k, cell] of Object.entries(cells)) cell.classList.toggle('is-on', k === key);
	};
	const table = () => {
		const court = duel({ card: true, face: 48 });
		court.left.set(YOU, 'You');
		court.right.set(RIVAL, 'They');
		return court;
	};
	const hide = ({ left, right }) => {
		for (const side of [left, right]) {
			setChip(side.shot, '');
			show(side.mark);
			side.score.textContent = '';
		}
		mark('');
	};
	const reveal = ({ left, right }, a, b) => {
		setChip(left.shot, a);
		setChip(right.shot, b);
		show(left.mark, false);
		show(right.mark, false);
	};
	const pay = ({ left, right }, a, b) => {
		left.score.textContent = `+${PAYOFF[a + b][0]}`;
		right.score.textContent = `+${PAYOFF[a + b][1]}`;
		mark(a + b);
	};
	return [
		{
			title: 'One round',
			text: 'Two players. Each round, both choose to cooperate or defect, without knowing what the other will do.',
			async play(stage, t) {
				const court = table();
				stage.append(court.node);
				hide(court);
				await t.say('Two players. Each picks a move in secret: cooperate or defect.');
				await t.hold();
				await t.say('Then both moves are shown at the same time.');
				reveal(court, 'C', 'C');
				await t.hold();
				await t.say('Both cooperated, so both get 3 points.');
				pay(court, 'C', 'C');
				await t.hold();
			}
		},
		{
			title: 'The four outcomes',
			text: 'Both cooperate: 3 points each. One defects: the defector gets 5, the other gets 0. Both defect: 1 point each.',
			async play(stage, t) {
				const court = table();
				stage.append(court.node);
				const outcomes = [
					['D', 'C', 'You defect while they cooperate: 5 points for you, 0 for them.'],
					['C', 'D', 'The other way round, you are the one left with 0.'],
					['D', 'D', 'And if both defect, both get just 1 point.']
				];
				hide(court);
				await t.say('There are three other ways a round can end.');
				await t.hold();
				for (const [a, b, text] of outcomes) {
					hide(court);
					await t.wait(700);
					reveal(court, a, b);
					await t.say(text);
					pay(court, a, b);
					await t.hold();
				}
			}
		},
		{
			title: 'The dilemma',
			text: 'Whatever the other player does, you score more this round if you defect. But if both think that way, both get 1, when both could have had 3.',
			async play(stage, t) {
				const cell = Object.fromEntries(Object.keys(PAYOFF).map((key) => [key, px(String(PAYOFF[key][0]), 'pay-cell wide')]));
				const head = (who, move) => h('span', { class: 'rule-line' }, [px(who), chip(move)]);
				stage.append(
					px('Your points', 'scene-status'),
					h('div', { class: 'pay-grid' }, [h('span'), head('They', 'C'), head('They', 'D'), head('You', 'C'), cell.CC, cell.CD, head('You', 'D'), cell.DC, cell.DD])
				);
				const light = (...keys) => {
					for (const [key, el] of Object.entries(cell)) el.classList.toggle('is-on', keys.includes(key));
				};
				mark('');
				await t.say('Look at your own points. Which move is better for you?');
				await t.hold();
				await t.say('If they cooperate, defecting pays you 5 instead of 3.');
				light('DC');
				await t.hold();
				await t.say('If they defect, defecting pays you 1 instead of 0.');
				light('DD');
				await t.hold();
				await t.say('So defecting always looks better. But the other player sees the same.');
				light('DC', 'DD');
				await t.hold();
				await t.say('Both defect and both get 1, when both could have had 3. That is the dilemma.');
				light('DD', 'CC');
				await t.hold();
			}
		},
		{
			title: 'It repeats',
			text: 'In Lilanaya the game repeats for many rounds, so the other player can answer what you did. Do you trust, punish, or forgive?',
			async play(stage, t) {
				// You defect once, in round 3; Tit for Tat answers in round 4.
				const match = { a: [...'CCDCCCCC'], b: [...'CCCDCCCC'], scoreA: [], scoreB: [] };
				let [sa, sb] = [0, 0];
				match.a.forEach((move, i) => {
					const [pa, pb] = PAYOFF[move + match.b[i]];
					match.scoreA.push((sa += pa));
					match.scoreB.push((sb += pb));
				});
				const court = duel({ face: 48 });
				const strip = miniStrip(match.a.length);
				stage.append(court.node, strip.node);
				court.left.set(YOU, 'You');
				court.right.set('tit_for_tat', 'Tit for Tat');
				const slow = { flight: 450, rest: 700 };
				mark('');
				await t.say('In Lilanaya the game repeats for many rounds. Here you play Tit for Tat.');
				await playRound(t, court, strip, match, 0, slow);
				await playRound(t, court, strip, match, 1, slow);
				await t.hold();
				await t.say('Round 3: you defect. 5 points for you, 0 for Tit for Tat.');
				await playRound(t, court, strip, match, 2, slow);
				await t.hold();
				await t.say('Round 4: Tit for Tat remembers and defects back. This time you get 0.');
				await playRound(t, court, strip, match, 3, slow);
				await t.hold();
				await t.say('Then both go back to cooperating.');
				for (let i = 4; i < match.a.length; i++) await playRound(t, court, strip, match, i);
				await t.hold();
				await t.say(`You end on ${sa} points. Cooperating every round would have paid ${match.a.length * PAYOFF.CC[0]}.`);
				await t.hold();
				await t.say('Trust, punish or forgive: that is what your strategy has to decide.');
				await t.hold();
			}
		}
	];
}

// --- the Prisoner's Dilemma section ---

/** The payoff table as real, readable HTML; the one-round scene lights up the cell it is showing. */
function payoffTable() {
	const cells = {};
	const cell = (key) => (cells[key] = h('td', null, `${PAYOFF[key][0]}, ${PAYOFF[key][1]}`));
	const node = h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Points for each outcome' }, [
		h('table', { class: 'grid-table pay-table' }, [
			h('caption', { class: 'meta' }, 'Points per round: you, they'),
			h('thead', null, h('tr', null, [h('td', null), h('th', { scope: 'col' }, 'They cooperate'), h('th', { scope: 'col' }, 'They defect')])),
			h('tbody', null, [
				h('tr', null, [h('th', { scope: 'row' }, 'You cooperate'), cell('CC'), cell('CD')]),
				h('tr', null, [h('th', { scope: 'row' }, 'You defect'), cell('DC'), cell('DD')])
			])
		])
	]);
	return { node, cells };
}

// --- the page ---

export async function howItWorksPage({ onLeave, query }) {
	const user = await getUser();
	const table = payoffTable();
	const films = [
		film({ label: 'Film: what Lilanaya is, in four chapters', chapters: WHAT_IT_IS }),
		film({ label: 'Film: how Lilanaya works, in four steps', chapters: HOW_IT_WORKS }),
		film({
			label: 'Film: the Prisoner’s Dilemma, in four chapters',
			chapters: dilemmaChapters(table.cells),
			aside: h('div', { class: 'stack' }, [h('h3', { class: 'section-heading' }, 'The points'), table.node])
		})
	];
	onLeave(() => {
		for (const f of films) f.destroy();
	});
	const start = user ? '/strategies/new' : signInUrl('/strategies/new');

	// For whoever records the voice: /how-it-works?voice-script lists every caption with the file name its clip must have.
	const script = query.has('voice-script') ? h('pre', { class: 'code', id: 'voice-script' }) : null;
	if (script) setTimeout(() => (script.textContent = JSON.stringify([...filmCaptions].map(([clip, text]) => ({ clip, text })), null, 1)), 500);

	return {
		title: 'How it works',
		node: h('div', { class: 'landing' }, [
			// Every section is one screen: it fills the window and fits inside it (see `.how-screen`).
			h('section', { id: 'what-it-is', class: 'how-screen how-top stack-md' }, [
				h('div', { class: 'stack' }, [
					sectionHeading('How it works', { level: 1 }),
					h('p', null, 'Three short films. Each one starts when you scroll to it. First: what Lilanaya is.')
				]),
				films[0].node,
				legend()
			]),

			h('section', { id: 'what-you-do', class: 'landing-section how-screen stack-md' }, [sectionHeading('What you do'), films[1].node, legend()]),

			h('section', { id: 'prisoners-dilemma', class: 'landing-section how-screen stack-md' }, [sectionHeading('The first world: Prisoner’s Dilemma'), films[2].node, legend()]),

			h('section', { id: 'why', class: 'landing-section how-screen stack-lg' }, [
				sectionHeading('Why Lilanaya'),
				h('ul', { class: 'figures two reasons' }, [
					h('li', { class: 'stack-sm' }, [h('strong', null, 'No coding needed.'), 'Plain English is enough.']),
					h('li', { class: 'stack-sm' }, [h('strong', null, 'Nothing is hidden.'), 'Every result can be traced back to the moves that caused it.']),
					h('li', { class: 'stack-sm' }, [h('strong', null, 'Fair and repeatable.'), 'The same run always gives the same result.']),
					h('li', { class: 'stack-sm' }, [h('strong', null, 'Known opponents.'), 'Test yourself against classics like Tit for Tat and Always Defect.'])
				]),
				h('div', { class: 'landing-close stack-md' }, [
				h('h2', { class: 'section-heading' }, 'Think your strategy can win?'),
				h('p', null, user ? 'Every strategy you make gets its own avatar.' : 'Sign in with your email. No password needed. Every strategy you make gets its own avatar.'),
				h('p', { class: 'row' }, [linkButton('Write your first strategy', start, { primary: true }), linkButton('Back to the start', '/')])
				])
			]),
			script
		])
	};
}
