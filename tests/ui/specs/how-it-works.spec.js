// How it works: three narrated films, each a row of chapters.
import { expect, open, test } from '../fixtures.js';

const FILMS = ['Film: what Lilanaya is, in four chapters', 'Film: how Lilanaya works, in four steps', 'Film: the Prisoner’s Dilemma, in four chapters'];

test('with motion reduced, each film is a set of finished pictures to step through', async ({ page }) => {
	await open(page, '/how-it-works', 'How it works');
	for (const label of FILMS) {
		const film = page.getByRole('group', { name: label });
		await expect(film).toBeVisible();
		await expect(film.locator('.film-chapters > li')).toHaveCount(4);
		await expect(film.locator('.film-state')).toHaveText('1 / 4');
		// A finished picture is on show, not an empty screen.
		await expect(film.locator('.scene-stage > *').first()).toBeAttached();
	}
	const film = page.getByRole('group', { name: FILMS[0] });
	const next = film.getByRole('button', { name: 'Next step' });
	for (const step of ['2 / 4', '3 / 4', '4 / 4']) {
		await next.click();
		await expect(film.locator('.film-state')).toHaveText(step);
	}
	await expect(film.locator('.film-chapters [aria-current="step"]')).toHaveCount(1);
	// After the last chapter the button leads back to the first.
	await film.getByRole('button', { name: 'First step' }).click();
	await expect(film.locator('.film-state')).toHaveText('1 / 4');
});

test('sound is off until asked for, and the choice is remembered', async ({ page }) => {
	await open(page, '/how-it-works', 'How it works');
	const film = page.getByRole('group', { name: FILMS[0] });
	const sound = film.getByRole('button', { name: /Sound/ });
	await expect(sound).toHaveText('Sound off');
	await expect(sound).toHaveAttribute('aria-pressed', 'false');
	await sound.click();
	// One switch for all three films.
	for (const label of FILMS) await expect(page.getByRole('group', { name: label }).getByRole('button', { name: /Sound/ })).toHaveText('Sound on');
	await page.reload();
	await expect(page.getByRole('group', { name: FILMS[0] }).getByRole('button', { name: /Sound/ })).toHaveAttribute('aria-pressed', 'true');
	await page.getByRole('group', { name: FILMS[0] }).getByRole('button', { name: /Sound/ }).click();
	await expect(page.getByRole('group', { name: FILMS[0] }).getByRole('button', { name: /Sound/ })).toHaveText('Sound off');
});

test.describe('with motion on', () => {
	test.use({ contextOptions: { reducedMotion: 'no-preference' } });

	test('a film plays by itself when it is in view, and can be paused and carried on', async ({ page }) => {
		await open(page, '/how-it-works', 'How it works');
		const film = page.getByRole('group', { name: FILMS[0] });
		await film.scrollIntoViewIfNeeded();
		await expect(film.locator('.film-state')).toContainText('Playing 1/4');
		await film.getByRole('button', { name: 'Pause' }).click();
		await expect(film.locator('.film-state')).toContainText('Paused 1/4');
		// Paused means nothing moves: the caption stays as it is.
		const caption = await film.locator('.film-caption').textContent();
		await page.waitForTimeout(1200);
		expect(await film.locator('.film-caption').textContent()).toBe(caption);
		await film.getByRole('button', { name: 'Play' }).click();
		await expect(film.locator('.film-state')).toContainText('Playing');
	});
});

test('the page ends with the way on', async ({ page }) => {
	await open(page, '/how-it-works', 'How it works');
	await expect(page.getByRole('link', { name: 'Write your first strategy' })).toHaveAttribute('href', '/sign-up?next=%2Fstrategies%2Fnew');
	await page.getByRole('link', { name: 'Back to the start' }).click();
	await expect(page.getByRole('heading', { level: 1 })).toContainText('Write a strategy.');
});
