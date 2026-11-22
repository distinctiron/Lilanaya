// What every UI test starts from: a visitor with an address of their own, and helpers to
// sign people up, read their email and prepare strategies and runs without clicking through the site.
import { test as base, expect, request as playwrightRequest } from '@playwright/test';

const MAIL_URL = process.env.MAIL_URL ?? 'http://localhost:8125';
const AI_URL = process.env.AI_URL ?? 'http://localhost:8090';

let counter = 0;
/** A short name nobody else in this test run has. */
export const unique = (prefix = 't') => `${prefix}${Date.now().toString(36)}${process.pid.toString(36)}${(counter++).toString(36)}`;

/**
 * An address of the visitor's own. The site limits requests per visitor address, and all tests come from
 * one container; nginx trusts the forwarded address from inside the Docker network (as it does behind
 * Render's proxy), so each test says it is someone else.
 */
function visitorAddress() {
	const n = (Date.now() + process.pid * 7919 + counter++ * 104729) % (254 * 254 * 254);
	return `203.${1 + (n % 254)}.${1 + (Math.floor(n / 254) % 254)}.${1 + (Math.floor(n / 254 / 254) % 254)}`;
}

/** The emails Mailpit has caught for `address`, newest first: [{ subject, text }]. */
async function emailsTo(address) {
	const mail = await playwrightRequest.newContext();
	try {
		const found = await (await mail.get(`${MAIL_URL}/api/v1/search`, { params: { query: `to:"${address}"` } })).json();
		const messages = [];
		for (const m of found.messages ?? []) {
			const full = await (await mail.get(`${MAIL_URL}/api/v1/message/${m.ID}`)).json();
			messages.push({ subject: full.Subject, text: full.Text });
		}
		return messages;
	} finally {
		await mail.dispose();
	}
}

/** Waits for the next email to `address` (after the `seen` it already had) and returns it. */
async function nextEmail(address, seen = 0) {
	let messages = [];
	await expect
		.poll(
			async () => {
				messages = await emailsTo(address);
				return messages.length;
			},
			{ message: `an email to ${address}`, timeout: 15_000 }
		)
		.toBeGreaterThan(seen);
	return messages[0];
}

/** The path of the sign-in link in an email (`/log-in/confirm?token=…`). */
function linkIn(email) {
	const found = email.text.match(/https?:\/\/[^/\s]+(\/log-in\/confirm\?token=[0-9a-f]+)/);
	if (!found) throw new Error(`The email "${email.subject}" holds no sign-in link:\n${email.text}`);
	return found[1];
}

/** What the stand-in AI gateway has been sent so far: `{ screened: [text], asked: [text] }`. */
async function aiCalls() {
	const ai = await playwrightRequest.newContext();
	try {
		return await (await ai.get(`${AI_URL}/__calls`)).json();
	} finally {
		await ai.dispose();
	}
}

/**
 * Signs `address` up through the API (ask for a link, read the email, confirm) in the browser context
 * `context`, which then holds the session cookie. Returns the user as the API describes them.
 */
async function signUpInContext(context, address) {
	const asked = await context.request.post('/api/auth/email', { data: { email: address, intent: 'sign_up' } });
	expect(asked.status(), await asked.text()).toBe(202);
	const token = linkIn(await nextEmail(address)).split('token=')[1];
	const confirmed = await context.request.post('/api/auth/email/confirm', { data: { token } });
	expect(confirmed.status(), await confirmed.text()).toBe(200);
	return confirmed.json();
}

/** Calls the API as the user of `context`; fails the test, with the API's answer, when the status is not the one expected. */
function apiOf(context) {
	const call = async (method, path, data, expected) => {
		const res = await context.request.fetch(path, { method, data });
		expect(res.status(), `${method} ${path}: ${await res.text()}`).toBe(expected);
		return res.json();
	};
	const api = {
		get: (path) => call('GET', path, undefined, 200),
		/** Saves a strategy; `extra` overrides the fields of a small grudger. */
		strategy: (extra = {}) =>
			call(
				'POST',
				'/api/strategies',
				{
					name: 'Grudger',
					description: 'Cooperates until betrayed, then never forgives.',
					mode: 'rules',
					states: ['NICE', 'GRIM'],
					initial_state: 'NICE',
					action_map: { NICE: 'C', GRIM: 'D' },
					transitions: [{ from: 'NICE', to: 'GRIM', guard: { op: 'last_opponent_action', is: 'D' } }],
					...extra
				},
				201
			),
		code: (name, source, extra = {}) => call('POST', '/api/strategies', { name, mode: 'code', source, ...extra }, 201),
		/** Starts a run and waits for the worker to finish it. Returns the finished run. */
		run: async (extra = {}) => {
			const queued = await call('POST', '/api/runs', { world_id: 'prisoners-dilemma', iterations: 30, ...extra }, 202);
			let run = queued;
			await expect
				.poll(
					async () => {
						run = await api.get(`/api/runs/${queued.id}`);
						return run.status;
					},
					{ message: `run ${queued.id} to finish`, timeout: 30_000 }
				)
				.toMatch(/completed|failed/);
			return run;
		}
	};
	return api;
}

export const test = base.extend({
	// Every browser context of a test comes from one visitor address, unlike any other test's.
	extraHTTPHeaders: async ({}, use) => {
		await use({ 'X-Forwarded-For': visitorAddress() });
	},

	/** Reading the caught email. */
	mail: async ({}, use) => {
		await use({ next: nextEmail, all: emailsTo, linkIn });
	},

	/** What the stand-in AI gateway was sent. */
	ai: async ({}, use) => {
		await use({ calls: aiCalls });
	},

	/** A new address for someone who has no account yet. */
	address: async ({}, use) => {
		await use(`${unique('visitor-')}@example.com`);
	},

	/**
	 * The page's visitor, signed up and logged in before the test starts: `{ user, email, api }`.
	 * Asking for this fixture is what logs the test's `page` in.
	 */
	account: async ({ context, address }, use) => {
		const user = await signUpInContext(context, address);
		await use({ user, email: address, api: apiOf(context) });
	},

	/**
	 * Other people: `await someone()` gives `{ page, context, user, email, api }` for a second logged-in
	 * visitor in a browser of their own, and `await someone({ signedIn: false })` a visitor without an account.
	 */
	someone: async ({ browser, contextOptions }, use) => {
		const contexts = [];
		await use(async ({ signedIn = true } = {}) => {
			const context = await browser.newContext({ ...contextOptions, extraHTTPHeaders: { 'X-Forwarded-For': visitorAddress() } });
			contexts.push(context);
			const email = `${unique('other-')}@example.com`;
			const user = signedIn ? await signUpInContext(context, email) : null;
			return { page: await context.newPage(), context, user, email, api: apiOf(context) };
		});
		for (const context of contexts) await context.close();
	},

	// Any error the page itself throws, and anything the browser refuses under the content policy, fails the test.
	page: async ({ page }, use, testInfo) => {
		const problems = [];
		page.on('pageerror', (error) => problems.push(`The page threw: ${error.message}`));
		page.on('console', (message) => {
			if (message.type() === 'error' && /Content Security Policy|Refused to/i.test(message.text())) problems.push(`The browser refused: ${message.text()}`);
		});
		await use(page);
		// Only for a test that got to its end: after a failure Playwright takes a screenshot, and to do that in
		// Safari's engine it adds a style of its own, which the site's content policy rightly refuses.
		if (testInfo.status === testInfo.expectedStatus) expect(problems, 'problems the page ran into').toEqual([]);
	}
});

export { expect };

/** The heading of the page on show (the router moves focus to it after every navigation). */
export const title = (page) => page.getByRole('heading', { level: 1 });

/** Opens `path` and waits until the page for it has been drawn. */
export async function open(page, path, heading) {
	await page.goto(path);
	if (heading) await expect(title(page)).toHaveText(heading);
	else await expect(page.locator('#main .meta', { hasText: 'Loading…' })).toHaveCount(0);
}

/** A code strategy that plays well for the 20 rounds it is checked with and breaks in round 26 of a run. */
export const BREAKS_IN_A_RUN = "function decide(input) { if (input.roundIndex === 25) throw new Error('boom'); return 'C'; }";
