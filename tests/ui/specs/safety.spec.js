// @safety What the deployed site must hold to whatever is thrown at it: headers, limits at the edge,
// text that tries to be markup, and where a visitor can be sent.
import { request as playwrightRequest } from '@playwright/test';
import { expect, open, test, title, unique } from '../fixtures.js';

const CSP = "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'";

test('@safety every answer carries the security headers, pages and API alike', async ({ page }) => {
	for (const path of ['/', '/runs/5', '/no/such/page', '/style.css', '/main.js', '/robots.txt', '/api/health', '/api/worlds', '/api/no-such-route', '/api/me']) {
		const answer = await page.request.get(path);
		const headers = answer.headers();
		expect(headers['content-security-policy'], path).toBe(CSP);
		expect(headers['x-content-type-options'], path).toBe('nosniff');
		expect(headers['x-frame-options'], path).toBe('DENY');
		expect(headers['referrer-policy'], path).toBe('no-referrer');
		expect(headers['strict-transport-security'], path).toBe('max-age=31536000');
		// The web server says what it is, not which version.
		expect(headers['server'], path).toBe('nginx');
	}
	// The same on an answer the web server makes itself.
	const tooBig = await page.request.post('/api/strategies/preview', { data: { strategy: { mode: 'code', source: '/'.repeat(200 * 1024) } } });
	expect(tooBig.status()).toBe(413);
	expect(tooBig.headers()['x-frame-options']).toBe('DENY');
});

test('@safety the site loads nothing from anywhere else, and the browser has nothing to refuse', async ({ page, account }) => {
	const base = new URL(page.url() === 'about:blank' ? test.info().project.use.baseURL : page.url()).origin;
	const elsewhere = [];
	page.on('request', (request) => {
		const url = request.url();
		if (!url.startsWith(base) && !url.startsWith('data:') && !url.startsWith('blob:')) elsewhere.push(url);
	});
	const run = await account.api.run({ iterations: 20 });
	for (const path of ['/', '/how-it-works', '/worlds', '/worlds/prisoners-dilemma', '/strategies', '/strategies/new/rules', '/strategies/new/code', '/runs/new', '/runs', `/runs/${run.id}?a=tit_for_tat&b=always_defect`, '/profile', '/sign-up']) {
		await open(page, path);
		await expect(title(page)).toBeVisible();
	}
	expect(elsewhere).toEqual([]);
	// The page fixture fails the test if the content policy refused anything, such as an inline style or script.
});

test('@safety text that tries to be markup is shown as the text it is, wherever it turns up', async ({ page, account, someone }) => {
	// Each would set `window.p` if the browser ever took it for markup. (Short words: see the long-name test in phone.spec.js.)
	const tag = unique('x');
	const name = `<img src=x onerror=p=1> ${tag}`;
	const description = '<script>p=1</script> <b>bold</b>';
	const strategy = await account.api.strategy({ name, description, visibility: 'public', states: ['<i>NICE</i>', 'GRIM'], initial_state: '<i>NICE</i>', action_map: { '<i>NICE</i>': 'C', GRIM: 'D' }, transitions: [{ from: '<i>NICE</i>', to: 'GRIM', guard: { op: 'last_opponent_action', is: 'D' } }] });
	const run = await account.api.run({ actors: [strategy.id, 'always_defect'], iterations: 20 });
	const renamed = await page.request.patch('/api/me', { data: { display_name: '<svg onload=p=1>' } });
	expect(renamed.status()).toBe(200);
	const other = await someone();

	const clean = async (who, where) => {
		expect(await who.evaluate(() => window.p), where).toBeUndefined();
		// Nothing typed by a user became an element.
		expect(await who.locator('#main img, #main script, #main b, #main i, #header svg[onload]').count(), where).toBe(0);
	};
	for (const who of [page, other.page]) {
		await open(who, '/strategies', 'Strategies');
		const card = who.locator('.card.fold', { hasText: tag });
		await expect(card.getByRole('heading')).toHaveText(name);
		await card.locator('summary').click();
		await expect(card).toContainText(description);
		await expect(card).toContainText('<i>NICE</i> (start)');
		await clean(who, 'the strategies list');

		await open(who, '/runs/new', 'New run');
		await expect(who.locator('label.check', { hasText: tag })).toContainText(name);
		await clean(who, 'the run form');

		await open(who, `/runs/${run.id}?a=${strategy.id}&b=always_defect`, `Run ${run.id}`);
		await expect(who.getByRole('heading', { name: `Match: ${name} versus Always Defect` })).toBeVisible();
		await expect(who.locator('table.bars')).toContainText(name);
		await clean(who, 'the results');

		await open(who, '/runs', 'Runs');
		await clean(who, 'the runs list');
	}
	// The author's display name, in their own header and next to their strategy for others.
	await expect(page.locator('#header .btn-name')).toHaveText('<svg onload=p=1>');
	await open(other.page, '/strategies', 'Strategies');
	await expect(other.page.locator('.card.fold', { hasText: tag })).toContainText('By <svg onload=p=1>');
	await clean(other.page, 'the author name');
	// And in the editor, where it goes into form fields.
	await open(page, `/strategies/${strategy.id}/edit`);
	await expect(page.getByLabel('Name', { exact: true })).toHaveValue(name);
	await expect(page.getByLabel('Description', { exact: true })).toHaveValue(description);
	await clean(page, 'the editor');
});

test('@safety the session cannot be read by scripts on the page, and goes to this site only', async ({ page, account }) => {
	await open(page, '/profile', 'Profile');
	expect(await page.evaluate(() => document.cookie)).not.toContain('lilanaya_session');
	const [cookie] = (await page.context().cookies()).filter((c) => c.name === 'lilanaya_session');
	expect(cookie).toMatchObject({ httpOnly: true, sameSite: 'Lax', path: '/', domain: new URL(page.url()).hostname });
	expect(cookie.value).toMatch(/^[0-9a-f]{64}$/);
	// About thirty days.
	expect(cookie.expires - Date.now() / 1000).toBeGreaterThan(29 * 24 * 3600);
	expect(cookie.expires - Date.now() / 1000).toBeLessThan(31 * 24 * 3600);
	// Nothing about the user is kept where scripts can read it.
	const stored = await page.evaluate(() => JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }));
	expect(stored).not.toContain(account.email);
	expect(stored).not.toContain(cookie.value);
});

test('@safety a link meant to send the visitor elsewhere after logging in leads to the profile instead', async ({ page, address, mail }) => {
	for (const [i, next] of ['//evil.example/', 'https://evil.example/', 'javascript:alert(1)'].entries()) {
		const who = i === 0 ? address : `${unique('redirect-')}@example.com`;
		await page.context().clearCookies();
		await open(page, `/sign-up?next=${encodeURIComponent(next)}`, 'Sign up');
		// The offer to log in instead does not carry it on either.
		await expect(page.locator('#main').getByRole('link', { name: 'Log in' })).toHaveAttribute('href', '/log-in');
		await page.getByLabel('Email address').fill(who);
		await page.getByRole('button', { name: 'Sign up' }).click();
		await expect(title(page)).toHaveText('Check your email');
		await page.goto(mail.linkIn(await mail.next(who)));
		await page.getByRole('button', { name: 'Continue' }).click();
		await expect(title(page)).toHaveText('Profile');
		expect(new URL(page.url()).pathname, next).toBe('/profile');
	}
});

test('@safety one visitor asking for sign-in links again and again is slowed down, and nobody else is', async ({ page, baseURL }) => {
	const from = (address) => playwrightRequest.newContext({ baseURL, extraHTTPHeaders: { 'X-Forwarded-For': address } });
	const eager = await from('198.51.100.77');
	const statuses = [];
	for (let i = 0; i < 30; i++) statuses.push((await eager.post('/api/auth/email', { data: { email: `${unique('flood-')}@example.com` } })).status());
	const sent = statuses.filter((s) => s === 202).length;
	const refused = statuses.filter((s) => s === 429).length;
	// Ten a minute with a burst of ten: at most about twenty get through, the rest are refused.
	expect(sent + refused).toBe(30);
	expect(sent).toBeGreaterThanOrEqual(10);
	expect(sent).toBeLessThanOrEqual(21);
	expect(refused).toBeGreaterThanOrEqual(9);
	// The rest of the site still answers that visitor.
	expect((await eager.get('/api/worlds')).status()).toBe(200);
	expect((await eager.get('/')).status()).toBe(200);
	await eager.dispose();

	// Someone at another address is not held up, and is told plainly if they ever are.
	const calm = await from('198.51.100.78');
	expect((await calm.post('/api/auth/email', { data: { email: `${unique('calm-')}@example.com` } })).status()).toBe(202);
	await calm.dispose();

	await page.route('**/api/auth/email', (route) => route.fulfill({ status: 429, contentType: 'text/html', body: '<html><body>429 Too Many Requests</body></html>' }));
	await open(page, '/sign-up', 'Sign up');
	await page.getByLabel('Email address').fill('someone@example.com');
	await page.getByRole('button', { name: 'Sign up' }).click();
	await expect(page.getByRole('alert')).toHaveText('The server answered 429.');
	await expect(page.getByRole('button', { name: 'Sign up' })).toBeEnabled();
});

test('@safety only the site’s own files are served: nothing of the server’s can be asked for', async ({ page }) => {
	const index = await (await page.request.get('/')).text();
	// Anything that is not a file of the site gets the site's own page (which then says "not found"), never a file of the server.
	for (const path of ['/.env', '/.git/config', '/Cargo.toml', '/compose.yaml', '/default.conf.template', '/security-headers.conf', '/etc/passwd', '/..%2f..%2fetc%2fpasswd', '/%2e%2e/%2e%2e/etc/passwd', '/api../etc/passwd', '/fonts/../../etc/passwd']) {
		const answer = await page.request.get(path, { maxRedirects: 0 });
		const body = await answer.text();
		expect([200, 400, 404], path).toContain(answer.status());
		expect(body, path).not.toMatch(/root:|AI_GATEWAY|DATABASE_URL|POSTGRES|proxy_pass|\[workspace\]/);
		if (answer.status() === 200) expect(body, path).toBe(index);
	}
	// A folder is not listed.
	const fonts = await page.request.get('/fonts/');
	expect(await fonts.text()).not.toContain('Index of');
	// What is meant to be public is.
	expect((await page.request.get('/robots.txt')).status()).toBe(200);
	const css = await page.request.get('/style.css');
	expect(css.headers()['content-type']).toContain('text/css');
});

test('@safety an address that is no page says so, and an unknown API route answers in the API’s own shape', async ({ page }) => {
	await open(page, '/no/such/page', 'Game over: page not found');
	await expect(page).toHaveTitle('Not found · Lilanaya');
	await page.getByRole('link', { name: 'Back to worlds' }).click();
	await expect(title(page)).toHaveText('Worlds');

	const answer = await page.request.get('/api/no-such-route');
	expect(answer.status()).toBe(404);
	expect(answer.headers()['content-type']).toBe('application/json');
	expect(await answer.json()).toEqual({ error: { code: 'not_found', message: 'This route was not found' } });
	const wrongMethod = await page.request.delete('/api/worlds');
	expect(wrongMethod.status()).toBe(405);
	expect((await wrongMethod.json()).error.code).toBe('method_not_allowed');
});

test('@safety a server that stops answering is reported on the page, with a way to try again', async ({ page }) => {
	let down = true;
	await page.route('**/api/worlds', (route) => (down ? route.abort('connectionrefused') : route.continue()));
	await page.goto('/worlds');
	await expect(title(page)).toHaveText('Something went wrong');
	await expect(page.getByRole('alert')).toContainText('The server could not be reached.');
	down = false;
	await page.getByRole('button', { name: 'Try again' }).click();
	await expect(title(page)).toHaveText('Worlds');
});

test('@safety an answer that is an error page from in between, not from the API, is still explained', async ({ page, account }) => {
	await page.route('**/api/runs', (route) => (route.request().method() === 'POST' ? route.fulfill({ status: 502, contentType: 'text/html', body: '<html>Bad Gateway</html>' }) : route.continue()));
	await open(page, '/runs/new', 'New run');
	await page.getByRole('button', { name: 'Run simulation' }).click();
	await expect(page.getByRole('alert').filter({ hasText: 'The server answered' })).toHaveText('The server answered 502.');
	await expect(page.getByRole('button', { name: 'Run simulation' })).toBeEnabled();
});
