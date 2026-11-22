import { h } from '../dom.js';

/** A guard (data, as stored) in plain words. */
export function describeGuard(guard) {
	switch (guard.op) {
		case 'last_opponent_action':
			return `the opponent just played ${guard.is}`;
		case 'last_own_action':
			return `it just played ${guard.is}`;
		case 'opponent_streak':
			return `the opponent has played ${guard.decision} ${guard.at_least} times in a row`;
		case 'outcome_streak':
			return `the last ${guard.at_least} rounds all ended ${guard.outcome}`;
		case 'round_index':
			return `at least ${guard.at_least} rounds have been played`;
		case 'rounds_left':
			return guard.at_most === 1 ? 'it is the last round' : `at most ${guard.at_most} rounds are left`;
		case 'and':
			return guard.all.map((g) => `(${describeGuard(g)})`).join(' and ');
		case 'or':
			return guard.any.map((g) => `(${describeGuard(g)})`).join(' or ');
		case 'not':
			return `not (${describeGuard(guard.guard)})`;
		default:
			return JSON.stringify(guard);
	}
}

/** The states of a rules strategy as a table: what each plays and where it goes next. */
export function machineTable(s) {
	return h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': `State machine of ${s.name}` }, [
		h('table', { class: 'grid-table' }, [
			h('thead', null, h('tr', null, ['State', 'Plays', 'Then moves'].map((t) => h('th', { scope: 'col' }, t)))),
			h(
				'tbody',
				null,
				s.states.map((state) => {
					const exits = s.transitions.filter((t) => t.from === state);
					return h('tr', null, [
						h('th', { scope: 'row' }, state + (state === s.initial_state ? ' (start)' : '')),
						h('td', null, s.action_map[state]),
						h('td', null, exits.length ? exits.map((t) => h('div', null, `to ${t.to} when ${describeGuard(t.guard)}`)) : 'never leaves')
					]);
				})
			)
		])
	]);
}

/** The source of a code strategy. */
export function codeBlock(s) {
	return h('pre', { class: 'code', tabindex: '0', 'aria-label': `Code of ${s.name}` }, h('code', null, s.source));
}
