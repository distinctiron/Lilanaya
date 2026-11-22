// Worlds: the list, and one world's rules, points and standings.
import { expect, open, test, title } from '../fixtures.js';

test('the list shows the built-in world with its points and the way into a run', async ({ page }) => {
	await open(page, '/worlds', 'Worlds');
	const world = page.locator('.cards .card').first();
	await expect(world.getByRole('heading', { name: 'Repeated Prisoner’s Dilemma' }).or(world.getByRole('heading', { name: "Repeated Prisoner's Dilemma" }))).toBeVisible();
	await expect(world).toContainText('2 decisions · 1000 rounds per match');
	// The only world there is, so it is folded out.
	await expect(world.getByRole('group', { name: 'Payoff matrix' })).toBeVisible();
	await expect(world.getByRole('link', { name: 'Run simulation' })).toHaveAttribute('href', '/runs/new?world=prisoners-dilemma');
	await world.getByText('The story behind it').click();
	await expect(world).toContainText('two partners in crime are questioned in separate rooms');
	await world.getByRole('link', { name: 'View world' }).click();
	await expect(title(page)).toHaveText("Repeated Prisoner's Dilemma");
	await expect(page).toHaveURL(/\/worlds\/prisoners-dilemma$/);
});

test('a world shows its rules, its payoff matrix and who plays in it', async ({ page }) => {
	await open(page, '/worlds/prisoners-dilemma', "Repeated Prisoner's Dilemma");
	await expect(page).toHaveTitle("Repeated Prisoner's Dilemma · Lilanaya");
	const rules = page.locator('dl.rules').first();
	for (const [term, value] of [['Decisions', 'C, D'], ['Outcomes', 'C/C, C/D, D/C, D/D'], ['Rounds per match', '1000'], ['Default seed', '42'], ['Ranked by', 'Total payoff']]) {
		await expect(rules.locator('dt', { hasText: new RegExp(`^${term}$`) }).locator('+ dd')).toHaveText(value);
	}
	const matrix = page.getByRole('group', { name: 'Payoff matrix' });
	const row = (name) => matrix.getByRole('row', { name: new RegExp(name) }).getByRole('cell');
	await expect(row('You play C')).toHaveText(['3, 3', '0, 5']);
	await expect(row('You play D')).toHaveText(['5, 0', '1, 1']);

	await expect(page.locator('.actor-grid > li')).toHaveCount(4);
	await expect(page.locator('.actor-grid')).toContainText('Tit for Tat');
	await expect(page.getByRole('navigation', { name: 'Breadcrumb' }).getByRole('link', { name: 'Worlds' })).toHaveAttribute('href', '/worlds');
	await page.getByRole('link', { name: 'How each strategy works' }).click();
	await expect(title(page)).toHaveText('Strategies');
});

test('a finished public run fills the high scores and the indicators', async ({ page, account }) => {
	const run = await account.api.run({ iterations: 40, seed: 7 });
	await open(page, '/worlds/prisoners-dilemma', "Repeated Prisoner's Dilemma");
	const board = page.getByRole('group', { name: 'Leaderboard' });
	// Other tests run at the same time, so the board may show a later run than this one; it always shows one.
	await expect(board.getByRole('row')).not.toHaveCount(0);
	await expect(board.locator('tbody tr').first().locator('.rank')).toHaveText('1');
	const averages = (await board.locator('tbody tr td.num:nth-of-type(2)').allTextContents()).map(Number);
	expect(averages).toEqual([...averages].sort((a, b) => b - a));
	await expect(page.getByRole('link', { name: /^See run \d+$/ }).first()).toBeVisible();
	const indicators = page.getByRole('group', { name: 'Indicators' });
	await expect(indicators.locator('tbody tr')).toHaveCount(3);
	await expect(indicators).not.toContainText('No runs yet');
	expect(run.status).toBe('completed');
});

test('a world that does not exist is said to be not found', async ({ page }) => {
	await open(page, '/worlds/atlantis', 'Not found');
	await expect(page.getByRole('alert')).toContainText('World "atlantis" was not found');
	await page.getByRole('link', { name: 'Back to worlds' }).click();
	await expect(title(page)).toHaveText('Worlds');
});
