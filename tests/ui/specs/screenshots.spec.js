// The Arcade look, kept by a few screenshots: black and white, pixel type, solid and hollow blocks.
//
// A screenshot that differs fails the test and the report shows the two side by side. When the
// change is meant, take them again with `./test ui --update-snapshots screenshots` and look at
// the new pictures in tests/ui/screenshots before committing them.
import { expect, open, test, title } from '../fixtures.js';

/** Fonts in place and nothing moving, so the same page gives the same picture. */
async function settled(page) {
	await page.evaluate(() => document.fonts.ready);
	await expect.poll(() => page.evaluate(() => [...document.fonts].filter((f) => f.status === 'loading').length)).toBe(0);
}

test('the top of the front page', async ({ page }) => {
	await open(page, '/');
	await settled(page);
	await expect(page).toHaveScreenshot('front-page.png');
});

test('a run’s final table and head-to-head grid', async ({ page, account }) => {
	const run = await account.api.run({ iterations: 50, seed: 42 });
	await open(page, `/runs/${run.id}`, `Run ${run.id}`);
	await settled(page);
	// Only what is the same for every run of these four strategies: not the run's number or date.
	await expect(page.locator('.high-scores')).toHaveScreenshot('high-scores.png');
	await expect(page.locator('.head-to-head .matrix-box')).toHaveScreenshot('head-to-head.png');
});

test('the code editor before anything is typed', async ({ page, account }) => {
	await open(page, '/strategies/new/code', 'New code strategy');
	await settled(page);
	await expect(page.locator('form.editor .screen').first()).toHaveScreenshot('code-editor.png');
});
