// session.js: who is signed in, asked once and shared by every page.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';

const realFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = realFetch;
});

/** A fresh copy of the module: its memory of who is signed in starts empty. */
let copies = 0;
const fresh = () => import(`../../src/app/web/lilanaya-client/public/session.js?copy=${copies++}`);

function answers(...list) {
	const calls = [];
	globalThis.fetch = async (path, options) => {
		calls.push(`${options.method} ${path}`);
		const next = list.shift();
		if (next instanceof Error) throw next;
		return new Response(JSON.stringify(next.body), { status: next.status ?? 200 });
	};
	return calls;
}

const ada = { id: 1, email: 'ada@example.com', display_name: 'Ada' };

test('the server is asked once, however many pages want to know', async () => {
	const session = await fresh();
	const calls = answers({ body: { user: ada } });
	assert.deepEqual(await session.getUser(), ada);
	assert.deepEqual(await session.getUser(), ada);
	assert.deepEqual(calls, ['GET /api/session']);
});

test('nobody signed in is null, and that is remembered too', async () => {
	const session = await fresh();
	const calls = answers({ body: { user: null } });
	assert.equal(await session.getUser(), null);
	assert.equal(await session.getUser(), null);
	assert.equal(calls.length, 1);
});

test('a server that is down or broken reads as signed out, without throwing', async () => {
	for (const failure of [new TypeError('fetch failed'), { status: 500, body: { error: { code: 'internal', message: 'x' } } }, { status: 429, body: null }]) {
		const session = await fresh();
		answers(failure);
		assert.equal(await session.getUser(), null);
	}
});

test('listeners hear every change: signing in, a refresh, signing out', async () => {
	const session = await fresh();
	const heard = [];
	session.onUserChange((user) => heard.push(user?.display_name ?? null));
	const calls = answers({ body: { user: ada } }, { body: { user: { ...ada, display_name: 'Ada L.' } } }, { body: { signed_out: true } });
	await session.getUser();
	await session.refreshUser();
	session.setUser({ ...ada, display_name: 'Countess' });
	await session.signOut();
	assert.deepEqual(heard, ['Ada', 'Ada L.', 'Countess', null]);
	assert.deepEqual(calls, ['GET /api/session', 'GET /api/session', 'POST /api/auth/sign-out']);
	assert.equal(await session.getUser(), null, 'signed out is remembered without asking again');
	assert.equal(calls.length, 3);
});

test('a failed sign-out leaves the user signed in', async () => {
	const session = await fresh();
	answers({ body: { user: ada } }, { status: 500, body: { error: { code: 'internal', message: 'x' } } });
	await session.getUser();
	await assert.rejects(session.signOut());
	assert.deepEqual(await session.getUser(), ada);
});

test('the way back after logging in is carried in the address, safely encoded', async () => {
	const session = await fresh();
	assert.equal(session.logInUrl('/runs/new?world=prisoners-dilemma&x=a b'), '/log-in?next=%2Fruns%2Fnew%3Fworld%3Dprisoners-dilemma%26x%3Da%20b');
	assert.equal(session.signUpUrl('/strategies/new/rules'), '/sign-up?next=%2Fstrategies%2Fnew%2Frules');
	// Whatever is in the path cannot add a second parameter of its own.
	assert.equal(new URLSearchParams(session.logInUrl('/a?next=//evil.example&error=x').split('?')[1]).get('next'), '/a?next=//evil.example&error=x');
});
