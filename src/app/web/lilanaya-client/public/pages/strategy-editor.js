import { ApiError, getJson, postJson, putJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { describeGuard } from '../components/machine.js';
import { roundStrip, stripLegend } from '../components/round-strip.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { getUser } from '../session.js';
import { signInPrompt } from './sign-in.js';

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
	['round_index', 'the game has lasted at least']
];

/** Signed-in users pick how to write the strategy; the choice is fixed once saved. */
export async function chooseModePage() {
	if (!(await getUser())) return { title: 'New strategy', node: signInPrompt('make a strategy') };
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

export async function strategyEditorPage({ params, navigate }) {
	const user = await getUser();
	if (!user) return { title: 'Strategy', node: signInPrompt(params.id ? 'edit a strategy' : 'make a strategy') };

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
	const kind = mode === 'code' ? codeFields(existing) : rulesFields(existing, { decisions, world, user, name, description, check: () => check() });

	const draft = () => ({ name: name.value, description: description.value, mode, ...kind.draft() });

	const showProblems = (message, details) =>
		h('div', { class: 'error-box', role: 'alert' }, [h('p', null, message), details.length ? h('ul', null, details.map((d) => h('li', null, d.path ? `${d.path}: ${d.message}` : d.message))) : null]);

	/** Asks the API to check the draft and play the sample matches; returns whether it is valid. */
	async function check() {
		checkButton.disabled = true;
		try {
			const result = await postJson('/api/strategies/preview', { world_id: world.id, strategy: draft() });
			report.replaceChildren(h('div', { class: 'stack' }, previewReport(result, decisions, showProblems)));
			return result.valid;
		} catch (error) {
			if (!(error instanceof ApiError)) console.error(error);
			report.replaceChildren(showProblems(error instanceof ApiError ? error.message : 'The strategy could not be checked.', error.details ?? []));
			return false;
		} finally {
			checkButton.disabled = false;
		}
	}

	const checkButton = button('Check and preview', { onclick: check });
	const saveButton = button(existing ? 'Save changes' : 'Save strategy', { primary: true, type: 'submit' });
	const saveErrors = h('div', { 'aria-live': 'assertive' });

	const form = h(
		'form',
		{
			class: 'stack-lg editor',
			novalidate: true,
			onsubmit: async (e) => {
				e.preventDefault();
				saveErrors.replaceChildren();
				saveButton.disabled = true;
				try {
					const body = { ...draft(), visibility: visibility.value, world_id: world.id };
					if (existing) await putJson(`/api/strategies/${encodeURIComponent(existing.id)}`, body);
					else await postJson('/api/strategies', body);
					await navigate('/strategies');
				} catch (error) {
					saveButton.disabled = false;
					if (!(error instanceof ApiError)) console.error(error);
					saveErrors.replaceChildren(showProblems(error instanceof ApiError ? error.message : 'The strategy could not be saved.', error.details ?? []));
					if ((error.details ?? []).some((d) => d.path === 'name')) name.focus();
				}
			}
		},
		[
			h('div', { class: 'field form' }, [h('label', { class: 'field-label', for: 'strategy-name' }, 'Name'), name]),
			h('div', { class: 'field form' }, [h('label', { class: 'field-label', for: 'strategy-description' }, 'Description'), description]),
			kind.body,
			h('div', { class: 'stack' }, [
				sectionHeading('Preview'),
				h('p', { class: 'meta' }, `Checks the strategy and plays ${decisions.length === 2 ? 'two' : 'some'} short sample matches · Uses none of your runs`),
				h('div', null, checkButton),
				report
			]),
			h('div', { class: 'field form' }, [
				h('label', { class: 'field-label', for: 'strategy-visibility' }, 'Who can see it'),
				visibility,
				h('p', { class: 'meta', id: 'strategy-visibility-hint' }, 'Public strategies can be read and entered in runs by anyone · A run with a private strategy is private too')
			]),
			saveErrors,
			h('div', { class: 'row' }, [saveButton, linkButton('Cancel', '/strategies')])
		]
	);

	return {
		title,
		node: h('section', { class: 'stack-lg' }, [
			h('div', { class: 'stack' }, [sectionHeading(title, { level: 1 }), h('p', { class: 'meta' }, `${mode === 'code' ? 'Code' : 'Rules'} · For ${world.metadata.name} · Decisions ${decisions.join(', ')}`)]),
			form
		])
	};
}

/** What the preview found: problems, warnings, and the sample matches as round strips. */
function previewReport(result, decisions, showProblems) {
	if (!result.valid) return [showProblems('This strategy cannot play yet:', result.errors)];
	const index = (moves) => moves.map((m) => decisions.indexOf(m));
	return [
		h('p', { role: 'status' }, 'The strategy is valid.'),
		result.warnings.length
			? h('div', { class: 'notice' }, [h('p', null, 'Worth a look, though it will run:'), h('ul', null, result.warnings.map((w) => h('li', null, w.message)))])
			: null,
		result.samples.map((s) =>
			h('div', { class: 'stack-sm' }, [
				h('p', null, [h('strong', null, `Against ${s.opponent.name}`), ` · ${s.rounds} rounds · You ${s.totals[0]}, them ${s.totals[1]}`]),
				roundStrip({
					movesA: index(s.own_moves),
					movesB: index(s.opponent_moves),
					label: `Your strategy played ${s.own_moves.join(' ')}. ${s.opponent.name} played ${s.opponent_moves.join(' ')}.`
				}).node
			])
		),
		result.samples.length
			? stripLegend({ top: 'your strategy', bottom: 'the opponent', solid: `Plays ${decisions[0]}`, hollow: decisions.length === 2 ? `Plays ${decisions[1]}` : 'Plays anything else' })
			: null
	];
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

function rulesFields(existing, { decisions, world, user, name, description, check }) {
	let model = toModel(existing, decisions);
	const box = h('div', { class: 'stack-lg' });

	/** Redraws the states, then puts keyboard focus on the element `focus` finds. */
	const draw = (focus) => {
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
	const canCompile = Boolean(user.features?.compile);
	const prompt = h(
		'textarea',
		{ id: 'strategy-prompt', maxlength: '2000', rows: '3', 'aria-describedby': 'strategy-prompt-hint', placeholder: 'Cooperate at first. If the opponent defects twice in a row, defect until they cooperate again.' },
		existing?.prompt ?? ''
	);
	const compileNote = h('div', { 'aria-live': 'polite' });
	const compileButton = button('Turn into rules', {
		disabled: !canCompile,
		onclick: async () => {
			compileNote.replaceChildren();
			compileButton.disabled = true;
			compileButton.textContent = 'Working…';
			try {
				const result = await postJson('/api/strategies/compile', { world_id: world.id, prompt: prompt.value });
				model = toModel(result.strategy, decisions);
				if (!name.value.trim()) name.value = result.strategy.name;
				if (!description.value.trim()) description.value = result.strategy.description;
				draw();
				compileNote.replaceChildren(
					h('div', { class: 'notice stack-sm' }, [
						h('p', null, [h('strong', null, 'Rules written from your description.'), ' Read them below and change anything that is not what you meant.']),
						result.rationale ? h('p', null, result.rationale) : null
					])
				);
				await check();
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
				const message = error instanceof ApiError ? (error.details[0] ? `The description ${error.details[0].message}.` : error.message) : 'The description could not be turned into rules.';
				compileNote.replaceChildren(h('div', { class: 'error-box', role: 'alert' }, h('p', null, message)));
			}
			compileButton.disabled = false;
			compileButton.textContent = 'Turn into rules';
		}
	});

	draw();
	const body = h('div', { class: 'stack-lg' }, [
		h('div', { class: 'stack' }, [
			h('div', { class: 'field' }, [
				h('label', { class: 'field-label', for: 'strategy-prompt' }, 'Describe it in plain English'),
				prompt,
				h(
					'p',
					{ class: 'meta', id: 'strategy-prompt-hint' },
					canCompile ? 'Optional · Replaces the rules below · You can still edit them by hand afterwards' : 'Not available on this server · Build the rules by hand below'
				)
			]),
			h('div', null, compileButton),
			compileNote
		]),
		h('div', { class: 'stack' }, [
			sectionHeading('Rules'),
			h('p', { class: 'prose' }, 'The strategy is always in one state and plays that state’s decision. Before each round it may move to another state, depending on what happened in the round before.'),
			box
		])
	]);
	return { body, draft: () => ({ ...fromModel(model), ...(prompt.value.trim() ? { prompt: prompt.value } : {}) }) };
}
