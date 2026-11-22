// The front page: a visitor plays five rounds by hand before anything is explained.
import { expect, open, test } from '../fixtures.js';

/** Plays the five rounds: `moves` is five letters, C or D. */
async function play(game, moves) {
	for (const [i, move] of [...moves].entries()) {
		await expect(game).toContainText(`Round ${i + 1} of 5. Your move:`);
		await game.getByRole('button', { name: move === 'C' ? 'Cooperate' : 'Defect' }).click();
	}
}

test('a visitor who cooperates five times meets Tit for Tat and gets the best score for two', async ({ page }) => {
	await open(page, '/');
	await expect(page).toHaveTitle('A game-theory arena · Lilanaya');
	await expect(page.getByRole('heading', { level: 1 })).toContainText('Write a strategy.');
	await page.getByRole('button', { name: 'Play a round' }).click();
	await expect(page.getByRole('heading', { name: 'Play five rounds' })).toBeFocused();
	await expect(page).toHaveURL(/#play$/);

	const game = page.locator('.hand-game');
	await expect(game.locator('.bubble')).toContainText('Five rounds, you and me.');
	// Who it is stays hidden until the game is over.
	await expect(game.locator('.talk-who')).toContainText('???');
	await play(game, 'CCCCC');

	await expect(game.locator('.bubble')).toContainText('15 each. I am Tit for Tat.');
	await expect(game.locator('.bubble')).toContainText('We both cooperated every round: 15 each');
	await expect(game.locator('.hand-score')).toHaveText(['15', '15']);
	await expect(game.locator('.talk-who')).toContainText('Tit for Tat');
	await expect(game).toContainText('You write the plan, and it plays for you.');
	// The keyboard is left on the way forward.
	await expect(game.getByRole('button', { name: 'See strategies play' })).toBeFocused();
});

test('the opponent answers a defection in the next round, and cannot answer the last one', async ({ page }) => {
	await open(page, '/#play');
	const game = page.locator('.hand-game');

	await game.getByRole('button', { name: 'Defect' }).click();
	await expect(game.locator('.hand-result')).toContainText('Round 1: You won the round');
	await expect(game.locator('.hand-result')).toContainText('You +5 · It +0');
	await expect(game.locator('.bubble')).toContainText('You took 5 and left me with nothing');
	await game.getByRole('button', { name: 'Cooperate' }).click();
	// It copies the defection one round late.
	await expect(game.locator('.hand-result')).toContainText('You cooperated, it defected.');
	await expect(game.locator('.hand-result')).toContainText('You +0 · It +5');
	await game.getByRole('button', { name: 'Cooperate' }).click();
	await game.getByRole('button', { name: 'Cooperate' }).click();
	await expect(game.locator('.bubble')).toContainText('Last round. After this one I cannot pay you back.');
	await game.getByRole('button', { name: 'Defect' }).click();
	// 5 + 0 + 3 + 3 + 5 against 0 + 5 + 3 + 3 + 0.
	await expect(game.locator('.bubble')).toContainText('16 to you, 11 to me.');
	await expect(game.locator('.bubble')).toContainText('You beat 15 by defecting at the very end');
	await expect(game.locator('.hand-score')).toHaveText(['16', '11']);

	// Playing again starts from nothing.
	await game.getByRole('button', { name: 'Play again' }).click();
	await expect(game).toContainText('Round 1 of 5. Your move:');
	await expect(game.locator('.hand-score')).toHaveText(['0', '0']);
	await expect(game.locator('.talk-who')).toContainText('???');
});

test('the whole game can be played from the keyboard', async ({ page }) => {
	await open(page, '/#play');
	const game = page.locator('.hand-game');
	await game.getByRole('button', { name: 'Cooperate' }).focus();
	for (let round = 0; round < 5; round++) {
		// The button just pressed keeps the focus, so Enter plays the same move again.
		await expect(game.getByRole('button', { name: 'Cooperate' })).toBeFocused();
		await page.keyboard.press('Enter');
	}
	await expect(game.locator('.bubble')).toContainText('15 each.');
	await page.keyboard.press('Enter');
	await expect(page.getByRole('heading', { name: 'Now strategies play each other' })).toBeFocused();
});

test('two built-in strategies can be picked and watched, and the ranking follows', async ({ page }) => {
	await open(page, '/#strategies');
	const lab = page.locator('.lab');
	// Tit for Tat against Always Defect is on show from the start: 19 against 24 after twenty rounds.
	await expect(lab.locator('.match-score')).toHaveText(['19', '24']);
	await expect(lab).toContainText('After 20 rounds: Tit for Tat 19, Always Defect 24. Always Defect wins.');

	await lab.getByRole('group', { name: 'Right player' }).getByText('Always Cooperate').click();
	await expect(lab).toContainText('After 20 rounds: Tit for Tat 60, Always Cooperate 60. A draw.');
	await lab.getByRole('group', { name: 'Left player' }).getByText('Always Defect').click();
	await expect(lab).toContainText('After 20 rounds: Always Defect 100, Always Cooperate 0. Always Defect wins.');
	await expect(lab.locator('.match-score')).toHaveText(['100', '0']);

	await lab.getByRole('button', { name: 'See the ranking' }).click();
	await expect(page.getByRole('heading', { name: 'The ranking' })).toBeFocused();
	const ranking = page.locator('#ranking');
	await expect(ranking.locator('.rank-row')).toHaveCount(4);
	const scores = (await ranking.locator('.rank-score').allTextContents()).map(Number);
	expect(scores).toEqual([...scores].sort((a, b) => b - a));
	await expect(ranking).toContainText(`${await ranking.locator('.rank-row .avatar-name').first().textContent()} comes out on top`);
});

test('the way on leads a visitor to signing up, and a logged-in user straight to a new strategy', async ({ page, someone }) => {
	await open(page, '/#your-turn');
	await expect(page.getByRole('link', { name: 'Create your first strategy' })).toHaveAttribute('href', '/sign-up?next=%2Fstrategies%2Fnew');
	await expect(page.locator('#your-turn')).toContainText('Sign up with your email. No password needed.');
	await page.getByRole('link', { name: 'How it works' }).last().click();
	await expect(page.getByRole('heading', { level: 1 })).toHaveText('How it works');

	const user = await someone();
	await open(user.page, '/#your-turn');
	await expect(user.page.getByRole('link', { name: 'Create your first strategy' })).toHaveAttribute('href', '/strategies/new');
	await user.page.getByRole('link', { name: 'Create your first strategy' }).click();
	await expect(user.page.getByRole('heading', { level: 1 })).toHaveText('New strategy');
});
