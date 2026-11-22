// pocket-dilemma.js: the small Prisoner's Dilemma the landing page and the films play in the browser.
// It must tell the same story as the engine on the server, so it is checked against the engine's golden run.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { CLASSICS, PAYOFF, copyThem, playAll, playMatch } from '../../src/app/web/lilanaya-client/public/components/pocket-dilemma.js';

const golden = (name) => JSON.parse(readFileSync(new URL(`../../src/engine/tests/golden/${name}`, import.meta.url), 'utf8'));

test("the payoffs are the built-in world's", () => {
	assert.deepEqual(PAYOFF, { CC: [3, 3], CD: [0, 5], DC: [5, 0], DD: [1, 1] });
	// A dilemma: temptation > reward > punishment > sucker, and taking turns pays less than cooperating.
	const [T, R, P, S] = [PAYOFF.DC[0], PAYOFF.CC[0], PAYOFF.DD[0], PAYOFF.CD[0]];
	assert.ok(T > R && R > P && P > S && 2 * R > T + S);
	// Seen from the other chair, every outcome is the mirror image.
	for (const [a, b] of ['CC', 'CD', 'DC', 'DD'].map((o) => [...o])) assert.deepEqual(PAYOFF[a + b], [...PAYOFF[b + a]].reverse());
});

test('the four classics are the ones the server has built in', () => {
	const summary = golden('pd_baselines_seed42.summary.json');
	assert.deepEqual(Object.keys(CLASSICS).sort(), summary.actors.map((a) => a.id).sort());
	for (const actor of summary.actors) assert.equal(CLASSICS[actor.id].name, actor.name);
});

test("every match of the classics is played move for move as the engine's golden run plays it", () => {
	const summary = golden('pd_baselines_seed42.summary.json');
	const traces = golden('pd_baselines_seed42.traces.json');
	assert.equal(traces.length, 6);
	for (const trace of traces) {
		const match = playMatch(CLASSICS[trace.a], CLASSICS[trace.b], summary.iterations);
		const letters = (side) => trace.moves.map((m) => summary.decisions[m[side]]);
		assert.deepEqual(match.a, letters(0), `${trace.a} against ${trace.b}`);
		assert.deepEqual(match.b, letters(1), `${trace.b} against ${trace.a}`);
		const stored = summary.matches.find((m) => m.a === trace.a && m.b === trace.b);
		assert.deepEqual([match.scoreA.at(-1), match.scoreB.at(-1)], stored.totals, `${trace.a} against ${trace.b}: totals`);
	}
});

test("a whole tournament of the classics ends with the engine's totals", () => {
	const summary = golden('pd_baselines_seed42.summary.json');
	const players = Object.entries(CLASSICS).map(([id, rule]) => ({ id, rule }));
	assert.deepEqual(playAll(players, summary.iterations), Object.fromEntries(summary.actors.map((a) => [a.id, a.total])));
	// The order players are given in changes nothing.
	assert.deepEqual(playAll([...players].reverse(), summary.iterations), playAll(players, summary.iterations));
});

test('scores run up round by round and never skip one', () => {
	const match = playMatch(CLASSICS.tit_for_tat, CLASSICS.always_defect, 5);
	assert.deepEqual(match, { a: ['C', 'D', 'D', 'D', 'D'], b: ['D', 'D', 'D', 'D', 'D'], scoreA: [0, 1, 2, 3, 4], scoreB: [5, 6, 7, 8, 9] });
	assert.deepEqual(playMatch(CLASSICS.always_cooperate, CLASSICS.always_cooperate, 0), { a: [], b: [], scoreA: [], scoreB: [] });
	const long = playMatch(CLASSICS.win_stay_lose_shift, CLASSICS.always_defect, 101);
	assert.equal(long.a.length, 101);
	assert.deepEqual(long.a.slice(0, 4), ['C', 'D', 'C', 'D'], 'switches after every bad round');
});

test("a visitor's hand-played game uses the same rule the films show: copy the other's last move", () => {
	assert.equal(copyThem('C', 'D'), 'D');
	assert.equal(copyThem('D', 'C'), 'C');
	assert.equal(CLASSICS.tit_for_tat.next, copyThem);
});

test('every classic has a name and a description to show', () => {
	for (const [id, classic] of Object.entries(CLASSICS)) {
		assert.ok(classic.name && classic.text.endsWith('.'), id);
		assert.ok(['C', 'D'].includes(classic.first), id);
		for (const mine of ['C', 'D']) for (const theirs of ['C', 'D']) assert.ok(['C', 'D'].includes(classic.next(mine, theirs)), id);
	}
});
