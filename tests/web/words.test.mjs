// The small functions that turn data into words and numbers on the page.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { describeGuard } from '../../src/app/web/lilanaya-client/public/components/machine.js';
import { abbreviate } from '../../src/app/web/lilanaya-client/public/components/matrix-grid.js';
import { fmt, fmtValue } from '../../src/app/web/lilanaya-client/public/dom.js';

test('every kind of condition the engine knows is put in plain words', () => {
	const cases = [
		[{ op: 'last_opponent_action', is: 'D' }, 'the opponent just played D'],
		[{ op: 'last_own_action', is: 'C' }, 'it just played C'],
		[{ op: 'opponent_streak', decision: 'D', at_least: 2 }, 'the opponent has played D 2 times in a row'],
		[{ op: 'outcome_streak', outcome: 'D/D', at_least: 3 }, 'the last 3 rounds all ended D/D'],
		[{ op: 'round_index', at_least: 10 }, 'at least 10 rounds have been played'],
		[{ op: 'rounds_left', at_most: 1 }, 'it is the last round'],
		[{ op: 'rounds_left', at_most: 3 }, 'at most 3 rounds are left'],
		[{ op: 'not', guard: { op: 'last_opponent_action', is: 'D' } }, 'not (the opponent just played D)'],
		[
			{ op: 'and', all: [{ op: 'last_own_action', is: 'D' }, { op: 'or', any: [{ op: 'round_index', at_least: 4 }, { op: 'rounds_left', at_most: 1 }] }] },
			'(it just played D) and ((at least 4 rounds have been played) or (it is the last round))'
		]
	];
	for (const [guard, words] of cases) assert.equal(describeGuard(guard), words);
});

test('the list above covers every condition the engine has', async () => {
	// The engine's list of conditions, read from its source: a new one must get words here too.
	const { readFileSync } = await import('node:fs');
	const source = readFileSync(new URL('../../src/engine/src/strategy.rs', import.meta.url), 'utf8');
	const body = source.slice(source.indexOf('pub enum Guard {'), source.indexOf('impl Guard'));
	const names = [...body.matchAll(/^\s{4}([A-Z][A-Za-z]+) \{/gm)].map((m) => m[1].replace(/[A-Z]/g, (c, i) => (i ? '_' : '') + c.toLowerCase()));
	assert.equal(names.length, 9, `read ${names.join(', ')}`);
	for (const op of names) {
		const guard = { op, is: 'C', decision: 'C', outcome: 'C/C', at_least: 2, at_most: 2, all: [], any: [], guard: { op: 'round_index', at_least: 1 } };
		assert.ok(!describeGuard(guard).startsWith('{'), `"${op}" is shown as raw data, not in words`);
	}
});

test('a condition nobody has words for is shown as it is, never hidden', () => {
	assert.equal(describeGuard({ op: 'something_new', n: 1 }), '{"op":"something_new","n":1}');
});

test('names are shortened to their initials for the head-to-head table', () => {
	assert.equal(abbreviate('Tit for Tat'), 'TFT');
	assert.equal(abbreviate('Win-Stay Lose-Shift'), 'WSLS');
	assert.equal(abbreviate('  always   defect '), 'AD');
	assert.equal(abbreviate('grudger'), 'G');
	assert.equal(abbreviate(''), '');
	assert.equal(abbreviate(' - '), '');
});

test('scores are shown to two decimals, whole indicator values as they are', () => {
	assert.equal(fmt(3), '3.00');
	assert.equal(fmt(2.345), '2.35');
	assert.equal(fmt(2.344), '2.34');
	assert.equal(fmt(1 / 3, 3), '0.333');
	assert.equal(fmt('2.5'), '2.50');
	assert.equal(fmtValue(42), '42');
	assert.equal(fmtValue(0), '0');
	assert.equal(fmtValue(0.5), '0.50');
	assert.equal(fmtValue(-1.005), '-1.00');
});
