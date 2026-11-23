// What plans, packs and prices are called on the pages.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { aiSentence, allowanceLine, day, money, packWords, strategiesLeft } from '../../src/app/web/lilanaya-client/public/components/plan-words.js';

const world = { id: 'prisoners-dilemma', metadata: { name: 'Prisoner’s Dilemma' } };
const free = (used, pack = 0) => ({ period_end: '2026-12-01T00:00:00Z', ai_requests: { limit: 3, per: 'world', used_by_world: used == null ? {} : { 'prisoners-dilemma': used }, used_this_month: 0 }, packs: { strategies_left: pack, runs_left: 0 } });
const subscriber = (used, pack = 0) => ({ period_end: '2026-12-23T10:00:00Z', ai_requests: { limit: 20, per: 'month', used_by_world: {}, used_this_month: used }, packs: { strategies_left: pack, runs_left: 0 } });

test('amounts are written with their currency', () => {
	assert.equal(money(899, 'eur'), '€8.99');
	assert.equal(money(2199, 'EUR'), '€21.99');
	assert.equal(money(500, 'usd'), '$5.00');
	assert.equal(money(0, 'eur'), '€0.00');
	// A currency code the browser does not know still shows the amount.
	assert.equal(money(599, 'not a currency'), '5.99 NOT A CURRENCY');
});

test('a day is written out, in UTC', () => {
	assert.equal(day('2026-12-23T10:00:00Z'), '23 December 2026');
	assert.equal(day('2027-01-01T00:00:00Z'), '1 January 2027');
	assert.equal(day('2026-12-31T23:59:59Z'), '31 December 2026');
	assert.equal(day(''), '');
	assert.equal(day('nonsense'), '');
});

test('plans and packs are told in strategies and runs, never in requests', () => {
	assert.equal(aiSentence({ limit: 3, per: 'world' }), 'The AI writes 3 strategies for you in each world. They never come back.');
	assert.equal(aiSentence({ limit: 20, per: 'month' }), 'The AI writes 20 strategies a month for you, across all worlds.');
	assert.equal(aiSentence({ limit: 1, per: 'world' }), 'The AI writes 1 strategy for you in each world. They never come back.');
	assert.equal(packWords({ strategies: 10, runs: 100 }), '10 strategies + 100 runs');
	assert.equal(packWords({ strategies: 50, runs: 1000 }), '50 strategies + 1,000 runs');
});

test('what is left is the plan’s own and then the packs', () => {
	assert.deepEqual(strategiesLeft(free(null), 'prisoners-dilemma'), { plan: 3, pack: 0, total: 3 });
	assert.deepEqual(strategiesLeft(free(2, 10), 'prisoners-dilemma'), { plan: 1, pack: 10, total: 11 });
	// More used than the plan gives (the limit was lowered since): none left, not fewer than none.
	assert.deepEqual(strategiesLeft(free(5), 'prisoners-dilemma'), { plan: 0, pack: 0, total: 0 });
	// What was used in one world says nothing about another.
	assert.deepEqual(strategiesLeft(free(3), 'another-world'), { plan: 3, pack: 0, total: 3 });
	assert.deepEqual(strategiesLeft(subscriber(20, 7), 'another-world'), { plan: 0, pack: 7, total: 7 });
	assert.deepEqual(strategiesLeft(undefined, 'prisoners-dilemma'), { plan: 0, pack: 0, total: 0 });
	// An answer from before packs existed.
	assert.deepEqual(strategiesLeft({ ai_requests: { limit: 3, per: 'world', used_by_world: {}, used_this_month: 0 } }, 'x'), { plan: 3, pack: 0, total: 3 });
});

test('the editor says what is left before one is used', () => {
	assert.equal(allowanceLine(free(1), world), 'The AI can write 2 of 3 more strategies for you in Prisoner’s Dilemma · Each press of “Turn into rules” uses one · They never come back');
	assert.equal(allowanceLine(free(2), world), 'The AI can write 1 of 3 more strategy for you in Prisoner’s Dilemma · Each press of “Turn into rules” uses one · They never come back');
	assert.equal(allowanceLine(subscriber(5, 10), world), 'The AI can write 15 of 20 more strategies for you until 23 December 2026 · 10 strategies left in your packs · Each press of “Turn into rules” uses one');
	assert.equal(allowanceLine(free(3, 4), world), 'Your plan’s own are used up in Prisoner’s Dilemma · 4 strategies left in your packs · Each press of “Turn into rules” uses one');
	assert.equal(allowanceLine(free(3), world), 'The AI has written all 3 strategies your plan gives you in Prisoner’s Dilemma. They do not come back · You can still build the rules by hand, or write the strategy in code');
	assert.equal(allowanceLine(subscriber(20), world), 'The AI has written all 20 strategies your plan gives you this month. They come back on 23 December 2026 · You can still build the rules by hand, or write the strategy in code');
	for (const usage of [free(0), free(3), free(3, 2), subscriber(0), subscriber(20), subscriber(20, 1)]) assert.ok(!/request/i.test(allowanceLine(usage, world)));
});
