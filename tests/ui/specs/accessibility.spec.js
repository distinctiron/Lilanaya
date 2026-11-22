// Accessibility: automatic checks of every page (axe), and what a keyboard and a screen reader rely on.
import AxeBuilder from '@axe-core/playwright';
import { expect, open, test, title } from '../fixtures.js';

/** Problems axe rates serious or critical on the page as it stands. */
async function seriousProblems(page) {
	const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
	return violations
		.filter((v) => v.impact === 'serious' || v.impact === 'critical')
		.map((v) => `${v.id} (${v.impact}): ${v.help}\n${v.nodes.slice(0, 5).map((n) => `    ${n.target.join(' ')}  ${n.failureSummary?.split('\n').slice(1).join(' ').trim() ?? ''}`).join('\n')}`);
}

const PUBLIC_PAGES = [
	['/', null],
	['/how-it-works', 'How it works'],
	['/worlds', 'Worlds'],
	['/worlds/prisoners-dilemma', "Repeated Prisoner's Dilemma"],
	['/strategies', 'Strategies'],
	['/runs', 'Runs'],
	['/sign-up', 'Sign up'],
	['/log-in', 'Log in'],
	['/log-in/confirm', 'This link is not complete'],
	['/profile', 'Log in first'],
	['/no/such/page', 'Game over: page not found']
];

for (const [path, heading] of PUBLIC_PAGES) {
	test(`${path} has no serious accessibility problems for a visitor`, async ({ page }) => {
		await open(page, path, heading);
		expect(await seriousProblems(page)).toEqual([]);
	});
}

test('the pages behind a log-in have no serious accessibility problems', async ({ page, account }) => {
	// Seven pages, each read through by axe: allow it three times the usual time.
	test.slow();
	const mine = await account.api.strategy({ visibility: 'public' });
	const run = await account.api.run({ actors: [mine.id, 'always_defect', 'tit_for_tat'], iterations: 30 });
	for (const [path, heading] of [
		['/profile', 'Profile'],
		['/strategies', 'Strategies'],
		['/strategies/new', 'New strategy'],
		['/strategies/new/code', 'New code strategy'],
		[`/strategies/${mine.id}/edit`, `Edit ${mine.name}`],
		['/runs/new', 'New run'],
		[`/runs/${run.id}?a=${mine.id}&b=always_defect`, `Run ${run.id}`]
	]) {
		await open(page, path, heading);
		expect(await seriousProblems(page), path).toEqual([]);
	}
});

test('the rules editor with everything on show has no serious accessibility problems', async ({ page, account }) => {
	// Rules open, a preview, and a flag from the AI.
	await open(page, '/strategies/new/rules', 'New rules strategy');
	await page.getByLabel('Describe your strategy in plain English').fill('Cooperate until betrayed, but flip a coin every tenth round.');
	await page.getByRole('button', { name: 'Turn into rules' }).click();
	await page.getByRole('button', { name: 'Continue with these rules' }).click();
	await page.locator('.card.fold summary', { hasText: 'Rules' }).click();
	await page.getByRole('button', { name: 'Check and preview' }).click();
	await expect(page.getByText('The strategy is valid.')).toBeVisible();
	expect(await seriousProblems(page), 'the rules editor').toEqual([]);
});

test('the first Tab stop is a way past the header, and every page has one main heading and one main area', async ({ page }) => {
	await open(page, '/worlds', 'Worlds');
	await page.keyboard.press('Tab');
	const skip = page.getByRole('link', { name: 'Skip to content' });
	await expect(skip).toBeFocused();
	await expect(skip).toBeVisible();
	await page.keyboard.press('Enter');
	// It lands on the page's heading, past the header and the menu.
	await expect(title(page)).toBeFocused();
	for (const [path, heading] of PUBLIC_PAGES) {
		await open(page, path, heading);
		await expect(page.getByRole('heading', { level: 1 }), path).toHaveCount(1);
		await expect(page.getByRole('main'), path).toHaveCount(1);
		await expect(page.locator('html'), path).toHaveAttribute('lang', 'en');
		expect(await page.title(), path).toMatch(/ · Lilanaya$/);
	}
});

test('moving between pages moves the focus to the new page’s heading and marks where the visitor is', async ({ page }) => {
	await open(page, '/worlds', 'Worlds');
	const nav = page.getByRole('navigation', { name: 'Main' });
	await expect(nav.getByRole('link', { name: 'Worlds' })).toHaveAttribute('aria-current', 'page');
	await nav.getByRole('link', { name: 'Strategies' }).click();
	await expect(title(page)).toHaveText('Strategies');
	await expect(title(page)).toBeFocused();
	await expect(page).toHaveTitle('Strategies · Lilanaya');
	await expect(nav.getByRole('link', { name: 'Strategies' })).toHaveAttribute('aria-current', 'page');
	await expect(nav.locator('[aria-current="page"]')).toHaveCount(1);
	// Back and forward work like on any site.
	await page.goBack();
	await expect(title(page)).toHaveText('Worlds');
	await page.goForward();
	await expect(title(page)).toHaveText('Strategies');
	// A reload of a deep address shows the same page.
	await page.goto('/worlds/prisoners-dilemma');
	await page.reload();
	await expect(title(page)).toHaveText("Repeated Prisoner's Dilemma");
});

test('a run can be started and a match opened with the keyboard alone', async ({ page, account }) => {
	await open(page, '/runs/new', 'New run');
	const rounds = page.getByLabel('Rounds per match');
	await rounds.focus();
	await page.keyboard.press('ControlOrMeta+a');
	await page.keyboard.type('30');
	// Enter in a field sends the form.
	await page.keyboard.press('Enter');
	await expect(page.getByRole('heading', { name: 'High scores' })).toBeVisible({ timeout: 30_000 });
	await expect(title(page)).toBeFocused();

	const cell = page.locator('table.matrix').getByRole('link', { name: /^Tit for Tat scored .* against Always Defect/ });
	await cell.focus();
	await page.keyboard.press('Enter');
	await expect(page.getByRole('heading', { name: 'Match: Tit for Tat versus Always Defect' })).toBeVisible();
	// The rounds are described in words for someone who cannot see the squares.
	await expect(page.getByRole('img', { name: /^First 30 of 30 rounds\. Tit for Tat: 1 cooperate, 29 defect\./ })).toBeVisible();
});

test('errors are announced: they are alerts, and the field in question says it is invalid', async ({ page, account }) => {
	await page.route('**/api/runs', (route) =>
		route.request().method() === 'POST'
			? route.fulfill({ status: 422, json: { error: { code: 'validation_failed', message: 'The run request is not valid', details: [{ path: 'iterations', message: 'must be between 1 and 10000' }] } } })
			: route.continue()
	);
	await open(page, '/runs/new', 'New run');
	await page.getByRole('button', { name: 'Run simulation' }).click();
	const alert = page.getByRole('alert').filter({ hasText: 'The run request is not valid' });
	await expect(alert).toContainText('iterations: must be between 1 and 10000');
	await expect(page.getByLabel('Rounds per match')).toHaveAttribute('aria-invalid', 'true');
	await expect(page.getByLabel('Seed')).not.toHaveAttribute('aria-invalid', 'true');
});
