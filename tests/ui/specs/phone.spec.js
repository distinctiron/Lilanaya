// The site on an iPhone (Safari's engine, phone width): the menu, no sideways scrolling, and the main journeys.
import { expect, open, PASSWORD, test, title } from '../fixtures.js';

/** How far the page can be scrolled sideways, in pixels. None is right. */
const sideways = (page) => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);

test('the header is one line, with the links behind a Menu button', async ({ page }) => {
	await open(page, '/worlds', 'Worlds');
	const menu = page.getByRole('button', { name: 'Menu' });
	const nav = page.getByRole('navigation', { name: 'Main' });
	await expect(menu).toBeVisible();
	await expect(menu).toHaveAttribute('aria-expanded', 'false');
	await expect(nav).toBeHidden();
	expect((await page.locator('.site-header').boundingBox()).height).toBeLessThan(90);

	await menu.click();
	await expect(page.getByRole('button', { name: 'Close' })).toHaveAttribute('aria-expanded', 'true');
	await expect(nav.getByRole('link', { name: 'Strategies' })).toBeVisible();
	await expect(page.getByRole('link', { name: 'Log in' })).toBeVisible();
	await nav.getByRole('link', { name: 'Strategies' }).click();
	await expect(title(page)).toHaveText('Strategies');
	// A new page starts with the menu folded away again.
	await expect(page.getByRole('button', { name: 'Menu' })).toHaveAttribute('aria-expanded', 'false');
	await expect(nav).toBeHidden();

	await page.getByRole('button', { name: 'Menu' }).click();
	await page.getByRole('button', { name: 'Close' }).click();
	await expect(nav).toBeHidden();
});

test('no page scrolls sideways', async ({ page, account }) => {
	const mine = await account.api.strategy({ name: 'A strategy with quite a long name to wrap', visibility: 'public' });
	const run = await account.api.run({ actors: [mine.id, 'always_defect', 'tit_for_tat', 'always_cooperate', 'win_stay_lose_shift'], iterations: 30 });
	for (const path of [
		'/',
		'/how-it-works',
		'/worlds',
		'/worlds/prisoners-dilemma',
		'/strategies',
		'/strategies/new',
		'/strategies/new/rules',
		'/strategies/new/code',
		`/strategies/${mine.id}/edit`,
		'/runs',
		'/runs/new',
		`/runs/${run.id}`,
		`/runs/${run.id}?a=${mine.id}&b=always_defect`,
		'/profile',
		'/no/such/page'
	]) {
		await open(page, path);
		await expect(title(page)).toBeVisible();
		expect(await sideways(page), path).toBeLessThanOrEqual(1);
	}
	await page.context().clearCookies();
	for (const path of ['/sign-up', '/log-in', '/profile']) {
		await open(page, path);
		expect(await sideways(page), path).toBeLessThanOrEqual(1);
	}
});

test('the five rounds on the front page are played by touch', async ({ page }) => {
	await open(page, '/');
	await page.getByRole('button', { name: 'Play a round' }).tap();
	const game = page.locator('.hand-game');
	for (let round = 1; round <= 5; round++) {
		await expect(game).toContainText(`Round ${round} of 5. Your move:`);
		await game.getByRole('button', { name: 'Cooperate' }).tap();
	}
	await expect(game.locator('.bubble')).toContainText('15 each. I am Tit for Tat.');
	// The buttons are big enough for a thumb.
	const next = await game.getByRole('button', { name: 'See strategies play' }).boundingBox();
	expect(next.height).toBeGreaterThanOrEqual(44);
	expect(await sideways(page)).toBeLessThanOrEqual(1);
});

test('signing up, writing a strategy and running it, all on the phone', async ({ page, address, mail }) => {
	await open(page, '/sign-up', 'Sign up');
	await page.getByLabel('Email address').fill(address);
	await page.getByRole('button', { name: 'Sign up' }).tap();
	await expect(title(page)).toHaveText('Check your email');
	await page.goto(mail.linkIn(await mail.next(address)));
	await page.getByLabel('Password', { exact: true }).fill(PASSWORD);
	await page.getByRole('button', { name: 'Continue' }).tap();
	await expect(title(page)).toHaveText('Profile');

	// A strategy from a description.
	await open(page, '/strategies/new/rules', 'New rules strategy');
	await page.getByLabel('Describe your strategy in plain English').fill('Cooperate until the opponent defects once, then defect for ever.');
	await page.getByRole('button', { name: 'Turn into rules' }).tap();
	await expect(page.getByLabel('Name', { exact: true })).toHaveValue('Grim Trigger');
	await page.getByRole('button', { name: 'Next: preview and save' }).tap();
	await page.getByRole('button', { name: 'Check and preview' }).tap();
	await expect(page.locator('.preview-match')).toHaveCount(2);
	await page.getByRole('button', { name: 'Create strategy' }).tap();
	await expect(title(page)).toHaveText('Your new competitor');
	await page.getByRole('link', { name: 'Send it into a run' }).tap();

	// The run form is two screens on a phone: who plays, then the settings.
	await expect(title(page)).toHaveText('New run');
	await page.getByRole('checkbox', { name: /Grim Trigger/ }).check();
	await page.getByRole('button', { name: 'Next: settings' }).tap();
	await expect(page.getByLabel('Rounds per match')).toBeInViewport();
	await page.getByLabel('Rounds per match').fill('40');
	await page.getByRole('button', { name: 'Run simulation' }).tap();
	await expect(page.getByRole('heading', { name: 'High scores' })).toBeVisible({ timeout: 30_000 });
	await expect(page.locator('table.bars tbody tr')).toHaveCount(5);
	await expect(page.locator('table.bars')).toContainText('Grim Trigger');

	// A match, picked from the head-to-head table.
	await page.locator('table.matrix').getByRole('link', { name: /^Grim Trigger scored .* against Always Defect/ }).tap();
	await expect(page.getByRole('heading', { name: 'Match: Grim Trigger versus Always Defect' })).toBeVisible();
	await expect(page.locator('.match .match-score')).toHaveText(['39', '44']);
	expect(await sideways(page)).toBeLessThanOrEqual(1);

	// And out again, through the menu.
	await page.getByRole('button', { name: 'Menu' }).tap();
	await page.getByRole('link', { name: /^Profile of / }).tap();
	await page.getByRole('button', { name: 'Log out' }).tap();
	await expect(title(page)).toHaveText('Worlds');
});

// Names may be 80 characters and a public strategy is listed for everyone, so one user's name must not
// be able to push other people's pages sideways.
test('a name of one long word does not push a page sideways', async ({ page, account }) => {
	const mine = await account.api.strategy({ name: 'Supercalifragilisticexpialidocious_and_then_some_more', visibility: 'public' });
	const run = await account.api.run({ actors: [mine.id, 'always_defect'], iterations: 30 });
	for (const path of ['/strategies', `/strategies/${mine.id}/edit`, '/runs/new', `/runs/${run.id}`, `/runs/${run.id}?a=${mine.id}&b=always_defect`, '/runs']) {
		await open(page, path);
		await expect(title(page)).toBeVisible();
		expect(await sideways(page), path).toBeLessThanOrEqual(1);
	}
});
