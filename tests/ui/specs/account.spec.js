// Accounts: signing up and logging in by emailed link, the profile, logging out.
import { expect, open, test, title } from '../fixtures.js';

test('signing up: what an account includes, the emailed link, and the first look at the profile', async ({ page, address, mail }) => {
	await open(page, '/sign-up', 'Sign up');
	// The limits are told before anyone signs up, with this server's numbers.
	const plan = page.locator('[data-role="plan-summary"]');
	await expect(plan).toContainText('3 AI requests per world. They never come back.');
	await expect(plan).toContainText('Deleting a strategy does not give one back.');
	await expect(plan).toContainText('3 runs a month, 2 runs at a time.');
	await expect(plan).toContainText('Subscribers get 20 AI requests a month, across all worlds.');

	await page.getByLabel('Email address').fill(`  ${address.toUpperCase()} `);
	await page.getByRole('button', { name: 'Sign up' }).click();
	await expect(title(page)).toHaveText('Check your email');
	await expect(title(page)).toBeFocused();
	await expect(page.locator('#main')).toContainText('The link works once · Valid for 15 minutes');

	const email = await mail.next(address);
	expect(email.subject).toBe('Finish signing up to Lilanaya');
	expect(email.text).toContain('It works once and for 15 minutes.');
	await page.goto(mail.linkIn(email));
	await expect(title(page)).toHaveText('Almost there');
	// The one-time token is taken out of the address bar at once.
	await expect(page).toHaveURL(/\/log-in\/confirm$/);
	await page.getByRole('button', { name: 'Continue' }).click();

	await expect(title(page)).toHaveText('Profile');
	await expect(page).toHaveURL(/\/profile$/);
	const main = page.locator('#main');
	await expect(main.locator('dl.rules')).toContainText(address);
	await expect(main.locator('dl.rules')).toContainText('Free');
	await expect(main).toContainText('3 left · 0 of 3 used');
	await expect(main).toContainText('2 runs at a time');
	await expect(main).toContainText('3 per world · They never come back');
	await expect(main).toContainText('None used yet');
	await expect(main).toContainText('Private worlds · Subscribers only');
	// The header now shows who is logged in instead of the way in.
	const name = address.split('@')[0];
	await expect(page.getByRole('link', { name: `Profile of ${name}` })).toBeVisible();
	await expect(page.getByRole('link', { name: 'Log in' })).toHaveCount(0);

	// The session outlives a reload.
	await page.reload();
	await expect(page.getByRole('link', { name: `Profile of ${name}` })).toBeVisible();
});

test('an emailed link works once', async ({ page, address, mail, someone }) => {
	await open(page, '/sign-up', 'Sign up');
	await page.getByLabel('Email address').fill(address);
	await page.getByRole('button', { name: 'Sign up' }).click();
	await expect(title(page)).toHaveText('Check your email');
	const link = mail.linkIn(await mail.next(address));
	await page.goto(link);
	await page.getByRole('button', { name: 'Continue' }).click();
	await expect(title(page)).toHaveText('Profile');

	// Someone else who got hold of the same link gets nowhere with it.
	const thief = await someone({ signedIn: false });
	await thief.page.goto(link);
	await thief.page.getByRole('button', { name: 'Continue' }).click();
	await expect(thief.page.getByRole('alert')).toContainText('This link has expired or was already used. Ask for a new one.');
	await expect(thief.page.getByRole('link', { name: 'Ask for a new link' })).toBeVisible();
	await thief.page.goto('/profile');
	await expect(title(thief.page)).toHaveText('Log in first');
});

test('logging out, and logging in again with a new link', async ({ page, account, mail }) => {
	await open(page, '/profile', 'Profile');
	await page.getByRole('button', { name: 'Log out' }).click();
	await expect(title(page)).toHaveText('Worlds');
	await expect(page.getByRole('link', { name: 'Log in' })).toBeVisible();
	await expect(page.getByRole('link', { name: 'Sign up' })).toBeVisible();
	// Logged out means logged out: the profile asks for a log-in.
	await open(page, '/profile', 'Log in first');

	await page.getByRole('link', { name: 'Log in' }).first().click();
	await expect(title(page)).toHaveText('Log in');
	await page.getByLabel('Email address').fill(account.email);
	await page.getByRole('button', { name: 'Email me a link' }).click();
	await expect(title(page)).toHaveText('Check your email');
	// The page says the same whether or not the address has an account.
	await expect(page.locator('#main')).toContainText('If the address has an account, it holds your log-in link.');
	const email = await mail.next(account.email, 1);
	expect(email.subject).toBe('Your Lilanaya log-in link');
	await page.goto(mail.linkIn(email));
	await page.getByRole('button', { name: 'Continue' }).click();
	// Back where the log-in was asked for.
	await expect(title(page)).toHaveText('Profile');
	await expect(page.locator('#main dl.rules')).toContainText(account.email);
});

test('logging in with an address that has no account makes none', async ({ page, address, mail }) => {
	await open(page, '/log-in', 'Log in');
	await page.getByLabel('Email address').fill(address);
	await page.getByRole('button', { name: 'Email me a link' }).click();
	await expect(title(page)).toHaveText('Check your email');
	const email = await mail.next(address);
	expect(email.subject).toBe('No Lilanaya account for this address yet');
	expect(email.text).toContain('/sign-up');
	expect(email.text).not.toContain('token=');
});

test('a page that needs an account sends the visitor back to it after logging in', async ({ page, address, mail }) => {
	await open(page, '/runs/new', 'Log in first');
	await expect(page.locator('#main')).toContainText('You need to be logged in to start a run.');
	await page.locator('#main').getByRole('link', { name: 'Sign up' }).click();
	await expect(page).toHaveURL(/\/sign-up\?next=%2Fruns%2Fnew$/);
	await page.getByLabel('Email address').fill(address);
	await page.getByRole('button', { name: 'Sign up' }).click();
	await expect(title(page)).toHaveText('Check your email');
	await page.goto(mail.linkIn(await mail.next(address)));
	await page.getByRole('button', { name: 'Continue' }).click();
	await expect(title(page)).toHaveText('New run');
	await expect(page).toHaveURL(/\/runs\/new$/);
});

test('an address that is not one is stopped before anything is sent', async ({ page }) => {
	await open(page, '/sign-up', 'Sign up');
	const email = page.getByLabel('Email address');
	for (const wrong of ['', 'not-an-address', 'two@@example.com']) {
		await email.fill(wrong);
		await page.getByRole('button', { name: 'Sign up' }).click();
		expect(await email.evaluate((el) => el.validity.valid)).toBe(false);
		await expect(title(page)).toHaveText('Sign up');
	}
	// One the browser lets through and the server does not.
	await email.fill('someone@nowhere');
	await page.getByRole('button', { name: 'Sign up' }).click();
	await expect(page.getByRole('alert')).toHaveText('Enter a valid email address');
	await expect(email).toBeFocused();
	await expect(page.getByRole('button', { name: 'Sign up' })).toBeEnabled();
});

test('a link without its token, and a failed sign-in through another site, are explained', async ({ page }) => {
	await open(page, '/log-in/confirm', 'This link is not complete');
	await page.getByRole('link', { name: 'Ask for a new link' }).click();
	await expect(title(page)).toHaveText('Log in');

	await open(page, '/log-in?error=cancelled', 'Log in');
	await expect(page.getByRole('alert').first()).toHaveText('That was cancelled. Nothing was changed.');
	// Said once: the address is cleaned, so a reload does not repeat it.
	await expect(page).toHaveURL(/\/log-in$/);
	await page.reload();
	await expect(title(page)).toHaveText('Log in');
	await expect(page.getByRole('alert').filter({ hasText: 'cancelled' })).toHaveCount(0);

	// The old addresses from earlier emails still work.
	await open(page, '/sign-in', 'Log in');
	// With no provider switched on, none is offered.
	await expect(page.getByRole('link', { name: /Continue with/ })).toHaveCount(0);
});

test('the display name can be changed and is checked', async ({ page, account }) => {
	await open(page, '/profile', 'Profile');
	const name = page.getByLabel('Display name');
	await expect(name).toHaveValue(account.user.display_name);
	await name.fill('  Countess Ada  ');
	await page.getByRole('button', { name: 'Save name' }).click();
	await expect(page.getByRole('status')).toHaveText('Saved');
	// The header follows at once.
	await expect(page.getByRole('link', { name: 'Profile of Countess Ada' })).toBeVisible();
	await page.reload();
	await expect(page.getByLabel('Display name')).toHaveValue('Countess Ada');

	await page.getByLabel('Display name').fill('   ');
	await page.getByRole('button', { name: 'Save name' }).click();
	await expect(page.getByRole('alert')).toHaveText('Display name must be 1 to 60 characters.');
	await expect(page.getByRole('link', { name: 'Profile of Countess Ada' })).toBeVisible();
});

test('a logged-in user who opens the sign-up page is sent on', async ({ page, account }) => {
	await page.goto('/sign-up');
	await expect(title(page)).toHaveText('Profile');
	await page.goto('/log-in?next=%2Fstrategies');
	await expect(title(page)).toHaveText('Strategies');
});

test('a subscriber sees a monthly allowance that comes back', async ({ page, account }) => {
	// Nobody can become a subscriber through the site yet (plans are set by hand), so the answer is stood in for.
	const subscriber = {
		...account.user,
		plan: 'subscriber',
		entitlements: { plan: 'subscriber', analytics_access: true, quick_test_access: true, private_worlds: true },
		usage: { ...account.user.usage, monthly_runs: 1000, used: 12, remaining: 988, ai_requests: { limit: 20, per: 'month', used_by_world: { 'prisoners-dilemma': 5 }, used_this_month: 5 } }
	};
	await page.route('**/api/session', (route) => route.fulfill({ json: { user: subscriber } }));
	await open(page, '/profile', 'Profile');
	const main = page.locator('#main');
	await expect(main.locator('dl.rules')).toContainText('Subscriber');
	await expect(main).toContainText('988 left · 12 of 1000 used');
	await expect(main).toContainText('15 left · 5 of 20 used this month');
	await expect(main).toContainText('Across all worlds · Resets on the 1st');
	await expect(main).toContainText('Private worlds · Included');
});
