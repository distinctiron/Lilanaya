import { allowanceLine, strategiesLeft } from '../components/plan-words.js';
import { ApiError, getJson, postJson, putJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { describeGuard } from '../components/machine.js';
import { roundStrip, stripLegend } from '../components/round-strip.js';
import { avatar, revealAvatar } from '../components/avatar.js';
import { screen } from '../components/screen.js';
import { sectionHeading } from '../components/section-heading.js';
import { h, prefersReducedMotion } from '../dom.js';
import { getUser } from '../session.js';
import { logInPrompt } from './sign-in.js';

const CODE_TEMPLATE = `// Called before every round. Return one of input.world.allowedDecisions.
function decide(input) {
  // First round: nothing has happened yet.
  if (input.roundIndex === 0) return "C";
  // Afterwards: copy what the opponent did last.
  return input.lastOpponentAction;
}
`;

const CODE_INPUT_FIELDS = [
	['roundIndex', 'Number of the round about to be played; 0 for the first'],
	['rounds', 'How many rounds the match has; the last round is roundIndex === rounds - 1'],
	['lastOwnAction', 'What you played last round, or null in the first round'],
	['lastOpponentAction', 'What the opponent played last round, or null in the first round'],
	['opponentStreak', 'How many rounds in a row the opponent has played lastOpponentAction'],
	['outcomeStreak', 'How many rounds in a row have ended exactly like the last one'],
	['ownCounts', 'How often you have played each decision so far, for example { C: 3, D: 1 }'],
	['opponentCounts', 'The same for the opponent'],
	['ownScore, opponentScore', 'Points so far in this match'],
	['world.allowedDecisions', 'The decisions this world allows, for example ["C", "D"]']
];

const CONDITION_TYPES = [
	['last_opponent_action', 'the opponent just played'],
	['last_own_action', 'I just played'],
	['opponent_streak', 'the opponent has repeated'],
	['outcome_streak', 'the same result has repeated'],
	['round_index', 'the game has lasted at least'],
	['rounds_left', 'the rounds left are at most']
];

/** Signed-in users pick how to write the strategy; the choice is fixed once saved. */
export async function chooseModePage() {
	if (!(await getUser())) return { title: 'New strategy', node: logInPrompt('make a strategy') };
	return {
		title: 'New strategy',
		node: h('section', { class: 'stack-lg' }, [
			h('div', { class: 'stack' }, [sectionHeading('New strategy', { level: 1 }), h('p', { class: 'prose' }, 'Pick how you want to write it. This cannot be changed once the strategy is saved.')]),
			h('ul', { class: 'mode-cards' }, [
				h('li', { class: 'card stack' }, [
					sectionHeading('Rules'),
					h('p', null, 'Describe the strategy in plain English, or build it from states and conditions. Easy to read, and anyone can see exactly what it does.'),
					h('p', null, linkButton('Write rules', '/strategies/new/rules', { primary: true }))
				]),
				h('li', { class: 'card stack' }, [
					sectionHeading('Code'),
					h('p', null, 'Write a JavaScript function that returns a decision each round. For behaviour that states and conditions cannot express, such as counting.'),
					h('p', null, linkButton('Write code', '/strategies/new/code'))
				])
			])
		])
	};
}

// --- conditions: the stored guard and the form's flat version of it ---

const newCondition = (decisions) => ({ negate: false, op: 'last_opponent_action', decision: decisions[decisions.length - 1], own: decisions[0], opponent: decisions[0], n: 2 });

/** A guard the form can show: `{ join, conditions }`, or `{ raw }` when it is nested deeper than the form goes. */
function toForm(guard, decisions) {
	const simple = (g) => {
		const base = newCondition(decisions);
		switch (g?.op) {
			case 'not': {
				const inner = simple(g.guard);
				return inner && !inner.negate ? { ...inner, negate: true } : null;
			}
			case 'last_opponent_action':
			case 'last_own_action':
				return { ...base, op: g.op, decision: g.is };
			case 'opponent_streak':
				return { ...base, op: g.op, decision: g.decision, n: g.at_least };
			case 'outcome_streak': {
				const [own, opponent] = String(g.outcome).split('/');
				return { ...base, op: g.op, own, opponent, n: g.at_least };
			}
			case 'round_index':
				return { ...base, op: g.op, n: g.at_least };
			case 'rounds_left':
				return { ...base, op: g.op, n: g.at_most };
			default:
				return null;
		}
	};
	const one = simple(guard);
	if (one) return { join: 'all', conditions: [one] };
	const list = guard?.op === 'and' ? guard.all : guard?.op === 'or' ? guard.any : null;
	const conditions = Array.isArray(list) ? list.map(simple) : [];
	if (conditions.length && conditions.every(Boolean)) return { join: guard.op === 'and' ? 'all' : 'any', conditions };
	return { raw: guard };
}

function toGuard(form) {
	if (form.raw) return form.raw;
	const guards = form.conditions.map((c) => {
		const n = Number.isInteger(c.n) ? c.n : 0;
		const base =
			c.op === 'opponent_streak'
				? { op: c.op, decision: c.decision, at_least: n }
				: c.op === 'outcome_streak'
					? { op: c.op, outcome: `${c.own}/${c.opponent}`, at_least: n }
					: c.op === 'round_index'
						? { op: c.op, at_least: n }
						: c.op === 'rounds_left'
							? { op: c.op, at_most: n }
						: { op: c.op, is: c.decision };
		return c.negate ? { op: 'not', guard: base } : base;
	});
	if (guards.length === 1) return guards[0];
	return form.join === 'any' ? { op: 'or', any: guards } : { op: 'and', all: guards };
}

/** The editor's working copy of a rules strategy. Moves point at state objects, so renaming a state keeps them attached. */
function toModel(doc, decisions) {
	const states = (doc?.states ?? []).map((name) => ({ name, plays: doc.action_map?.[name] ?? decisions[0], moves: [] }));
	if (!states.length) states.push({ name: 'START', plays: decisions[0], moves: [] });
	for (const t of doc?.transitions ?? []) {
		const from = states.find((s) => s.name === t.from);
		const to = states.find((s) => s.name === t.to);
		if (from && to) from.moves.push({ to, guard: toForm(t.guard, decisions) });
	}
	return { states, initial: states.find((s) => s.name === doc?.initial_state) ?? states[0] };
}

function fromModel(model) {
	return {
		states: model.states.map((s) => s.name.trim()),
		initial_state: model.initial.name.trim(),
		action_map: Object.fromEntries(model.states.map((s) => [s.name.trim(), s.plays])),
		transitions: model.states.flatMap((s) => s.moves.map((m) => ({ from: s.name.trim(), to: m.to.name.trim(), guard: toGuard(m.guard) })))
	};
}

// --- the page ---

export async function strategyEditorPage({ params, navigate, onLeave }) {
	const user = await getUser();
	if (!user) return { title: 'Strategy', node: logInPrompt(params.id ? 'edit a strategy' : 'make a strategy') };

	const existing = params.id ? await getJson(`/api/strategies/${encodeURIComponent(params.id)}`) : null;
	if (existing && !existing.mine) throw new ApiError(403, 'forbidden', 'Only its owner can edit a strategy.');
	const mode = existing ? existing.mode : params.mode;
	if (mode !== 'rules' && mode !== 'code') throw new ApiError(404, 'not_found', 'There is no such kind of strategy.');

	const { worlds } = await getJson('/api/worlds');
	const world = worlds.find((w) => w.id === existing?.world_id) ?? worlds[0];
	const decisions = world.decisions.allowed;
	const title = existing ? `Edit ${existing.name}` : mode === 'code' ? 'New code strategy' : 'New rules strategy';

	// Shared fields.
	const name = h('input', { id: 'strategy-name', type: 'text', maxlength: '80', required: true, value: existing?.name ?? '' });
	const description = h('textarea', { id: 'strategy-description', maxlength: '500', rows: '2' }, existing?.description ?? '');
	const visibility = h('select', { id: 'strategy-visibility', 'aria-describedby': 'strategy-visibility-hint' }, [
		h('option', { value: 'private', selected: (existing?.visibility ?? 'private') === 'private' }, 'Private: only you'),
		h('option', { value: 'public', selected: existing?.visibility === 'public' }, 'Public: everyone')
	]);

	// What the two modes add: `body` is the form, `draft()` the fields sent to the API.
	const report = h('div', { class: 'stack', 'aria-live': 'polite' });
	// A new rules strategy starts as a description only: its name, description and rules appear once rules have been written from it.
	const staged = !existing && mode === 'rules' && Boolean(user.features?.compile);
	const kind = mode === 'code' ? codeFields(existing) : rulesFields(existing, { decisions, world, user, name, description, staged, open: () => open(), check: () => check(), clearReport: () => clearReport() });

	const draft = () => ({ name: name.value, description: description.value, mode, ...kind.draft() });

	const showProblems = (message, details) =>
		h('div', { class: 'error-box', role: 'alert' }, [h('p', null, message), details.length ? h('ul', null, details.map((d) => h('li', null, d.path ? `${d.path}: ${d.message}` : d.message))) : null]);

	/**
	 * Asks the API to check the draft, then plays the sample matches round by round so it is plain that
	 * something happened and how the strategy reacts. Returns whether the draft is valid, before the playing ends.
	 */
	let checks = 0; // a newer check, or leaving the page, stops an older one that is still playing
	onLeave(() => (checks += 1));
	async function check() {
		const run = ++checks;
		checkButton.disabled = true;
		checkButton.textContent = 'Checking…';
		const done = () => {
			if (run !== checks) return;
			checkButton.disabled = false;
			checkButton.textContent = 'Check and preview';
		};
		try {
			const result = await postJson('/api/strategies/preview', { world_id: world.id, strategy: draft() });
			if (run !== checks) return result.valid;
			const view = previewReport(result, decisions, world, showProblems);
			report.replaceChildren(h('div', { class: 'stack' }, view.nodes));
			if (!result.valid) kind.showRules?.();
			checkButton.textContent = 'Playing…';
			view.play(() => run !== checks).then(done);
			return result.valid;
		} catch (error) {
			if (!(error instanceof ApiError)) console.error(error);
			report.replaceChildren(showProblems(error instanceof ApiError ? error.message : 'The strategy could not be checked.', error.details ?? []));
			done();
			return false;
		}
	}

	const checkButton = button('Check and preview', { onclick: check });
	const saveButton = button(existing ? 'Save changes' : 'Create strategy', { primary: true, type: 'submit' });
	const saveErrors = h('div', { 'aria-live': 'assertive' });

	// Side by side where there is room, so the first screen keeps its height for the strategy itself.
	const details = h('div', { class: 'field-row editor-details', hidden: staged }, [
		h('div', { class: 'field' }, [h('label', { class: 'field-label', for: 'strategy-name' }, 'Name'), name]),
		h('div', { class: 'field' }, [h('label', { class: 'field-label', for: 'strategy-description' }, 'Description'), description])
	]);
	// The second screen: try the strategy out, say who may see it, and save.
	const finish = screen(
		[
		h('div', { class: 'stack' }, [
			sectionHeading('Preview'),
			h('p', { class: 'meta' }, `Checks the strategy and plays ${decisions.length === 2 ? 'two' : 'some'} short sample matches, round by round · Uses none of your runs`),
			h('div', null, checkButton),
			report
		]),
		h('div', { class: 'field' }, [
			h('label', { class: 'field-label', for: 'strategy-visibility' }, 'Who can see it'),
			h('div', { class: 'form' }, visibility),
			h('p', { class: 'meta', id: 'strategy-visibility-hint' }, 'Public strategies can be read and entered in runs by anyone · A run with a private strategy is private too')
		]),
		saveErrors,
		h('div', { class: 'row' }, [saveButton, linkButton('Cancel', '/strategies')])
		],
		{ class: 'stack-md', hidden: staged }
	);
	/** Forgets an earlier preview: the rules it showed have been replaced. */
	const clearReport = () => {
		checks += 1;
		checkButton.disabled = false;
		checkButton.textContent = 'Check and preview';
		report.replaceChildren();
	};
	// The way on from the first screen, since saving is a screen down.
	const nextStep = h('div', { hidden: staged }, button('Next: preview and save', { onclick: () => finish.scrollIntoView({ block: 'start', behavior: prefersReducedMotion() ? 'auto' : 'smooth' }) }));
	/** Brings in the name, the description and everything needed to finish. */
	const open = () => {
		details.hidden = false;
		finish.hidden = false;
		nextStep.hidden = false;
	};

	// The form is the page: a screen for writing the strategy, and one for trying it out and saving it.
	const page = h(
		'form',
		{
			class: 'screens editor',
			novalidate: true,
			onsubmit: async (e) => {
				e.preventDefault();
				saveErrors.replaceChildren();
				saveButton.disabled = true;
				try {
					const body = { ...draft(), visibility: visibility.value, world_id: world.id };
					if (existing) {
						await putJson(`/api/strategies/${encodeURIComponent(existing.id)}`, body);
						await navigate('/strategies');
					} else {
						// A new strategy is shown off before anything else: its avatar is built and turns on the spot.
						const made = await postJson('/api/strategies', body);
						if (made?.id) page.replaceChildren(h('div', { class: 'screen' }, fighterReveal(made, onLeave)));
						else await navigate('/strategies');
					}
				} catch (error) {
					saveButton.disabled = false;
					if (!(error instanceof ApiError)) console.error(error);
					saveErrors.replaceChildren(showProblems(error instanceof ApiError ? error.message : 'The strategy could not be saved.', error.details ?? []));
				}
			}
		},
		[
			screen(
				[
					h('div', { class: 'stack' }, [
						h('div', { class: 'avatar-heading' }, [existing ? avatar(existing.id, { size: 40 }) : null, sectionHeading(title, { level: 1 })]),
						h('p', { class: 'meta' }, `${mode === 'code' ? 'Code' : 'Rules'} · For ${world.metadata.name} · Decisions ${decisions.join(', ')}${existing ? '' : ' · Gets its avatar when created'}`)
					]),
					details,
					kind.body,
					nextStep
				],
				{ class: 'stack-md' }
			),
			finish
		]
	);
	return { title, node: page };
}

const BLOCK_MS = 30; // one block of a new strategy's avatar

/**
 * Shows off a strategy that was just created: its avatar is built block by block on a floor
 * line, then turns on the spot the way a fighter does on a select screen.
 */
function fighterReveal(s, onLeave) {
	const still = prefersReducedMotion();
	const face = avatar(s.id, { size: 192, build: !still, label: `The avatar of ${s.name}` });
	const stand = h('div', { class: 'fighter-stand' }, [h('div', { class: 'fighter-turn' }, face), h('div', { class: 'fighter-floor', 'aria-hidden': 'true' })]);
	if (!still) {
		let shown = 0;
		const timer = setInterval(() => {
			if (revealAvatar(face, ++shown) > shown) return;
			clearInterval(timer);
			stand.classList.add('is-ready');
		}, BLOCK_MS);
		onLeave(() => clearInterval(timer));
	}
	const heading = sectionHeading('Your new competitor', { level: 1 });
	heading.tabIndex = -1;
	requestAnimationFrame(() => {
		window.scrollTo(0, 0);
		heading.focus({ preventScroll: true });
	});
	return h('div', { class: 'fighter stack-lg' }, [
		heading,
		stand,
		h('div', { class: 'stack', role: 'status' }, [
			h('p', { class: 'fighter-name' }, s.name),
			s.description ? h('p', { class: 'prose' }, s.description) : null,
			h('p', { class: 'meta' }, `${s.mode === 'code' ? 'Code' : 'Rules'} · ${s.visibility === 'public' ? 'Public' : 'Private'} · Created`)
		]),
		h('div', { class: 'row fighter-actions' }, [linkButton('Send it into a run', '/runs/new', { primary: true }), linkButton('All strategies', '/strategies'), linkButton('Edit', `/strategies/${encodeURIComponent(s.id)}/edit`)])
	]);
}

/** Shown while the language model is writing rules: marching blocks and a blinking caret. */
function workingBox() {
	return h('div', { class: 'working', role: 'status' }, [
		h('span', { class: 'working-blocks', 'aria-hidden': 'true' }, Array.from({ length: 8 }, () => h('span'))),
		h('p', { class: 'working-title' }, ['Working on your rules', h('span', { class: 'working-caret', 'aria-hidden': 'true' })]),
		h('p', { class: 'meta' }, 'Reading your description and writing rules from it · This can take half a minute')
	]);
}

const ROUND_MS = 150; // one round of a sample match
const BETWEEN_MS = 600; // the pause before the next sample match

/**
 * What the preview found: problems, warnings, and the sample matches as round strips.
 * Returns `{ nodes, play(stopped) }`: `play` runs the matches one after the other, a round at a
 * time, with the points adding up; without it (and under reduced motion) they are shown finished.
 */
function previewReport(result, decisions, world, showProblems) {
	if (!result.valid) return { nodes: [showProblems('This strategy cannot play yet:', result.errors)], play: async () => {} };
	const index = (moves) => moves.map((m) => decisions.indexOf(m));
	const points = (own, opponent) => world.payoff?.matrix?.[own]?.[opponent] ?? null;
	const samples = result.samples.map((s) => {
		const strip = roundStrip({
			movesA: index(s.own_moves),
			movesB: index(s.opponent_moves),
			label: `Your strategy played ${s.own_moves.join(' ')}. ${s.opponent.name} played ${s.opponent_moves.join(' ')}.`
		});
		const score = h('span');
		const caption = h('p', { class: 'meta preview-caption', 'aria-hidden': 'true' });
		/** Shows the match as it stood after `k` rounds. */
		const show = (k) => {
			strip.show(k);
			let totals = [0, 0];
			for (let r = 0; r < k && totals; r++) {
				const won = points(s.own_moves[r], s.opponent_moves[r]);
				totals = won ? [totals[0] + won[0], totals[1] + won[1]] : null;
			}
			if (k >= s.rounds || !totals) totals = k >= s.rounds ? s.totals : null;
			score.textContent = totals ? ` · You ${totals[0]}, them ${totals[1]}` : '';
			caption.textContent =
				k === 0 ? 'About to start' : k >= s.rounds ? `${s.rounds} rounds played` : `Round ${k} of ${s.rounds} · You played ${s.own_moves[k - 1]} · ${s.opponent.name} played ${s.opponent_moves[k - 1]}`;
		};
		show(s.rounds);
		const node = h('div', { class: 'stack-sm preview-match' }, [h('p', null, [h('strong', null, `Against ${s.opponent.name}`), ` · ${s.rounds} rounds`, score]), strip.node, caption]);
		return { node, show, rounds: s.rounds };
	});
	const nodes = [
		h('p', { role: 'status' }, 'The strategy is valid.'),
		result.warnings.length
			? h('div', { class: 'notice' }, [h('p', null, 'Worth a look, though it will run:'), h('ul', null, result.warnings.map((w) => h('li', null, w.message)))])
			: null,
		samples.map((s) => s.node),
		samples.length
			? stripLegend({ top: 'your strategy', bottom: 'the opponent', solid: `Plays ${decisions[0]}`, hollow: decisions.length === 2 ? `Plays ${decisions[1]}` : 'Plays anything else' })
			: null
	];
	const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
	const play = async (stopped) => {
		if (prefersReducedMotion() || !samples.length) return;
		for (const s of samples) {
			s.show(0);
			s.node.classList.add('is-waiting');
		}
		samples[0].node.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
		for (const s of samples) {
			s.node.classList.remove('is-waiting');
			s.node.classList.add('is-playing');
			for (let k = 1; k <= s.rounds; k++) {
				await wait(ROUND_MS);
				if (stopped()) return;
				s.show(k);
			}
			s.node.classList.remove('is-playing');
			await wait(BETWEEN_MS);
			if (stopped()) return;
		}
	};
	return { nodes, play };
}

// --- code mode ---

function codeFields(existing) {
	const source = h(
		'textarea',
		{ id: 'strategy-source', class: 'code-input', spellcheck: 'false', autocapitalize: 'off', autocomplete: 'off', wrap: 'off', 'aria-describedby': 'strategy-source-hint' },
		existing?.source ?? CODE_TEMPLATE
	);
	const body = h('div', { class: 'stack' }, [
		h('div', { class: 'field' }, [
			h('label', { class: 'field-label', for: 'strategy-source' }, 'Code'),
			source,
			h('p', { class: 'meta', id: 'strategy-source-hint' }, 'JavaScript · Must define decide(input) · No clock, no randomness, no network · At most 16 KB')
		]),
		h('details', null, [
			h('summary', null, 'What decide receives'),
			h(
				'dl',
				{ class: 'rules' },
				CODE_INPUT_FIELDS.flatMap(([field, meaning]) => [h('dt', null, h('code', null, field)), h('dd', null, meaning)])
			),
			h('p', { class: 'prose' }, 'The function is called once per round for each match and must return the same answer for the same input. A call that loops for too long, uses too much memory or returns anything but an allowed decision stops the run.')
		])
	]);
	return { body, draft: () => ({ source: source.value }) };
}

// --- rules mode ---

const FLAG_KINDS = {
	violation: { title: 'Violates the rules', mark: 'X' },
	assumption: { title: 'Made assumptions', mark: '!' }
};

/** One flag box: a coloured frame, a mark and a title that say what kind of attention is needed, then the details. */
function flagBox(kind, children) {
	const { title, mark } = FLAG_KINDS[kind];
	return h('div', { class: `flag flag-${kind} stack-sm`, role: kind === 'violation' ? 'alert' : 'status' }, [
		h('div', { class: 'flag-head' }, [h('span', { class: 'flag-mark', 'aria-hidden': 'true' }, mark), h('strong', { class: 'flag-title' }, title)]),
		children
	]);
}

/** The flags raised about a plain-English description, as one box per kind: rule violations first, then assumptions. */
function flagBoxes(flags) {
	const sentence = (text) => (/[.!?]$/.test(text) ? text : `${text}.`);
	const parts = (n) => (n === 1 ? 'One part' : `${n} parts`);
	const list = (items) =>
		h(
			'ul',
			{ class: 'stack-sm' },
			items.map((f) =>
				h('li', null, [
					f.asked ? h('strong', null, `“${f.asked}”`) : null,
					f.why ? ` ${sentence(f.why)}` : null,
					f.instead ? [' ', h('span', { class: 'flag-instead' }, 'In the rules:'), ` ${sentence(f.instead)}`] : null,
					f.code ? ' A code strategy can do exactly this.' : null
				])
			)
		);
	const broken = flags.filter((f) => f.kind === 'violation');
	const assumed = flags.filter((f) => f.kind !== 'violation');
	return [
		broken.length
			? flagBox('violation', [h('p', null, `${parts(broken.length)} of your description ${broken.length === 1 ? 'asks' : 'ask'} for something no strategy may do. The rules do not do it.`), list(broken)])
			: null,
		assumed.length
			? flagBox('assumption', [
					h('p', null, `${parts(assumed.length)} of your description could not be followed to the letter, so a choice was made for you. Check that it is what you meant.`),
					list(assumed),
					assumed.some((f) => f.code)
						? h('p', null, ['Rules cannot do everything code can: ', h('a', { href: '/strategies/new/code', target: '_blank', rel: 'noopener' }, 'write it as code instead (opens a new tab)'), '.'])
						: null
				])
			: null
	].filter(Boolean);
}

function rulesFields(existing, { decisions, world, user, name, description, staged, open, check, clearReport }) {
	let model = toModel(existing, decisions);
	const box = h('div', { class: 'stack-lg' });
	const rulesCount = h('span');

	/** Redraws the states, then puts keyboard focus on the element `focus` finds. */
	const draw = (focus) => {
		const moves = model.states.reduce((n, state) => n + state.moves.length, 0);
		rulesCount.textContent = `${model.states.length} ${model.states.length === 1 ? 'state' : 'states'} · ${moves} ${moves === 1 ? 'move' : 'moves'}`;
		box.replaceChildren(...model.states.map(stateCard), h('div', null, button('Add a state', { onclick: addState, 'data-role': 'add-state' })));
		focus?.(box)?.focus();
	};

	const addState = () => {
		const taken = new Set(model.states.map((s) => s.name));
		let n = model.states.length + 1;
		while (taken.has(`STATE_${n}`)) n += 1;
		model.states.push({ name: `STATE_${n}`, plays: decisions[0], moves: [] });
		draw((root) => root.querySelector(`[data-state="${model.states.length - 1}"] input[type="text"]`));
	};

	const decisionSelect = (value, onchange, label) =>
		h('select', { 'aria-label': label, onchange: (e) => onchange(e.target.value) }, decisions.map((d) => h('option', { value: d, selected: d === value }, d)));
	const countInput = (c, label) =>
		h('input', { type: 'number', inputmode: 'numeric', min: '1', step: '1', value: String(c.n), 'aria-label': label, oninput: (e) => (c.n = e.target.value === '' ? NaN : Number(e.target.value)) });

	function conditionRow(move, c, at) {
		const parts =
			c.op === 'opponent_streak'
				? [decisionSelect(c.decision, (v) => (c.decision = v), 'Decision'), countInput(c, 'How many times'), 'times in a row']
				: c.op === 'outcome_streak'
					? [
							countInput(c, 'How many rounds'),
							'rounds in a row: I played',
							decisionSelect(c.own, (v) => (c.own = v), 'My decision'),
							'and the opponent',
							decisionSelect(c.opponent, (v) => (c.opponent = v), "The opponent's decision")
						]
					: c.op === 'round_index'
						? [countInput(c, 'How many rounds'), 'rounds']
						: c.op === 'rounds_left'
							? [countInput(c, 'How many rounds'), '(counting the coming round)']
						: [decisionSelect(c.decision, (v) => (c.decision = v), 'Decision')];
		return h('div', { class: 'sentence' }, [
			h('select', { 'aria-label': 'True or not true', onchange: (e) => (c.negate = e.target.value === 'not') }, [
				h('option', { value: 'is', selected: !c.negate }, at === 0 ? 'when' : 'when also'),
				h('option', { value: 'not', selected: c.negate }, at === 0 ? 'unless' : 'unless also')
			]),
			h(
				'select',
				{
					'aria-label': 'Kind of condition',
					onchange: (e) => {
						c.op = e.target.value;
						draw();
					}
				},
				CONDITION_TYPES.map(([value, label]) => h('option', { value, selected: value === c.op }, label))
			),
			parts,
			move.guard.conditions.length > 1
				? button('Remove', {
						'aria-label': 'Remove this condition',
						onclick: () => {
							move.guard.conditions.splice(at, 1);
							draw();
						}
					})
				: null
		]);
	}

	function moveBlock(state, move, at, stateAt) {
		const target = h(
			'select',
			{ 'aria-label': 'State to go to', onchange: (e) => (move.to = model.states[Number(e.target.value)]) },
			model.states.map((s, i) => h('option', { value: String(i), selected: s === move.to }, s.name || `State ${i + 1}`))
		);
		const raw = move.guard.raw;
		return h('li', { class: 'move stack-sm', 'data-move': String(at) }, [
			h('div', { class: 'sentence' }, [
				h('span', null, 'Go to'),
				target,
				!raw && move.guard.conditions.length > 1
					? h('select', { 'aria-label': 'How the conditions combine', onchange: (e) => (move.guard.join = e.target.value) }, [
							h('option', { value: 'all', selected: move.guard.join === 'all' }, 'if all of these hold'),
							h('option', { value: 'any', selected: move.guard.join === 'any' }, 'if any of these holds')
						])
					: null
			]),
			raw
				? [h('p', null, `when ${describeGuard(raw)}`), h('p', { class: 'meta' }, 'This condition is more nested than the form can show · It is kept as it is unless you replace it')]
				: move.guard.conditions.map((c, i) => conditionRow(move, c, i)),
			h('div', { class: 'row' }, [
				raw
					? button('Replace condition', {
							onclick: () => {
								move.guard = { join: 'all', conditions: [newCondition(decisions)] };
								draw();
							}
						})
					: button('Add condition', {
							onclick: () => {
								move.guard.conditions.push(newCondition(decisions));
								draw();
							}
						}),
				at > 0
					? button('Move up', {
							'aria-label': 'Check this move earlier',
							onclick: () => {
								state.moves.splice(at - 1, 2, move, state.moves[at - 1]);
								draw((root) => root.querySelector(`[data-state="${stateAt}"] [data-move="${at - 1}"] select`));
							}
						})
					: null,
				button('Remove move', {
					onclick: () => {
						state.moves.splice(at, 1);
						draw((root) => root.querySelector(`[data-state="${stateAt}"] [data-role="add-move"]`));
					}
				})
			])
		]);
	}

	function stateCard(state, at) {
		const label = () => state.name || `State ${at + 1}`;
		const legend = h('legend', { class: 'field-label' }, `State: ${label()}`);
		const nameInput = h('input', {
			type: 'text',
			value: state.name,
			maxlength: '40',
			'aria-label': 'State name',
			autocapitalize: 'characters',
			oninput: (e) => {
				state.name = e.target.value;
				legend.textContent = `State: ${label()}`;
			},
			// Other states' "go to" lists show this name: bring them up to date once typing is done.
			onchange: () => draw()
		});
		return h('fieldset', { class: 'card stack', 'data-state': String(at) }, [
			legend,
			h('div', { class: 'sentence' }, [
				h('span', null, 'Called'),
				nameInput,
				h('span', null, 'plays'),
				decisionSelect(state.plays, (v) => (state.plays = v), `What ${label()} plays`)
			]),
			h('label', { class: 'check' }, [
				h('input', {
					type: 'radio',
					name: 'initial-state',
					checked: model.initial === state,
					onchange: () => (model.initial = state)
				}),
				h('span', null, 'The strategy starts here')
			]),
			state.moves.length
				? [h('p', { class: 'meta' }, 'Before each round · Checked top to bottom · The first that fits wins'), h('ul', { class: 'moves' }, state.moves.map((m, i) => moveBlock(state, m, i, at)))]
				: h('p', { class: 'meta' }, 'No moves: once here, the strategy stays here'),
			h('div', { class: 'row' }, [
				button('Add a move', {
					'data-role': 'add-move',
					'aria-label': `Add a move out of ${label()}`,
					onclick: () => {
						state.moves.push({ to: model.states.find((s) => s !== state) ?? state, guard: { join: 'all', conditions: [newCondition(decisions)] } });
						draw((root) => root.querySelector(`[data-state="${at}"] [data-move="${state.moves.length - 1}"] select`));
					}
				}),
				model.states.length > 1
					? button('Remove state', {
							'aria-label': `Remove state ${label()}`,
							onclick: () => {
								model.states.splice(at, 1);
								for (const s of model.states) s.moves = s.moves.filter((m) => m.to !== state);
								if (model.initial === state) model.initial = model.states[0];
								draw((root) => root.querySelector('[data-role="add-state"]'));
							}
						})
					: null
			])
		]);
	}

	// Plain English: the description is sent to the language model, and the rules it answers with replace the ones below.
	// When `staged`, the description is all there is at first; the name, the description and the rules are brought in
	// once rules have been written, and only after the author has accepted any flags.
	const canCompile = Boolean(user.features?.compile);
	let opened = !staged;
	let generated = { name: '', description: '' };
	const openHint = canCompile ? 'Optional · Replaces the rules below · You can still edit them by hand afterwards' : 'Not available on this server · Build the rules by hand below';
	const hint = h('p', { class: 'meta', id: 'strategy-prompt-hint' }, staged ? 'Press Enter to turn it into rules · Shift + Enter for a new line' : openHint);
	const prompt = h(
		'textarea',
		{
			id: 'strategy-prompt',
			maxlength: '2000',
			rows: '3',
			'aria-describedby': 'strategy-prompt-hint',
			placeholder: 'Cooperate at first. If the opponent defects twice in a row, defect until they cooperate again.',
			// Enter sends the description only while it is all there is: later it would replace rules edited by hand.
			onkeydown: (e) => {
				if (opened || e.key !== 'Enter' || e.shiftKey || e.isComposing) return;
				e.preventDefault();
				if (!compileButton.disabled) compileButton.click();
			}
		},
		existing?.prompt ?? ''
	);
	const compileNote = h('div', { 'aria-live': 'polite' });
	// The rules are folded away unless they are the only way in: written from a description, they are detail to look at on request.
	const rulesFold = h('details', { open: !canCompile || (existing && !existing.prompt) ? '' : null }, [
		h('summary', null, [h('span', { class: 'section-heading' }, 'Rules'), h('span', { class: 'meta fold-tags' }, [rulesCount, ' · Open to read or change them'])]),
		h('div', { class: 'fold-body stack-lg' }, [
			h('p', { class: 'prose' }, 'The strategy is always in one state and plays that state’s decision. Before each round it may move to another state, depending on what happened in the round before.'),
			box
		])
	]);
	const rulesSection = h('div', { class: 'card fold', hidden: staged }, rulesFold);

	/** Puts the generated name and description in their fields, unless the author has written their own. */
	const fill = (strategy) => {
		if (!name.value.trim() || name.value === generated.name) name.value = strategy.name;
		if (!description.value.trim() || description.value === generated.description) description.value = strategy.description;
		generated = { name: strategy.name, description: strategy.description };
	};
	/** Brings in the name, the description, the rules and the rest of the form. */
	const openUp = () => {
		opened = true;
		rulesSection.hidden = false;
		byHand.hidden = true;
		hint.textContent = openHint;
		open();
	};

	const byHand = button('Build the rules by hand instead', {
		hidden: !staged,
		onclick: () => {
			openUp();
			rulesFold.open = true;
		}
	});
	const compileButton = button('Turn into rules', {
		disabled: !canCompile,
		primary: staged,
		onclick: async () => {
			compileNote.replaceChildren(workingBox());
			compileButton.disabled = true;
			byHand.disabled = true;
			prompt.readOnly = true;
			try {
				const result = await postJson('/api/strategies/compile', { world_id: world.id, prompt: prompt.value });
				model = toModel(result.strategy, decisions);
				draw();
				const flags = result.flags ?? [];
				const written = h('div', { class: 'notice stack-sm' }, [
					h('p', null, [h('strong', null, 'Rules written from your description.'), ' They are folded away under Rules below: open that to read them or change anything that is not what you meant. Press Check and preview to watch them play.']),
					result.rationale ? h('p', null, result.rationale) : null
				]);
				/** Shows the name and description that were generated, and brings in the rest of the form. */
				const finish = async (first) => {
					fill(result.strategy);
					if (first) {
						openUp();
						name.focus();
					}
					// Rules the validator still rejects are reported at once; rules that pass are played when the author asks.
					if (!result.valid) await check();
					else clearReport();
				};
				if (!opened && flags.length) {
					// Nothing is named or shown as rules until the author has read the flags and chosen to go on.
					const gate = h('div', { class: 'stack-sm flag-gate' }, [
						h('p', null, 'Continue if that is what you want, or change the description and try again.'),
						h('div', { class: 'row' }, [
							button('Continue with these rules', {
								primary: true,
								onclick: async () => {
									gate.replaceWith(written);
									await finish(true);
								}
							}),
							button('Change the description', { onclick: () => prompt.focus() })
						])
					]);
					compileNote.replaceChildren(...flagBoxes(flags), gate);
				} else {
					compileNote.replaceChildren(...flagBoxes(flags), written);
					await finish(!opened);
				}
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
				const message = error instanceof ApiError ? (error.details[0] ? `The description ${error.details[0].message}.` : error.message) : 'The description could not be turned into rules.';
				compileNote.replaceChildren(
					error instanceof ApiError && error.code === 'rule_violation'
						? flagBox('violation', [h('p', null, message), h('p', null, 'No rules were written. Change the description and try again.')])
						: h('div', { class: 'error-box', role: 'alert' }, h('p', null, message))
				);
			}
			byHand.disabled = false;
			prompt.readOnly = false;
			// Whatever came of it, show what is left; a failure to ask leaves the last known number in place.
			try {
				usage = await getJson('/api/usage');
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
			}
			showAllowance();
		}
	});

	// Strategies the AI writes: the plan has a number per world (for good) or per paid month, and packs add to it.
	// Say how many are left before one is used.
	let usage = user.usage;
	const allowance = h('p', { class: 'meta', 'aria-live': 'polite', hidden: !canCompile, 'data-role': 'ai-strategies' });
	const showAllowance = () => {
		if (!canCompile || !usage?.ai_requests) {
			compileButton.disabled = !canCompile;
			return;
		}
		const left = strategiesLeft(usage, world.id).total;
		// When none are left, say where more can be had.
		allowance.replaceChildren(allowanceLine(usage, world), ...(left === 0 && user.features?.billing ? [' · ', h('a', { href: '/plans' }, 'Get more')] : []));
		compileButton.disabled = left === 0;
	};
	showAllowance();
	// The numbers that came with the session may be from before an earlier request: ask again.
	if (canCompile) {
		getJson('/api/usage')
			.then((fresh) => {
				usage = fresh;
				showAllowance();
			})
			.catch((error) => {
				if (!(error instanceof ApiError)) console.error(error);
			});
	}

	draw();
	const body = h('div', { class: 'stack-md' }, [
		h('div', { class: 'stack' }, [
			h('div', { class: 'field' }, [h('label', { class: 'field-label', for: 'strategy-prompt' }, staged ? 'Describe your strategy in plain English' : 'Describe it in plain English'), prompt, hint]),
			h('div', { class: 'row' }, [compileButton, byHand]),
			allowance,
			compileNote
		]),
		rulesSection
	]);
	return { body, showRules: () => (rulesFold.open = true), draft: () => ({ ...fromModel(model), ...(prompt.value.trim() ? { prompt: prompt.value } : {}) }) };
}
