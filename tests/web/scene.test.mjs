// scene.js: the clock the films run on. Pausing must freeze an animation exactly where it is.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { clock } from '../../src/app/web/lilanaya-client/public/components/scene.js';

/** Fake time: `setTimeout`, `clearTimeout` and `performance.now` all move only when the test says so. */
function time(t) {
	t.mock.timers.enable({ apis: ['setTimeout', 'Date'] });
	let now = 0;
	t.mock.method(performance, 'now', () => now);
	return async (ms) => {
		now += ms;
		t.mock.timers.tick(ms);
		// Let the promises that just resolved run.
		await new Promise((resolve) => setImmediate(resolve));
	};
}

/** Follows a wait: `done` turns true when it has resolved. */
function watch(promise) {
	const state = { done: false };
	promise.then(() => (state.done = true));
	return state;
}

test('a stopped clock lets no time pass', async (t) => {
	const pass = time(t);
	const c = clock();
	const wait = watch(c.wait(100));
	await pass(10_000);
	assert.equal(wait.done, false, 'the clock starts stopped');
	c.run(true);
	await pass(99);
	assert.equal(wait.done, false);
	await pass(1);
	assert.equal(wait.done, true);
});

test('pausing keeps what was left of every wait', async (t) => {
	const pass = time(t);
	const c = clock();
	c.run(true);
	const [short, long] = [watch(c.wait(100)), watch(c.wait(300))];
	await pass(60);
	c.run(false);
	await pass(5_000);
	assert.deepEqual([short.done, long.done], [false, false], 'frozen while paused');
	c.run(true);
	await pass(39);
	assert.equal(short.done, false, '40 ms were left, 39 have passed');
	await pass(1);
	assert.deepEqual([short.done, long.done], [true, false]);
	await pass(200);
	assert.equal(long.done, true);
});

test('pausing and resuming many times never loses or gains time', async (t) => {
	const pass = time(t);
	const c = clock();
	const wait = watch(c.wait(100));
	for (let i = 0; i < 9; i++) {
		c.run(true);
		await pass(10);
		c.run(false);
		await pass(1000);
	}
	assert.equal(wait.done, false, '90 ms of running time so far');
	c.run(true);
	c.run(true);
	await pass(10);
	assert.equal(wait.done, true);
});

test('a wait asked for while running starts at once; one of no time still waits its turn', async (t) => {
	const pass = time(t);
	const c = clock();
	c.run(true);
	const zero = watch(c.wait(0));
	assert.equal(zero.done, false);
	await pass(0);
	assert.equal(zero.done, true);
});

test('dropping the clock abandons every wait for good', async (t) => {
	const pass = time(t);
	const c = clock();
	c.run(true);
	const wait = watch(c.wait(100));
	await pass(50);
	c.drop();
	await pass(10_000);
	c.run(true);
	await pass(10_000);
	assert.equal(wait.done, false, 'a film that was left never carries on');
	// The clock itself can be used again.
	const next = watch(c.wait(10));
	await pass(10);
	assert.equal(next.done, true);
});
