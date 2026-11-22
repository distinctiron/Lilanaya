// What one user keeps private, another user and a visitor never see: in lists, on pages, or by address.
import { expect, open, test, title, unique } from '../fixtures.js';

const card = (page, name) => page.locator('.card.fold', { has: page.getByRole('heading', { name, exact: true }) });

test('@safety a private strategy and the runs it played in exist for their owner alone', async ({ page, account, someone }) => {
	const name = unique('Secret ');
	const secret = await account.api.strategy({ name, description: 'Only I know how this plays.' });
	const run = await account.api.run({ actors: [secret.id, 'always_defect'], iterations: 20 });
	expect(run.private).toBe(true);

	// The owner sees both.
	await open(page, '/strategies', 'Strategies');
	await expect(card(page, name)).toContainText('Rules · Private');
	await open(page, '/runs', 'Runs');
	await expect(page.getByRole('link', { name: `Run ${run.id}`, exact: true })).toBeVisible();
	await open(page, `/runs/${run.id}`, `Run ${run.id}`);

	for (const other of [await someone(), await someone({ signedIn: false })]) {
		const who = other.user ? 'another user' : 'a visitor';
		await open(other.page, '/strategies', 'Strategies');
		await expect(other.page.locator('#main'), who).not.toContainText(name);
		await open(other.page, '/runs', 'Runs');
		await expect(other.page.getByRole('link', { name: `Run ${run.id}`, exact: true }), who).toHaveCount(0);
		// By address: not found, in the same words as something that was never there.
		await open(other.page, `/runs/${run.id}`, 'Not found');
		await expect(other.page.getByRole('alert'), who).toContainText(`Run "${run.id}" was not found`);
		await open(other.page, `/runs/${run.id}?a=${secret.id}&b=always_defect`, 'Not found');
		await expect(other.page.locator('#main'), who).not.toContainText(name);
		for (const path of [`/api/strategies/${secret.id}`, `/api/runs/${run.id}`, `/api/runs/${run.id}/race`, `/api/runs/${run.id}/artifacts`, `/api/runs/${run.id}/artifacts/run_summary.json`, `/api/runs/${run.id}/matches/always_defect/${secret.id}`]) {
			const answer = await other.context.request.get(path);
			expect(answer.status(), `${who}: ${path}`).toBe(404);
			expect(await answer.text(), `${who}: ${path}`).not.toContain(name);
		}
		if (other.user) {
			// Not on offer for a run, and not to be edited.
			await open(other.page, '/runs/new', 'New run');
			await expect(other.page.locator('#main')).not.toContainText(name);
			await open(other.page, `/strategies/${secret.id}/edit`, 'Not found');
			const entered = await other.context.request.post('/api/runs', { data: { world_id: 'prisoners-dilemma', actors: [secret.id, 'always_defect'] } });
			expect(entered.status()).toBe(422);
		}
	}
	// The world's public standings were not fed by it.
	const board = await account.api.get('/api/worlds/prisoners-dilemma/leaderboard');
	expect(board.run_id).not.toBe(run.id);
});

test('@safety a published strategy can be read and played by others, and changed by nobody but its owner', async ({ page, account, someone }) => {
	const name = unique('Shared ');
	const shared = await account.api.strategy({ name, visibility: 'public', prompt: 'the words I typed for the AI' });
	const other = await someone();

	await open(other.page, '/strategies', 'Strategies');
	const theirs = card(other.page, name);
	// Listed under other players, with the author's display name and nothing else about them.
	await expect(theirs).toContainText(`Rules · By ${account.user.display_name}`);
	await theirs.locator('summary').click();
	await expect(theirs.getByRole('group', { name: `State machine of ${name}` })).toBeVisible();
	await expect(theirs.getByRole('link', { name: /Edit/ })).toHaveCount(0);
	await expect(theirs.getByRole('button', { name: /Delete/ })).toHaveCount(0);
	await expect(other.page.locator('#main')).not.toContainText(account.email);
	await expect(other.page.locator('#main')).not.toContainText('the words I typed for the AI');

	// Its editor is closed to them, and so is the API behind it.
	await open(other.page, `/strategies/${shared.id}/edit`);
	await expect(other.page.getByRole('alert')).toContainText('Only its owner can edit a strategy.');
	expect((await other.context.request.put(`/api/strategies/${shared.id}`, { data: { name: 'Mine now', mode: 'rules', states: ['A'], initial_state: 'A', action_map: { A: 'D' } } })).status()).toBe(403);
	expect((await other.context.request.delete(`/api/strategies/${shared.id}`)).status()).toBe(403);
	expect((await other.context.request.get(`/api/strategies/${shared.id}`).then((r) => r.json())).prompt).toBeUndefined();

	// They can send it into a run of their own.
	await open(other.page, '/runs/new', 'New run');
	const box = other.page.getByRole('checkbox', { name: new RegExp(name) });
	await expect(other.page.locator('label.check', { has: box })).toContainText(`By ${account.user.display_name}`);
	await box.check();
	await other.page.getByLabel('Rounds per match').fill('20');
	await other.page.getByRole('button', { name: 'Run simulation' }).click();
	await expect(other.page.getByRole('heading', { name: 'High scores' })).toBeVisible({ timeout: 30_000 });
	await expect(other.page.locator('table.bars')).toContainText(name);
	// Unchanged for its owner.
	expect(await account.api.get(`/api/strategies/${shared.id}`)).toMatchObject({ name, visibility: 'public', mine: true, prompt: 'the words I typed for the AI' });
});

test('@safety logging out on one device leaves nothing behind on it', async ({ page, account }) => {
	const secret = await account.api.strategy({ name: unique('Mine ') });
	await open(page, '/profile', 'Profile');
	await page.getByRole('button', { name: 'Log out' }).click();
	await expect(title(page)).toHaveText('Worlds');
	// The pages that showed private things now ask for a log-in, also when gone back to.
	for (const path of ['/profile', '/runs/new', `/strategies/${secret.id}/edit`, '/strategies/new']) await open(page, path, 'Log in first');
	await open(page, '/strategies', 'Strategies');
	await expect(page.locator('#main')).not.toContainText(secret.name);
	expect((await page.request.get('/api/me')).status()).toBe(401);
	expect((await page.context().cookies()).filter((c) => c.name === 'lilanaya_session')).toEqual([]);
});
