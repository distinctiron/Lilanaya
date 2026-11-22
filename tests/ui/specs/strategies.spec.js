// Strategies written by hand: rules built from states and conditions, and code.
import { expect, open, test, title, unique } from '../fixtures.js';

/** Opens the rules editor ready for building by hand. */
async function rulesByHand(page) {
	await open(page, '/strategies/new', 'New strategy');
	await page.getByRole('link', { name: 'Write rules' }).click();
	await expect(title(page)).toHaveText('New rules strategy');
	// With the AI switched on the editor opens on the description; building by hand is one press away.
	await page.getByRole('button', { name: 'Build the rules by hand instead' }).click();
	await expect(page.getByLabel('Name', { exact: true })).toBeVisible();
}

/** The fold in the list for the strategy called `name`. */
const card = (page, name) => page.locator('.card.fold', { has: page.getByRole('heading', { name, exact: true }) });

test('a visitor can read how the built-in strategies work, and is offered an account to make their own', async ({ page }) => {
	await open(page, '/strategies', 'Strategies');
	await expect(page.getByRole('link', { name: 'Sign up to make your own' })).toHaveAttribute('href', '/sign-up?next=%2Fstrategies%2Fnew');
	await expect(page.getByRole('link', { name: 'New strategy' })).toHaveCount(0);
	const builtIn = page.locator('.screen', { has: page.getByRole('heading', { name: 'Built in' }) }).last();
	for (const name of ['Always Cooperate', 'Always Defect', 'Tit for Tat', 'Win-Stay Lose-Shift']) await expect(card(page, name)).toBeVisible();
	void builtIn;

	const tft = card(page, 'Tit for Tat');
	await expect(tft).toContainText('Rules');
	await tft.locator('summary').click();
	const machine = tft.getByRole('group', { name: 'State machine of Tit for Tat' });
	await expect(machine).toBeVisible();
	await expect(machine).toContainText('when the opponent just played D');
	// Nobody edits a built-in.
	await expect(tft.getByRole('link', { name: /Edit/ })).toHaveCount(0);
	await expect(tft.getByRole('button', { name: /Delete/ })).toHaveCount(0);
	// One strategy is open at a time.
	await card(page, 'Always Defect').locator('summary').click();
	await expect(machine).toBeHidden();

	// Making one needs an account.
	await open(page, '/strategies/new', 'Log in first');
	await open(page, '/strategies/new/rules', 'Log in first');
	await open(page, '/strategies/tit_for_tat/edit', 'Log in first');
});

test('rules are built by hand, previewed against the samples, created, and listed as private', async ({ page, account }) => {
	const name = unique('Grudger ');
	await rulesByHand(page);
	await page.getByLabel('Name', { exact: true }).fill(name);
	await page.getByLabel('Description', { exact: true }).fill('Cooperates until betrayed, then never forgives.');

	// One state to begin with; add the second and the move between them.
	const state = (n) => page.locator(`fieldset[data-state="${n}"]`);
	await expect(page.locator('fieldset[data-state]')).toHaveCount(1);
	await state(0).getByLabel('State name').fill('NICE');
	await state(0).getByLabel('State name').blur();
	await page.getByRole('button', { name: 'Add a state' }).click();
	await expect(state(1).getByLabel('State name')).toBeFocused();
	await state(1).getByLabel('State name').fill('GRIM');
	await state(1).getByLabel('State name').blur();
	await state(1).getByLabel('What GRIM plays').selectOption('D');
	await state(0).getByRole('button', { name: 'Add a move out of NICE' }).click();
	// A new move goes to the other state when the opponent just defected: exactly a grudger.
	await expect(state(0).getByLabel('State to go to')).toHaveValue('1');
	await expect(state(0).getByLabel('Kind of condition')).toHaveValue('last_opponent_action');
	await expect(state(0).getByLabel('Decision')).toHaveValue('D');
	await expect(page.locator('.card.fold summary', { hasText: 'Rules' })).toContainText('2 states · 1 move');

	await page.getByRole('button', { name: 'Check and preview' }).click();
	const report = page.locator('[aria-live="polite"]', { hasText: 'The strategy is valid.' });
	await expect(report).toBeVisible();
	await expect(report).toContainText('Worth a look, though it will run:');
	await expect(report).toContainText('once in "GRIM", the strategy never leaves it');
	const samples = report.locator('.preview-match');
	await expect(samples).toHaveCount(2);
	await expect(samples.nth(0)).toContainText('Against Always Cooperate · 20 rounds · You 60, them 60');
	await expect(samples.nth(1)).toContainText('Against Always Defect · 20 rounds · You 19, them 24');
	await expect(page.getByRole('button', { name: 'Check and preview' })).toBeEnabled();

	await expect(page.getByLabel('Who can see it')).toHaveValue('private');
	await page.getByRole('button', { name: 'Create strategy' }).click();
	await expect(title(page)).toHaveText('Your new competitor');
	await expect(page.getByRole('img', { name: `The avatar of ${name}` })).toBeVisible();
	await expect(page.locator('.fighter')).toContainText('Rules · Private · Created');

	await page.getByRole('link', { name: 'All strategies' }).click();
	await expect(title(page)).toHaveText('Strategies');
	const mine = card(page, name);
	await expect(mine).toContainText('Rules · Private');
	await mine.locator('summary').click();
	await expect(mine).toContainText('Cooperates until betrayed, then never forgives.');
	await expect(mine.getByRole('group', { name: `State machine of ${name}` })).toContainText('to GRIM when the opponent just played D');
	// It is in the database as it was built.
	const saved = (await account.api.get('/api/strategies')).strategies.find((s) => s.name === name);
	expect(saved).toMatchObject({ mode: 'rules', visibility: 'private', mine: true, states: ['NICE', 'GRIM'], initial_state: 'NICE', action_map: { NICE: 'C', GRIM: 'D' } });
	expect(saved.transitions).toEqual([{ from: 'NICE', to: 'GRIM', guard: { op: 'last_opponent_action', is: 'D' } }]);
});

test('every kind of condition can be built, combined, reordered and removed', async ({ page, account }) => {
	const name = unique('Conditions ');
	await rulesByHand(page);
	await page.getByLabel('Name', { exact: true }).fill(name);
	const state = (n) => page.locator(`fieldset[data-state="${n}"]`);
	await page.getByRole('button', { name: 'Add a state' }).click();
	await state(1).getByLabel('What STATE_2 plays').selectOption('D');

	// First move: "unless the opponent has repeated C 3 times in a row", and also "the rounds left are at most 2".
	await state(0).getByRole('button', { name: 'Add a move out of START' }).click();
	const first = state(0).locator('[data-move="0"]');
	await first.getByLabel('True or not true').selectOption('not');
	await first.getByLabel('Kind of condition').selectOption('opponent_streak');
	await first.getByLabel('Decision').selectOption('C');
	await first.getByLabel('How many times').fill('3');
	await first.getByRole('button', { name: 'Add condition' }).click();
	await first.getByLabel('Kind of condition').nth(1).selectOption('rounds_left');
	await first.getByLabel('How many rounds').fill('2');
	await first.getByLabel('How the conditions combine').selectOption('any');

	// Second move: "the same result has repeated 2 rounds in a row: I played C and the opponent D".
	await state(0).getByRole('button', { name: 'Add a move out of START' }).click();
	const second = state(0).locator('[data-move="1"]');
	await second.getByLabel('Kind of condition').selectOption('outcome_streak');
	await second.getByLabel('My decision').selectOption('C');
	await second.getByLabel("The opponent's decision").selectOption('D');
	// Moves are checked top to bottom; this one is moved to the top.
	await second.getByRole('button', { name: 'Check this move earlier' }).click();
	await expect(state(0).locator('[data-move="0"]').getByLabel('Kind of condition')).toHaveValue('outcome_streak');

	// A third, added and taken away again, leaves no trace.
	await state(0).getByRole('button', { name: 'Add a move out of START' }).click();
	await state(0).locator('[data-move="2"]').getByRole('button', { name: 'Remove move' }).click();
	await expect(state(0).locator('[data-move]')).toHaveCount(2);

	// Back from the second state after ten rounds, when it just played D itself.
	await state(1).getByRole('button', { name: 'Add a move out of STATE_2' }).click();
	const back = state(1).locator('[data-move="0"]');
	await back.getByLabel('Kind of condition').selectOption('round_index');
	await back.getByLabel('How many rounds').fill('10');
	await back.getByRole('button', { name: 'Add condition' }).click();
	await back.getByLabel('Kind of condition').nth(1).selectOption('last_own_action');
	await back.getByLabel('Decision').selectOption('D');

	await page.getByRole('button', { name: 'Check and preview' }).click();
	await expect(page.getByText('The strategy is valid.')).toBeVisible();
	await page.getByRole('button', { name: 'Create strategy' }).click();
	await expect(title(page)).toHaveText('Your new competitor');

	const saved = (await account.api.get('/api/strategies')).strategies.find((s) => s.name === name);
	expect(saved.transitions).toEqual([
		{ from: 'START', to: 'STATE_2', guard: { op: 'outcome_streak', outcome: 'C/D', at_least: 2 } },
		{ from: 'START', to: 'STATE_2', guard: { op: 'or', any: [{ op: 'not', guard: { op: 'opponent_streak', decision: 'C', at_least: 3 } }, { op: 'rounds_left', at_most: 2 }] } },
		{ from: 'STATE_2', to: 'START', guard: { op: 'and', all: [{ op: 'round_index', at_least: 10 }, { op: 'last_own_action', is: 'D' }] } }
	]);

	// Opened again, the editor shows what was saved, condition for condition.
	await page.getByRole('link', { name: 'Edit' }).click();
	await expect(title(page)).toHaveText(`Edit ${name}`);
	const again = page.locator('fieldset[data-state="0"] [data-move="1"]');
	await expect(again.getByLabel('How the conditions combine')).toHaveValue('any');
	await expect(again.getByLabel('True or not true').first()).toHaveValue('not');
	await expect(again.getByLabel('How many times')).toHaveValue('3');
});

test('a strategy that cannot play is told why, and is not saved', async ({ page, account }) => {
	await rulesByHand(page);
	// No name, and two states with the same name.
	await page.getByRole('button', { name: 'Add a state' }).click();
	await page.locator('fieldset[data-state="1"]').getByLabel('State name').fill('START');
	await page.locator('fieldset[data-state="1"]').getByLabel('State name').blur();
	await page.getByRole('button', { name: 'Check and preview' }).click();
	const problems = page.getByRole('alert').filter({ hasText: 'This strategy cannot play yet:' });
	await expect(problems).toContainText('states[1]: "START" is listed more than once');

	await page.getByRole('button', { name: 'Create strategy' }).click();
	const refused = page.getByRole('alert').filter({ hasText: 'The strategy is not valid' });
	await expect(refused).toContainText('name: must be 1 to 80 characters');
	await expect(refused).toContainText('states[1]: "START" is listed more than once');
	await expect(title(page)).toHaveText('New rules strategy');
	await expect(page.getByRole('button', { name: 'Create strategy' })).toBeEnabled();
	expect((await account.api.get('/api/strategies')).strategies.filter((s) => s.mine)).toEqual([]);
});

test('a strategy is edited, published and deleted by its owner', async ({ page, account }) => {
	const name = unique('Editable ');
	const made = await account.api.strategy({ name });
	await open(page, '/strategies', 'Strategies');
	await card(page, name).locator('summary').click();
	await card(page, name).getByRole('link', { name: `Edit ${name}` }).click();
	await expect(title(page)).toHaveText(`Edit ${name}`);
	await expect(page).toHaveURL(new RegExp(`/strategies/${made.id}/edit$`));
	await expect(page.getByLabel('Name', { exact: true })).toHaveValue(name);
	await expect(page.locator('fieldset[data-state]')).toHaveCount(2);

	const renamed = `${name} II`;
	await page.getByLabel('Name', { exact: true }).fill(renamed);
	await page.getByLabel('Who can see it').selectOption('public');
	await page.getByRole('button', { name: 'Save changes' }).click();
	await expect(title(page)).toHaveText('Strategies');
	await expect(card(page, renamed)).toContainText('Rules · Public');
	await expect(card(page, name)).toHaveCount(0);
	// Same strategy, same id, same avatar.
	expect((await account.api.get(`/api/strategies/${made.id}`)).name).toBe(renamed);

	// Deleting asks first; saying no keeps it.
	await card(page, renamed).locator('summary').click();
	page.once('dialog', (dialog) => {
		expect(dialog.message()).toBe(`Delete "${renamed}"? Runs it already played in stay as they are.`);
		dialog.dismiss();
	});
	await card(page, renamed).getByRole('button', { name: `Delete ${renamed}` }).click();
	await expect(card(page, renamed)).toBeVisible();
	page.once('dialog', (dialog) => dialog.accept());
	await card(page, renamed).getByRole('button', { name: `Delete ${renamed}` }).click();
	await expect(card(page, renamed)).toHaveCount(0);
	await expect(page.getByText('You have no strategies yet')).toBeVisible();
});

test('a strategy is written as code, checked, and created', async ({ page, account }) => {
	const name = unique('Copycat ');
	await open(page, '/strategies/new', 'New strategy');
	await page.getByRole('link', { name: 'Write code' }).click();
	await expect(title(page)).toHaveText('New code strategy');
	// It starts from a working example: copy the opponent's last move.
	await expect(page.getByLabel('Code')).toHaveValue(/function decide\(input\)/);
	await page.getByText('What decide receives').click();
	await expect(page.locator('dl.rules')).toContainText('lastOpponentAction');
	await page.getByLabel('Name', { exact: true }).fill(name);

	await page.getByRole('button', { name: 'Check and preview' }).click();
	const samples = page.locator('.preview-match');
	await expect(samples.nth(0)).toContainText('Against Always Cooperate · 20 rounds · You 60, them 60');
	await expect(samples.nth(1)).toContainText('Against Always Defect · 20 rounds · You 19, them 24');

	// Broken code is explained in the author's terms.
	const code = page.getByLabel('Code');
	for (const [source, says] of [
		["function decide() { return 'MAYBE'; }", 'decide returned "MAYBE"; it must return one of: C, D'],
		['function decide() { while (true) {} }', 'took too many steps (is there a loop that never ends?)'],
		['const answer = 42;', 'the code must define a function named decide'],
		["function decide() { return Date.now() % 2 ? 'C' : 'D'; }", 'Date']
	]) {
		await code.fill(source);
		await page.getByRole('button', { name: 'Check and preview' }).click();
		await expect(page.getByRole('alert').filter({ hasText: 'This strategy cannot play yet:' })).toContainText(says);
	}
	await page.getByRole('button', { name: 'Create strategy' }).click();
	await expect(page.getByRole('alert').filter({ hasText: 'The strategy is not valid' })).toBeVisible();

	// Code that counts: something rules cannot do.
	await code.fill("function decide(input) {\n  return input.opponentCounts.D > input.opponentCounts.C ? 'D' : 'C';\n}\n");
	await page.getByRole('button', { name: 'Create strategy' }).click();
	await expect(title(page)).toHaveText('Your new competitor');
	await expect(page.locator('.fighter')).toContainText('Code · Private · Created');
	await page.getByRole('link', { name: 'All strategies' }).click();
	await card(page, name).locator('summary').click();
	await expect(card(page, name).getByLabel(`Code of ${name}`)).toContainText('input.opponentCounts.D');
	expect((await account.api.get('/api/strategies')).strategies.find((s) => s.name === name)).toMatchObject({ mode: 'code', language: 'javascript' });
});

test('the kind of a strategy is fixed, and addresses that make no sense say so', async ({ page, account }) => {
	const made = await account.api.code(unique('Coded '), "function decide() { return 'C'; }");
	await open(page, `/strategies/${made.id}/edit`);
	await expect(title(page)).toHaveText(`Edit ${made.name}`);
	// A code strategy is edited as code: there are no states to build.
	await expect(page.getByLabel('Code')).toBeVisible();
	await expect(page.locator('fieldset[data-state]')).toHaveCount(0);

	await open(page, '/strategies/new/poetry', 'Not found');
	await open(page, '/strategies/no-such-strategy/edit', 'Not found');
	// A built-in is nobody's to edit.
	await open(page, '/strategies/tit_for_tat/edit');
	await expect(page.getByRole('alert')).toContainText('Only its owner can edit a strategy.');
});

test.describe('with motion on', () => {
	test.use({ contextOptions: { reducedMotion: 'no-preference' } });

	test('the preview plays its sample matches round by round, and a new strategy gets its avatar block by block', async ({ page, account }) => {
		await rulesByHand(page);
		await page.getByLabel('Name', { exact: true }).fill(unique('Animated '));
		await page.getByRole('button', { name: 'Check and preview' }).click();
		await expect(page.getByRole('button', { name: 'Playing…' })).toBeDisabled();
		const first = page.locator('.preview-match').first();
		await expect(first.locator('.preview-caption')).toContainText(/Round \d+ of 20 · You played C/);
		// It ends on the finished match, and the button comes back.
		await expect(page.getByRole('button', { name: 'Check and preview' })).toBeEnabled({ timeout: 20_000 });
		await expect(first).toContainText('You 60, them 60');
		await expect(first.locator('.preview-caption')).toHaveText('20 rounds played');

		await page.getByRole('button', { name: 'Create strategy' }).click();
		await expect(title(page)).toHaveText('Your new competitor');
		const stand = page.locator('.fighter-stand');
		await expect(stand).toHaveClass(/is-ready/, { timeout: 15_000 });
		await expect(stand.locator('svg rect[visibility="hidden"]')).toHaveCount(0);
	});
});
