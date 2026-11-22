// Runs: the form, the results (race, high scores, head to head, match view, files) and the list.
import { BREAKS_IN_A_RUN, expect, open, test, title, unique } from '../fixtures.js';

const BUILT_IN = ['Always Cooperate', 'Always Defect', 'Tit for Tat', 'Win-Stay Lose-Shift'];

/** Fills in the form and starts the run; returns the run's number once its results are on show. */
async function start(page, { rounds = 50, seed, order } = {}) {
	await page.getByLabel('Rounds per match').fill(String(rounds));
	if (seed != null) await page.getByLabel('Seed').fill(String(seed));
	if (order) await page.getByLabel('Match order').selectOption(order);
	await page.getByRole('button', { name: 'Run simulation' }).click();
	await expect(page).toHaveURL(/\/runs\/\d+$/);
	const id = Number(page.url().split('/').pop());
	// Queued, then running, then the results: the page follows by itself.
	await expect(page.getByRole('heading', { name: 'High scores' })).toBeVisible({ timeout: 30_000 });
	await expect(title(page)).toHaveText(`Run ${id}`);
	return id;
}

/** The final table as shown: [[name, total], …] from first to last. */
async function highScores(page) {
	const rows = page.locator('table.bars tbody tr');
	const table = [];
	for (let i = 0; i < (await rows.count()); i++) table.push([await rows.nth(i).locator('th.name').innerText(), await rows.nth(i).locator('td.score').innerText()]);
	return table;
}

test('the form starts from the world’s own settings with the built-in strategies entered', async ({ page, account }) => {
	await open(page, '/runs/new?world=prisoners-dilemma', 'New run');
	await expect(page.getByLabel('World')).toHaveValue('prisoners-dilemma');
	for (const name of BUILT_IN) await expect(page.getByRole('checkbox', { name: new RegExp(name) })).toBeChecked();
	await expect(page.getByLabel('Rounds per match')).toHaveValue('1000');
	await expect(page.getByLabel('Seed')).toHaveValue('42');
	await expect(page.getByLabel('Match order')).toHaveValue('all_pairs');

	// A run needs two actors: said where they are picked.
	for (const name of BUILT_IN.slice(1)) await page.getByRole('checkbox', { name: new RegExp(name) }).uncheck();
	await page.getByRole('button', { name: 'Run simulation' }).click();
	await expect(page.getByRole('alert').filter({ hasText: 'Pick at least two actors.' })).toBeVisible();
	await expect(page.getByRole('checkbox', { name: /Always Cooperate/ })).toBeFocused();
	await expect(page).toHaveURL(/\/runs\/new/);

	// Numbers out of range do not leave the browser.
	await page.getByRole('checkbox', { name: /Tit for Tat/ }).check();
	for (const [label, wrong] of [['Rounds per match', '0'], ['Rounds per match', '10001'], ['Rounds per match', '2.5'], ['Seed', '-1'], ['Seed', '']]) {
		const field = page.getByLabel(label);
		const before = await field.inputValue();
		await field.fill(wrong);
		await page.getByRole('button', { name: 'Run simulation' }).click();
		expect(await field.evaluate((el) => el.validity.valid), `${label} = "${wrong}"`).toBe(false);
		await expect(page).toHaveURL(/\/runs\/new/);
		await field.fill(before);
	}
	expect((await account.api.get('/api/usage')).used).toBe(0);
});

test('a run is started, finishes, and shows who won, every pairing and every round', async ({ page, account }) => {
	await open(page, '/runs/new', 'New run');
	const id = await start(page, { rounds: 50 });
	await expect(page).toHaveTitle(`Run ${id} · Lilanaya`);
	await expect(page.locator('.race-screen > .meta').first()).toContainText('Seed 42 · 4 actors · 50 rounds per match');
	await expect(page.getByRole('button', { name: 'Replay' })).toBeVisible();

	// The final table: totals over three matches of fifty rounds each.
	expect(await highScores(page)).toEqual([
		['Always Defect', '454'],
		['Tit for Tat', '349'],
		['Win-Stay Lose-Shift', '325'],
		['Always Cooperate', '300']
	]);

	// Head to head: nobody plays themselves, every other cell is a match to watch.
	const matrix = page.locator('table.matrix');
	await expect(matrix.locator('td.cell-empty')).toHaveCount(4);
	await expect(matrix.locator('td.cell a')).toHaveCount(12);
	await expect(page.getByRole('heading', { name: /^Match/ })).toHaveText('Match');
	await expect(page.locator('#main')).toContainText('Pick a cell in the head-to-head table to watch that match here');

	const cell = matrix.getByRole('link', { name: 'Tit for Tat scored 0.98 points per round against Always Defect. Show this match.' });
	await expect(cell).toHaveText('0.98');
	await cell.click();
	await expect(page.getByRole('heading', { name: 'Match: Tit for Tat versus Always Defect' })).toBeVisible();
	await expect(cell).toHaveAttribute('aria-current', 'true');
	// The address names the match, so it can be shared.
	await expect(page).toHaveURL(new RegExp(`/runs/${id}\\?a=tit_for_tat&b=always_defect$`));
	await expect(page.locator('.match .match-score')).toHaveText(['49', '54']);
	await expect(page.locator('#main')).toContainText('Round 40 / 40 shown of 50');
	await expect(page.locator('#main')).toContainText('Scores are totals over all 50 rounds');
	await expect(page.getByRole('img', { name: 'First 40 of 50 rounds. Tit for Tat: 1 cooperate, 39 defect. Always Defect: 0 cooperate, 40 defect.' })).toBeVisible();

	// The same match from the other side.
	await matrix.getByRole('link', { name: /^Always Defect scored 1\.08 points per round against Tit for Tat/ }).click();
	await expect(page.getByRole('heading', { name: 'Match: Always Defect versus Tit for Tat' })).toBeVisible();
	await expect(page.locator('.match .match-score')).toHaveText(['54', '49']);

	// Opened by its address, the match is on show from the start.
	await page.goto(`/runs/${id}?a=always_cooperate&b=win_stay_lose_shift`);
	await expect(page.getByRole('heading', { name: 'Match: Always Cooperate versus Win-Stay Lose-Shift' })).toBeVisible();
	await expect(page.locator('.match .match-score')).toHaveText(['150', '150']);

	// The indicators of this run.
	await expect(page.getByRole('group', { name: 'Indicators' }).locator('tbody tr')).toHaveCount(3);
	expect((await account.api.get('/api/usage')).used).toBe(1);
});

test('a run’s files can be downloaded, and the manifest lists every one with its checksum', async ({ page, account }) => {
	const run = await account.api.run({ iterations: 20 });
	await open(page, `/runs/${run.id}`, `Run ${run.id}`);
	const downloads = page.locator('.screen', { has: page.getByRole('heading', { name: 'Downloads' }) }).first();
	for (const name of ['run_summary.json', 'pairwise_matrix.json', 'manifest.json']) {
		await expect(downloads.getByRole('link', { name, exact: true })).toHaveAttribute('href', `/api/runs/${run.id}/artifacts/${name}`);
	}
	await downloads.getByText('Round-by-round traces (6 matches)').click();
	await expect(downloads.getByRole('link', { name: 'always_cooperate__always_defect.json' })).toBeVisible();

	const manifest = await (await page.request.get(`/api/runs/${run.id}/artifacts/manifest.json`)).json();
	expect(manifest).toMatchObject({ run_id: run.id, world_id: 'prisoners-dilemma', seed: 42, iterations: 20 });
	expect(manifest.artifacts.map((a) => a.name).sort()).toEqual(
		['pairwise_matrix.json', 'run_summary.json', ...['always_cooperate__always_defect', 'always_cooperate__tit_for_tat', 'always_cooperate__win_stay_lose_shift', 'always_defect__tit_for_tat', 'always_defect__win_stay_lose_shift', 'tit_for_tat__win_stay_lose_shift'].map((n) => `traces/${n}.json`)].sort()
	);
	// Every file is what the manifest says it is.
	const { createHash } = await import('node:crypto');
	for (const listed of manifest.artifacts) {
		const body = await (await page.request.get(`/api/runs/${run.id}/artifacts/${listed.name}`)).body();
		expect(body.length, listed.name).toBe(listed.size_bytes);
		expect(createHash('sha256').update(body).digest('hex'), listed.name).toBe(listed.sha256);
	}
});

test('the same seed gives the same result, whatever order the matches are played in', async ({ page, account }) => {
	await open(page, '/runs/new', 'New run');
	const first = await start(page, { rounds: 80, seed: 2026 });
	const table = await highScores(page);
	await expect(page.locator('.race-screen > .meta').first()).not.toContainText('Random order');

	await open(page, '/runs/new', 'New run');
	const second = await start(page, { rounds: 80, seed: 2026, order: 'sample_per_round' });
	expect(second).not.toBe(first);
	await expect(page.locator('.race-screen > .meta').first()).toContainText('Seed 2026 · 4 actors · 80 rounds per match · Random order, reshuffled every round');
	expect(await highScores(page)).toEqual(table);

	await open(page, '/runs/new', 'New run');
	await start(page, { rounds: 80, seed: 2026, order: 'shuffle_all' });
	await expect(page.locator('.race-screen > .meta').first()).toContainText('Random order, shuffled once');
	expect(await highScores(page)).toEqual(table);
});

test('@safety the month’s runs are counted, and the form says so when they are used up', async ({ page, account }) => {
	// This server gives a free account three runs a month.
	for (let i = 0; i < 3; i++) await account.api.run({ iterations: 5 });
	await open(page, '/profile', 'Profile');
	await expect(page.locator('#main')).toContainText('0 left · 3 of 3 used');
	await expect(page.getByRole('img', { name: '3 of 3 runs used' })).toBeVisible();

	await open(page, '/runs/new', 'New run');
	await page.getByRole('button', { name: 'Run simulation' }).click();
	await expect(page.getByRole('alert').filter({ hasText: 'You have used all' })).toHaveText('You have used all 3 runs your plan gives you this month. The count starts again on the 1st.');
	// Nothing was started, and the form can be used again.
	await expect(page).toHaveURL(/\/runs\/new$/);
	await expect(page.getByRole('button', { name: 'Run simulation' })).toBeEnabled();
	expect((await account.api.get('/api/usage')).used).toBe(3);
});

test('a user’s own strategy is entered in a run, and a private one makes the run private', async ({ page, account }) => {
	const name = unique('Mine ');
	const mine = await account.api.strategy({ name });
	await open(page, '/runs/new', 'New run');
	const box = page.getByRole('checkbox', { name: new RegExp(name) });
	// Own strategies are offered, not entered, until ticked.
	await expect(box).not.toBeChecked();
	await expect(page.locator('label.check', { has: box })).toContainText('Private: the run will be private too');
	await box.check();
	const id = await start(page, { rounds: 40 });
	await expect(page.locator('.race-screen > .meta').first()).toContainText('5 actors');
	expect((await highScores(page)).map(([who]) => who)).toContain(name);
	// Its avatar is the strategy's own, and its match can be watched like any other.
	await page.locator('table.matrix').getByRole('link', { name: new RegExp(`^${name} scored .* against Always Defect`) }).click();
	await expect(page.getByRole('heading', { name: `Match: ${name} versus Always Defect` })).toBeVisible();
	await expect(page).toHaveURL(new RegExp(`a=${mine.id}&b=always_defect$`));
	expect((await account.api.get(`/api/runs/${id}`)).private).toBe(true);
});

test('a run that fails says which strategy broke and in which round', async ({ page, account }) => {
	const broken = await account.api.code(unique('Breaks '), BREAKS_IN_A_RUN, { visibility: 'public' });
	const run = await account.api.run({ actors: [broken.id, 'always_defect'], iterations: 50 });
	expect(run.status).toBe('failed');
	await open(page, `/runs/${run.id}`, `Run ${run.id}`);
	const alert = page.getByRole('alert');
	await expect(alert).toContainText('This run failed. The run could not be completed');
	await expect(alert).toContainText(`actors.${broken.id}: ${broken.name} failed in round 26: boom`);
	// No winner is shown for a run that did not finish.
	await expect(page.getByRole('heading', { name: 'High scores' })).toHaveCount(0);
	await expect(page.locator('#main').getByRole('link', { name: 'New run' })).toHaveAttribute('href', '/runs/new?world=prisoners-dilemma');

	await open(page, '/runs', 'Runs');
	const row = page.getByRole('row', { name: new RegExp(`^Run ${run.id}\\b`) });
	await expect(row).toContainText('failed');
});

test('while a run waits or runs, the page says so and shows the result when it is there', async ({ page, account }) => {
	const run = await account.api.run({ iterations: 20 });
	// The first two answers say the run is still on its way; then the real ones come through.
	const waiting = { id: run.id, status: 'queued', world_id: run.world_id, world_name: run.world_name, seed: run.seed, iterations: run.iterations, actor_count: run.actor_count, private: false, created_at: run.created_at, finished_at: null };
	let asked = 0;
	await page.route(`**/api/runs/${run.id}`, (route) => {
		asked += 1;
		if (asked === 1) return route.fulfill({ json: waiting });
		if (asked === 2) return route.fulfill({ json: { ...waiting, status: 'running' } });
		return route.continue();
	});
	await page.goto(`/runs/${run.id}`);
	await expect(page.getByRole('status')).toHaveText('Waiting in the queue…');
	await expect(page.locator('#main')).toContainText("Repeated Prisoner's Dilemma · Seed 42 · 4 actors · 20 rounds per match");
	await expect(page.getByRole('heading', { name: 'High scores' })).toBeVisible();
	expect(asked).toBeGreaterThanOrEqual(3);
});

test('the list shows the latest runs with their winners, newest first', async ({ page, account }) => {
	const first = await account.api.run({ iterations: 50 });
	const second = await account.api.run({ iterations: 10, actors: ['always_cooperate', 'tit_for_tat'], seed: 9 });
	await open(page, '/runs', 'Runs');
	const row = (id) => page.getByRole('row', { name: new RegExp(`^Run ${id}\\b`) });
	await expect(row(first.id)).toContainText('completed');
	await expect(row(first.id)).toContainText('Always Defect (3.03)');
	await expect(row(second.id).getByRole('cell')).toHaveText(['completed', "Repeated Prisoner's Dilemma", '9', '10', '2', /\(3\.00\)$/]);
	// Newest first.
	const ids = (await page.locator('table.runs-list tbody th a').allTextContents()).map((text) => Number(text.replace('Run ', '')));
	expect(ids).toEqual([...ids].sort((a, b) => b - a));
	expect(ids.indexOf(second.id)).toBeLessThan(ids.indexOf(first.id));

	await row(first.id).getByRole('link', { name: `Run ${first.id}` }).click();
	await expect(title(page)).toHaveText(`Run ${first.id}`);
	await page.getByRole('link', { name: "Repeated Prisoner's Dilemma" }).click();
	await expect(title(page)).toHaveText("Repeated Prisoner's Dilemma");
});

test('a run that does not exist is said to be not found', async ({ page }) => {
	for (const path of ['/runs/987654321', '/runs/first']) {
		await open(page, path, 'Not found');
		await expect(page.getByRole('alert')).toContainText('was not found');
	}
});

test.describe('with motion on', () => {
	test.use({ contextOptions: { reducedMotion: 'no-preference' } });

	test('the race is run the first time a result is opened, and a match plays round by round', async ({ page, account }) => {
		const run = await account.api.run({ iterations: 50 });
		await open(page, `/runs/${run.id}`, `Run ${run.id}`);
		// The race ends on the final standings.
		await expect(page.locator('.race-screen')).toContainText('Always Defect');
		await page.getByRole('link', { name: /^Tit for Tat scored 0\.98/ }).click();
		const label = page.locator('#main').getByText(/Round \d+ \/ 40 shown of 50/);
		// Rounds fill in one at a time, and the points count up with them.
		await expect(label).not.toHaveText('Round 40 / 40 shown of 50', { timeout: 1000 });
		await expect(label).toHaveText('Round 40 / 40 shown of 50', { timeout: 15_000 });
		await expect(page.locator('.match .match-score')).toHaveText(['49', '54']);
		// Replay runs the race again without leaving the page.
		await page.getByRole('button', { name: 'Replay' }).click();
		await expect(title(page)).toHaveText(`Run ${run.id}`);
	});
});
