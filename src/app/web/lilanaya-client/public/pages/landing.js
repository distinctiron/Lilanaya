import { avatar, avatarName } from '../components/avatar.js';
import { button, linkButton } from '../components/button.js';
import { CLASSICS, PAYOFF, playAll, playMatch } from '../components/pocket-dilemma.js';
import { roundStrip, stripLegend } from '../components/round-strip.js';
import { h, prefersReducedMotion } from '../dom.js';
import { getUser, signUpUrl } from '../session.js';

// The front page teaches by doing, one idea per step:
//   1. the visitor plays five rounds by hand against an opponent that talks back (the rules);
//   2. the opponent turns out to be Tit for Tat (what a strategy is);
//   3. strategies play each other (matches and the ranking);
//   4. three steps and one action (what to do next).
// The films and the deep dive live on /how-it-works.

// What each classic says about itself.
const MOTTO = {
	tit_for_tat: 'I do what you did.',
	always_cooperate: 'I trust everyone.',
	always_defect: 'Nothing personal.',
	win_stay_lose_shift: 'If it worked, I do it again.'
};

/** Moves the reader to the screen that `heading` belongs to: scrolls it into view and puts keyboard focus on the heading. */
function goTo(heading) {
	const section = heading.closest('section');
	if (section.id) history.replaceState(null, '', `#${section.id}`); // so the address can be shared
	section.scrollIntoView({ behavior: prefersReducedMotion() ? 'auto' : 'smooth', block: 'start' });
	heading.focus({ preventScroll: true });
}

const moveLegend = (...more) =>
	h('div', { class: 'legend meta' }, [
		h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-solid', 'aria-hidden': 'true' }), 'Cooperate']),
		h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-hollow', 'aria-hidden': 'true' }), 'Defect']),
		...more
	]);

// --- step 1 and 2: play by hand, then meet the strategy ---

const HAND_ROUNDS = 5;
const OPPONENT = 'tit_for_tat';

// What the opponent says after a round, by outcome (the visitor's move, then its own).
const REACTIONS = {
	CC: ['Nice. 3 points each. Same again?', 'We make a good team. Again?', 'Still friends. 3 each.', 'I could do this all day.'],
	DC: ['Ouch. You took 5 and left me with nothing. I will remember that.', 'Again? You take 5, I get 0. I will not forget.'],
	CD: ['That was for last round. Now we are even. Friends again?', 'You know why I did that. Shall we start over?'],
	DD: ['See? 1 point each. Nobody wins like this. Want to make peace?', 'Still 1 each. One of us has to be nice first.']
};
const HELLO = 'Hi. Five rounds, you and me. Each round we both pick at the same time. So: will you cooperate, or defect?';
const LAST_ROUND = ' Last round. After this one I cannot pay you back.';

/** The five-round game against a talking opponent. `onDone` runs when the visitor asks to move on. Returns the node. */
function playByHand(onDone) {
	const rule = CLASSICS[OPPONENT];
	const best = HAND_ROUNDS * PAYOFF.CC[0];
	let mine = []; // the visitor's moves so far
	let theirs = [];
	const said = {}; // outcome → how often it has come up, to vary the reactions

	const face = h('div', { class: 'talk-face' }, avatar(OPPONENT, { size: 80 }));
	const name = h('span', { class: 'px' });
	const bubble = h('p', { class: 'bubble', 'aria-live': 'polite' });
	const squares = [0, 1].map(() => Array.from({ length: HAND_ROUNDS }, () => h('span', { class: 'chip big say' })));
	const gains = [0, 1].map(() => Array.from({ length: HAND_ROUNDS }, () => h('span', { class: 'hand-gain' })));
	const scores = [h('span', { class: 'hand-score' }), h('span', { class: 'hand-score' })];
	const leads = [h('span', { class: 'px' }), h('span', { class: 'px' })]; // says who is ahead on the total
	const theirLabel = h('span');
	const youTag = () => h('span', { class: 'you-tag' }, 'You');
	const moveButton = (label, m) => {
		const el = button('', { class: 'btn btn-big btn-move', onclick: () => move(m) });
		el.replaceChildren(h('span', { class: 'chip say', 'data-move': m, 'aria-hidden': 'true' }), label);
		return el;
	};
	const lane = (side, who) =>
		h('div', { class: side === 0 ? 'hand-lane is-you' : 'hand-lane' }, [
			h('span', { class: 'hand-who px' }, who),
			h('span', { class: 'hand-row' }, squares[side].map((square, i) => h('span', { class: 'hand-cell' }, [square, gains[side][i]]))),
			h('span', { class: 'hand-total' }, [scores[side], leads[side]])
		]);
	const result = h('p', { class: 'hand-result' });
	const prompt = h('p', { class: 'px' });
	const actions = h('div', { class: 'row' });
	const note = h('p', { class: 'meta' });
	const key = moveLegend(h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-win', 'aria-hidden': 'true' }), 'Won the round']));

	const points = h('dl', { class: 'hand-points meta' }, [
		h('dt', null, 'Both cooperate'),
		h('dd', null, `${PAYOFF.CC[0]} points each`),
		h('dt', null, 'One defects'),
		h('dd', null, `${PAYOFF.DC[0]} for the defector, ${PAYOFF.DC[1]} for the other`),
		h('dt', null, 'Both defect'),
		h('dd', null, `${PAYOFF.DD[0]} point each`)
	]);

	const total = (side) => mine.reduce((sum, move, i) => sum + PAYOFF[move + theirs[i]][side], 0);
	const word = (move) => (move === 'C' ? 'cooperated' : 'defected');

	function paint() {
		const round = mine.length;
		const over = round === HAND_ROUNDS;
		squares.forEach((row, side) =>
			row.forEach((el, i) => {
				el.setAttribute('data-move', (side === 0 ? mine : theirs)[i] ?? '');
				el.classList.toggle('is-next', i === round);
				// Points won in that round; whoever got more is raised.
				const pay = i < round ? PAYOFF[mine[i] + theirs[i]] : null;
				const won = pay != null && pay[side] > pay[1 - side];
				el.classList.toggle('is-winner', won);
				gains[side][i].textContent = pay ? `+${pay[side]}` : '';
				gains[side][i].className = won ? 'hand-gain is-winner' : pay && pay[side] === pay[1 - side] ? 'hand-gain is-draw' : 'hand-gain';
			})
		);
		const totals = [total(0), total(1)];
		scores.forEach((el, side) => {
			const ahead = totals[side] > totals[1 - side];
			el.textContent = String(totals[side]);
			el.classList.toggle('is-leading', ahead);
			leads[side].textContent = ahead ? (over ? 'Wins' : 'Leads') : '';
		});
		name.textContent = theirLabel.textContent = over ? rule.name : '???';

		if (round) {
			const [a, b] = [mine[round - 1], theirs[round - 1]];
			const [pa, pb] = PAYOFF[a + b];
			const verdict = pa === pb ? 'A draw' : pa > pb ? 'You won the round' : 'It won the round';
			result.replaceChildren(h('span', { class: 'px' }, `Round ${round}: ${verdict}`), h('br'), `You ${word(a)}, it ${word(b)}.`, h('br'), `You +${pa} · It +${pb}`);
		}
		result.hidden = !round;

		if (!over) {
			prompt.replaceChildren(youTag(), ` Round ${round + 1} of ${HAND_ROUNDS}. Your move:`);
			actions.replaceChildren(moveButton('Cooperate', 'C'), moveButton('Defect', 'D'));
			note.textContent = '';
			points.hidden = false;
			return;
		}
		prompt.textContent = 'In Lilanaya you do not play by hand. You write the plan, and it plays for you.';
		const next = button('See strategies play', { primary: true, class: 'btn btn-primary btn-big', onclick: onDone });
		actions.replaceChildren(next, button('Play again', { class: 'btn btn-big', onclick: reset }));
		note.textContent = 'Its row is your row, one round late.';
		points.hidden = true;
		next.focus();
	}

	function speak(text) {
		bubble.textContent = text;
		if (prefersReducedMotion()) return;
		face.classList.remove('is-talking');
		void face.offsetWidth; // lets the hop start again
		face.classList.add('is-talking');
	}

	function move(my) {
		const round = mine.length;
		const their = round ? rule.next(theirs[round - 1], mine[round - 1]) : rule.first;
		mine.push(my);
		theirs.push(their);
		const outcome = my + their;
		if (mine.length < HAND_ROUNDS) {
			const lines = REACTIONS[outcome];
			const line = lines[Math.min(said[outcome] ?? 0, lines.length - 1)];
			said[outcome] = (said[outcome] ?? 0) + 1;
			speak(line + (mine.length === HAND_ROUNDS - 1 ? LAST_ROUND : ''));
			paint();
			// The buttons were drawn again; keep the keyboard on the move just made.
			actions.children[my === 'C' ? 0 : 1].focus();
			return;
		}
		const [a, b] = [total(0), total(1)];
		const score = a === b ? `${a} each.` : `${a} to you, ${b} to me.`;
		const lesson =
			a === best && b === best
				? `We both cooperated every round: ${best} each, the best two players can do together.`
				: a > best
					? `You beat ${best} by defecting at the very end, when I could not answer. Real matches are much longer.`
					: `Had we both cooperated every round, we would have ${best} each.`;
		speak(`${score} I am ${rule.name}. My whole plan: start nice, then copy your last move. That plan is my strategy. ${lesson}`);
		paint();
	}

	function reset() {
		mine = [];
		theirs = [];
		for (const key of Object.keys(said)) delete said[key];
		speak(HELLO);
		paint();
		actions.children[0].focus();
	}

	bubble.textContent = HELLO;
	paint();

	return h('div', { class: 'hand-game' }, [
		h('div', { class: 'talk' }, [h('div', { class: 'talk-who' }, [face, name]), bubble]),
		h('div', { class: 'hand-table' }, [
			h('div', { class: 'stack' }, [
				h('div', { class: 'hand-track', role: 'group', 'aria-label': 'Moves and points so far' }, [
					h('div', { class: 'hand-heads px', 'aria-hidden': 'true' }, [
						h('span', null, 'Round'),
						h('span', { class: 'hand-row' }, squares[0].map((_, i) => h('span', { class: 'hand-cell' }, String(i + 1)))),
						h('span', null, 'Total')
					]),
					lane(0, youTag()),
					lane(1, [avatar(OPPONENT, { size: 24 }), theirLabel])
				]),
				key,
				result,
				note
			]),
			h('div', { class: 'stack hand-yours' }, [prompt, actions, points])
		])
	]);
}

// --- step 3: strategies play each other ---

/** Pick any two classics and watch twenty rounds. Returns { node, destroy }. */
function classicsLab(onNext) {
	const LAB_ROUNDS = 20;
	const ROUND_MS = 130;
	const ids = Object.keys(CLASSICS);
	const picked = { a: 'tit_for_tat', b: 'always_defect' };
	let timer = 0;

	const scoreA = h('div', { class: 'match-score' });
	const scoreB = h('div', { class: 'match-score' });
	const sideA = h('div', { class: 'lab-who' });
	const sideB = h('div', { class: 'lab-who' });
	const stripBox = h('div');
	const result = h('p', { 'aria-live': 'polite' });

	function play(animate) {
		clearTimeout(timer);
		const [a, b] = [CLASSICS[picked.a], CLASSICS[picked.b]];
		const match = playMatch(a, b, LAB_ROUNDS);
		const index = (move) => (move === 'C' ? 0 : 1);
		const strip = roundStrip({
			movesA: match.a.map(index),
			movesB: match.b.map(index),
			label: `${LAB_ROUNDS} rounds. ${a.name} played ${match.a.join(' ')}. ${b.name} played ${match.b.join(' ')}.`
		});
		sideA.replaceChildren(avatar(picked.a, { size: 48 }), h('div', { class: 'match-name' }, a.name));
		sideB.replaceChildren(avatar(picked.b, { size: 48 }), h('div', { class: 'match-name' }, b.name));
		stripBox.replaceChildren(strip.node, stripLegend({ top: a.name, bottom: b.name, solid: 'Cooperate', hollow: 'Defect' }));
		const [sa, sb] = [match.scoreA.at(-1), match.scoreB.at(-1)];
		const crown = (won) => {
			scoreA.classList.toggle('is-winner', won && sa > sb);
			scoreB.classList.toggle('is-winner', won && sb > sa);
		};
		const verdict = `After ${LAB_ROUNDS} rounds: ${a.name} ${sa}, ${b.name} ${sb}. ${sa === sb ? 'A draw.' : `${(sa > sb ? a : b).name} wins.`}`;
		const frame = (k) => {
			strip.show(k);
			scoreA.textContent = String(k ? match.scoreA[k - 1] : 0);
			scoreB.textContent = String(k ? match.scoreB[k - 1] : 0);
		};
		if (!animate || prefersReducedMotion()) {
			frame(LAB_ROUNDS);
			result.textContent = verdict;
			crown(true);
			return;
		}
		result.textContent = '';
		crown(false);
		let k = 0;
		const tick = () => {
			frame(k);
			if (k++ < LAB_ROUNDS) timer = setTimeout(tick, ROUND_MS);
			else {
				result.textContent = verdict;
				crown(true);
			}
		};
		tick();
	}

	const picker = (side, title) =>
		h('fieldset', { class: 'stack-sm' }, [
			h('legend', { class: 'field-label' }, title),
			h('div', { class: 'lab-picks' }, ids.map((id) =>
				h('label', { class: 'lab-pick' }, [
					h('input', {
						type: 'radio',
						name: `classic-${side}`,
						class: 'visually-hidden',
						value: id,
						checked: picked[side] === id,
						onchange: () => {
							picked[side] = id;
							play(true);
						}
					}),
					avatar(id, { size: 32 }),
					h('span', { class: 'stack-xs' }, [h('span', null, CLASSICS[id].name), h('span', { class: 'meta' }, `“${MOTTO[id]}”`)])
				])
			))
		]);

	const node = h('div', { class: 'stack-md lab' }, [
		h('div', { class: 'columns' }, [picker('a', 'Left player'), picker('b', 'Right player')]),
		h('div', { class: 'match' }, [
			h('div', { class: 'match-side match-left' }, [sideA, scoreA]),
			h('div', { class: 'centre-line', 'aria-hidden': 'true' }),
			h('div', { class: 'match-side match-right' }, [sideB, scoreB])
		]),
		stripBox,
		result,
		h('div', { class: 'row' }, [button('Play again', { onclick: () => play(true) }), button('See the ranking', { onclick: onNext })])
	]);

	// Show the finished match at once; play it through the first time it scrolls into view.
	play(false);
	let observer = null;
	if ('IntersectionObserver' in window && !prefersReducedMotion()) {
		observer = new IntersectionObserver((entries) => {
			if (!entries.some((entry) => entry.isIntersecting)) return;
			observer.disconnect();
			play(true);
		}, { threshold: 0.4 });
		observer.observe(stripBox);
	}
	return {
		node,
		destroy() {
			clearTimeout(timer);
			observer?.disconnect();
		}
	};
}

/** The four classics, everyone against everyone, ranked by average points per round. */
function classicsRanking() {
	const RANK_ROUNDS = 1000;
	const ids = Object.keys(CLASSICS);
	const totals = playAll(ids.map((id) => ({ id, rule: CLASSICS[id] })), RANK_ROUNDS);
	const average = (id) => totals[id] / ((ids.length - 1) * RANK_ROUNDS);
	const sorted = [...ids].sort((a, b) => totals[b] - totals[a]);
	const top = average(sorted[0]);
	return {
		winner: CLASSICS[sorted[0]].name,
		node: h('div', { class: 'stack' }, [
			h('ol', { class: 'rank-list' }, sorted.map((id, i) =>
				h('li', { class: 'rank-row' }, [
					h('span', { class: 'px', 'aria-hidden': 'true' }, String(i + 1)),
					avatarName(id, CLASSICS[id].name),
					h('span', { class: 'rank-track', 'aria-hidden': 'true' }, h('span', { class: 'bar', style: { width: `${((average(id) / top) * 100).toFixed(1)}%` } })),
					h('span', { class: 'rank-score' }, average(id).toFixed(2))
				])
			)),
			h('p', { class: 'meta' }, `Average points per round · Everyone against everyone · ${RANK_ROUNDS} rounds per match`)
		])
	};
}

// --- step 4: what to do next ---

function steps() {
	const strip = (moves) => h('span', { class: 'step-strip' }, [...moves].map((move) => h('span', { class: 'chip', 'data-move': move })));
	const step = (number, picture, title, text) => h('li', { class: 'step stack-sm' }, [h('div', { class: 'step-pic', 'aria-hidden': 'true' }, picture), h('h3', { class: 'px' }, `${number} ${title}`), h('p', null, text)]);
	return h('ol', { class: 'steps' }, [
		step(1, h('span', { class: 'px' }, 'Prisoner’s Dilemma'), 'Pick a world', 'A world is a game with fixed rules. The first is the Prisoner’s Dilemma.'),
		step(2, avatar('challenger', { size: 48 }), 'Write a strategy', 'In plain English, as rules, or as code. Each one gets its own avatar.'),
		step(3, strip('CCDCCCDC'), 'Run and improve', 'Run it against other strategies. See every round, change it, and run again.')
	]);
}

// --- the page ---

export async function landingPage({ onLeave }) {
	const user = await getUser();
	const lab = classicsLab(() => goTo(rankHeading));
	const ranking = classicsRanking();
	onLeave(() => lab.destroy());

	const start = user ? '/strategies/new' : signUpUrl('/strategies/new');
	const playHeading = h('h2', { class: 'section-heading', tabindex: '-1' }, 'Play five rounds');
	const arenaHeading = h('h2', { class: 'section-heading', tabindex: '-1' }, 'Now strategies play each other');
	const rankHeading = h('h2', { class: 'section-heading', tabindex: '-1' }, 'The ranking');
	const turnHeading = h('h2', { class: 'section-heading', tabindex: '-1' }, 'Can you write one that beats them?');

	return {
		title: 'A game-theory arena',
		node: h('div', { class: 'landing' }, [
			h('section', { class: 'landing-hero' }, [
				avatar(OPPONENT, { size: 64 }),
				h('h1', { class: 'hero-title', tabindex: '-1' }, ['Write a strategy.', h('br'), 'Watch it play.']),
				h('p', { class: 'lead' }, 'Lilanaya is an arena for game theory. You write a strategy, and it plays against other strategies. Start by playing one game yourself. It takes a minute.'),
				h('p', null, button('Play a round', { primary: true, class: 'btn btn-primary btn-big', onclick: () => goTo(playHeading) }))
			]),

			h('section', { id: 'play', class: 'landing-section landing-screen stack-md' }, [playHeading, playByHand(() => goTo(arenaHeading))]),

			h('section', { id: 'strategies', class: 'landing-section landing-screen stack-md' }, [
				h('div', { class: 'stack' }, [arenaHeading, h('p', null, 'A strategy is a plan that plays for you. Four well-known ones are built in. Pick two and watch twenty rounds.')]),
				lab.node
			]),

			h('section', { id: 'ranking', class: 'landing-section landing-screen stack-md' }, [
				h('div', { class: 'stack' }, [
					rankHeading,
					h('p', { class: 'prose' }, `One match is not enough. Every strategy plays every other one. The highest average score wins. In this small group, ${ranking.winner} comes out on top.`)
				]),
				ranking.node,
				h('p', null, button('Your turn', { onclick: () => goTo(turnHeading) }))
			]),

			h('section', { id: 'your-turn', class: 'landing-section landing-screen stack-lg' }, [
				turnHeading,
				steps(),
				h('div', { class: 'landing-close stack' }, [
					h('p', null, linkButton('Create your first strategy', start, { primary: true, class: 'btn btn-primary btn-big' })),
					user ? null : h('p', { class: 'meta' }, 'Sign up with your email.'),
					h('p', { class: 'meta' }, ['Want the full story first? ', h('a', { href: '/how-it-works' }, 'How it works'), ' has three short animations and a closer look at the Prisoner’s Dilemma.'])
				])
			])
		])
	};
}
