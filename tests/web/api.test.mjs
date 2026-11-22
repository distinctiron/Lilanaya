// api.js: how the front end talks to the API and what it makes of every kind of answer.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { ApiError, deleteJson, getJson, patchJson, postJson, putJson } from '../../src/app/web/lilanaya-client/public/api.js';

const realFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = realFetch;
});

/** Makes `fetch` answer with `status` and `body` (text), and records what it was called with. */
function answer(status, body) {
	const calls = [];
	globalThis.fetch = async (path, options) => {
		calls.push({ path, ...options });
		return new Response(body, { status });
	};
	return calls;
}

test('a GET sends no body and says it wants JSON', async () => {
	const calls = answer(200, '{"worlds":[]}');
	assert.deepEqual(await getJson('/api/worlds'), { worlds: [] });
	assert.deepEqual(calls, [{ path: '/api/worlds', method: 'GET', headers: { Accept: 'application/json' }, body: undefined }]);
});

test('every writing method sends its body as JSON', async () => {
	for (const [send, method] of [
		[postJson, 'POST'],
		[putJson, 'PUT'],
		[patchJson, 'PATCH']
	]) {
		const calls = answer(200, '{}');
		await send('/api/thing', { name: 'Grüdger', n: 1 });
		assert.equal(calls[0].method, method);
		assert.equal(calls[0].headers['Content-Type'], 'application/json');
		assert.deepEqual(JSON.parse(calls[0].body), { name: 'Grüdger', n: 1 });
	}
	const calls = answer(200, '{"deleted":"x"}');
	assert.deepEqual(await deleteJson('/api/strategies/x'), { deleted: 'x' });
	assert.equal(calls[0].method, 'DELETE');
	assert.equal(calls[0].body, undefined);
});

test('a POST without a body (sign out) sends none', async () => {
	const calls = answer(200, '{"signed_out":true}');
	await postJson('/api/auth/sign-out');
	assert.equal(calls[0].body, undefined);
	assert.equal(calls[0].headers['Content-Type'], undefined);
});

test('addresses are relative, so they go through the same site', async () => {
	const calls = answer(200, '{}');
	await getJson('/api/runs/7');
	assert.equal(calls[0].path, '/api/runs/7');
});

test("an error answer becomes an ApiError with the API's code, message and details", async () => {
	answer(422, JSON.stringify({ error: { code: 'validation_failed', message: 'The run request is not valid', details: [{ path: 'iterations', message: 'must be between 1 and 10000' }] } }));
	await assert.rejects(postJson('/api/runs', {}), (error) => {
		assert.ok(error instanceof ApiError && error instanceof Error);
		assert.equal(error.name, 'ApiError');
		assert.equal(error.status, 422);
		assert.equal(error.code, 'validation_failed');
		assert.equal(error.message, 'The run request is not valid');
		assert.deepEqual(error.details, [{ path: 'iterations', message: 'must be between 1 and 10000' }]);
		return true;
	});
});

test('an error without details has an empty list, so pages can always loop over it', async () => {
	answer(404, JSON.stringify({ error: { code: 'not_found', message: 'Run "9" was not found' } }));
	await assert.rejects(getJson('/api/runs/9'), (error) => error.status === 404 && error.code === 'not_found' && Array.isArray(error.details) && error.details.length === 0);
});

test('an error page that is not JSON (a proxy, a rate limit) still becomes a readable ApiError', async () => {
	for (const [status, body] of [
		[502, '<html><body>Bad Gateway</body></html>'],
		[429, ''],
		[500, 'null'],
		[503, '{"unexpected":"shape"}']
	]) {
		answer(status, body);
		await assert.rejects(getJson('/api/worlds'), (error) => {
			assert.equal(error.name, 'ApiError');
			assert.equal(error.status, status);
			assert.equal(error.code, 'http_error');
			assert.equal(error.message, `The server answered ${status}.`);
			assert.deepEqual(error.details, []);
			return true;
		});
	}
});

test('a server that cannot be reached is an ApiError too, not a crash', async () => {
	globalThis.fetch = async () => {
		throw new TypeError('fetch failed');
	};
	await assert.rejects(getJson('/api/worlds'), (error) => error.name === 'ApiError' && error.status === 0 && error.code === 'network' && error.message === 'The server could not be reached.');
});

test('a good answer that is not JSON gives null instead of throwing', async () => {
	answer(200, '');
	assert.equal(await getJson('/api/odd'), null);
});
