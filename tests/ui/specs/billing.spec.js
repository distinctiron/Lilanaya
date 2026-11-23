// Paying: the plans page, subscribing, buying a pack, cancelling, and leaving.
// Stripe is a stand-in (tests/ui/fake-stripe): its Checkout page and portal are plain pages with buttons.
import { expect, open, test, title } from '../fixtures.js';

const main = (page) => page.locator('#main');
const outcome = (page) => page.locator('[data-role="payment-outcome"]');

/** Ticks the box and presses the buy button inside `card`; ends on the stand-in Checkout page. */
async function goToPay(page, card, label) {
	await page.getByRole('checkbox', { name: /I accept the terms/ }).check();
	await page.locator(`[data-role="${card}"]`).getByRole('button', { name: label }).click();
	await expect(title(page)).toHaveText('Stand-in for Stripe Checkout');
}

test('the plans page tells a visitor what is free, what is for sale and what it costs', async ({ page }) => {
	await open(page, '/plans', 'Plans');
	await expect(page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Plans' })).toHaveAttribute('aria-current', 'page');
	const free = page.locator('[data-role="plan-free"]');
	await expect(free).toContainText('No charge');
	await expect(free).toContainText('The AI writes 3 strategies for you in each world. They never come back.');
	const subscriber = page.locator('[data-role="plan-subscriber"]');
	await expect(subscriber).toContainText('€8.99 a month + VAT');
	await expect(subscriber).toContainText('The AI writes 20 strategies a month for you, across all worlds.');
	await expect(subscriber).toContainText('1,000 runs a month');
	await expect(page.locator('[data-role="pack-small"]')).toContainText('10 strategies + 100 runs');
	await expect(page.locator('[data-role="pack-small"]')).toContainText('€5.99 + VAT · One payment');
	await expect(page.locator('[data-role="pack-medium"]')).toContainText('20 strategies + 200 runs');
	await expect(page.locator('[data-role="pack-medium"]')).toContainText('€9.99');
	await expect(page.locator('[data-role="pack-large"]')).toContainText('50 strategies + 500 runs');
	await expect(page.locator('[data-role="pack-large"]')).toContainText('€21.99');
	await expect(main(page)).toContainText('Deleting a strategy does not give one back');
	// The site speaks of strategies, not of requests.
	await expect(main(page)).not.toContainText(/request/i);

	// Buying takes an account: a visitor is sent to sign up, and comes back here.
	await expect(page.getByRole('button', { name: 'Buy' })).toHaveCount(0);
	await expect(subscriber.getByRole('link', { name: 'Sign up to buy' })).toHaveAttribute('href', '/sign-up?next=%2Fplans');
});

test('the terms, the privacy notice and the refund rules are a click away', async ({ page }) => {
	await open(page, '/plans', 'Plans');
	const links = page.locator('[data-role="legal-links"]');
	await links.getByRole('link', { name: 'Terms' }).click();
	await expect(title(page)).toHaveText('Terms');
	await expect(main(page)).toContainText('A strategy is used each time the AI writes rules from your description');
	await page.locator('[data-role="legal-links"]').getByRole('link', { name: 'Privacy' }).click();
	await expect(title(page)).toHaveText('Privacy');
	await expect(main(page)).toContainText('We never receive your card number');
	await expect(main(page)).toContainText('Download my data');
	await page.locator('[data-role="legal-links"]').getByRole('link', { name: 'Refunds and cancelling' }).click();
	await expect(title(page)).toHaveText('Refunds and cancelling');
	await expect(main(page)).toContainText('you may withdraw from a purchase within 14 days');
	await expect(page).toHaveURL(/\/refunds$/);
});

test('subscribing: the tick, paying at Stripe, a subscriber on return, and cancelling', async ({ page, account }) => {
	await open(page, '/plans', 'Plans');
	// Nothing is started without the tick in the box.
	await page.locator('[data-role="plan-subscriber"]').getByRole('button', { name: 'Subscribe' }).click();
	await expect(page.getByRole('alert')).toContainText('Tick the box at the top first');
	await expect(page).toHaveURL(/\/plans$/);

	await goToPay(page, 'plan-subscriber', 'Subscribe');
	await expect(page.locator('[data-role="item"]')).toHaveText('subscription');
	await expect(page.locator('[data-role="amount"]')).toContainText('8.99 EUR a month');
	// Said again where the money is paid.
	await expect(page.locator('[data-role="note"]')).toContainText('right of withdrawal');
	await page.getByRole('button', { name: 'Pay', exact: true }).click();

	await expect(title(page)).toHaveText('Plans');
	await expect(outcome(page)).toHaveText('Payment received. You are a subscriber now.');
	// The session's id is taken out of the address bar: a reload does not ask again.
	await expect(page).toHaveURL(/\/plans$/);
	await expect(page.locator('[data-role="plan-subscriber"]')).toContainText(/Yours · Renews on \d+ \w+ \d{4}/);
	await expect(page.locator('[data-role="plan-subscriber"]').getByRole('button', { name: 'Subscribe' })).toHaveCount(0);

	await open(page, '/profile', 'Profile');
	await expect(main(page).locator('dl.rules')).toContainText('Subscriber');
	await expect(page.locator('[data-role="plan-status"]')).toHaveText(/Subscriber · Paid until \d+ \w+ \d{4} · Renews then/);
	await expect(main(page)).toContainText('1000 left · 0 of 1000 used');
	await expect(main(page)).toContainText('20 left · 0 of 20 used');
	const usage = await account.api.get('/api/usage');
	expect(usage.ai_requests.per).toBe('month');

	// Cancelling is done at Stripe: the subscription stays until the month that was paid for ends.
	await page.getByRole('button', { name: 'Manage subscription and invoices' }).click();
	await expect(title(page)).toHaveText('Stand-in for Stripe’s customer portal');
	await page.getByRole('button', { name: 'Cancel subscription' }).click();
	await expect(title(page)).toHaveText('Profile');
	await expect(page.locator('[data-role="plan-status"]')).toHaveText(/Subscriber until \d+ \w+ \d{4} · Cancelled, so it is not renewed/);
	await expect(main(page).locator('dl.rules')).toContainText('Subscriber');
});

test('a pack bought when the plan’s runs are used up lets the next run start', async ({ page, account }) => {
	// The test site gives a free account three runs a month.
	for (let i = 0; i < 3; i++) await account.api.run({ iterations: 5 });
	await open(page, '/runs/new', 'New run');
	await page.getByRole('button', { name: 'Run simulation' }).click();
	const refused = page.getByRole('alert').filter({ hasText: 'You have used all 3 runs' });
	await expect(refused).toContainText('A pack on the Plans page gives you more straight away.');
	await refused.getByRole('link', { name: 'See plans and packs' }).click();
	await expect(title(page)).toHaveText('Plans');

	await goToPay(page, 'pack-small', 'Buy');
	await expect(page.locator('[data-role="item"]')).toHaveText('small');
	// Stripe's own message is late: the site finds out when the browser comes back.
	await page.getByRole('button', { name: 'Pay, message arrives late' }).click();
	await expect(outcome(page)).toHaveText('Payment received. The pack has been added to your account.');
	await expect(page.locator('[data-role="pack-balance"]')).toHaveText('Left in your packs: 10 strategies + 100 runs');

	// The run that was refused now starts, and comes out of the pack.
	const run = await account.api.run({ iterations: 5 });
	expect(run.status).toBe('completed');
	await open(page, '/profile', 'Profile');
	await expect(page.locator('[data-role="pack-balance"]')).toHaveText('Left in your packs · 10 strategies + 99 runs');
	// What was paid, VAT included, with its date.
	await expect(page.locator('[data-role="purchases"]')).toContainText(/\d+ \w+ \d{4} · 10 strategies \+ 100 runs · €7\.49/);
	// A pack is not a subscription.
	await expect(page.locator('[data-role="plan-status"]')).toHaveText('Free');
	await expect(main(page)).toContainText('0 left · 3 of 3 used');
});

test('a pack lets the AI go on writing when the free strategies are used up', async ({ page, account }) => {
	await open(page, '/strategies/new/rules', 'New rules strategy');
	const allowance = page.locator('[data-role="ai-strategies"]');
	await page.getByLabel('Describe your strategy').fill('Cooperate until the opponent defects once, then defect for ever.');
	for (let i = 0; i < 3; i++) {
		await page.getByRole('button', { name: 'Turn into rules' }).click();
		await expect(allowance).toContainText(i < 2 ? `The AI can write ${2 - i} of 3 more` : 'The AI has written all 3 strategies your plan gives you');
	}
	await expect(page.getByRole('button', { name: 'Turn into rules' })).toBeDisabled();
	await allowance.getByRole('link', { name: 'Get more' }).click();
	await expect(title(page)).toHaveText('Plans');
	await goToPay(page, 'pack-small', 'Buy');
	await page.getByRole('button', { name: 'Pay', exact: true }).click();
	await expect(outcome(page)).toContainText('Payment received');

	await open(page, '/strategies/new/rules', 'New rules strategy');
	await expect(allowance).toContainText("Your plan’s own are used up in Repeated Prisoner's Dilemma · 10 strategies left in your packs");
	await page.getByLabel('Describe your strategy').fill('Cooperate until the opponent defects once, then defect for ever.');
	await page.getByRole('button', { name: 'Turn into rules' }).click();
	await expect(allowance).toContainText('9 strategies left in your packs');
});

test('@safety leaving Stripe without paying gives nothing', async ({ page, account }) => {
	await open(page, '/plans', 'Plans');
	await goToPay(page, 'pack-large', 'Buy');
	const session = page.url().split('/').pop();
	await page.getByRole('button', { name: 'Go back without paying' }).click();
	await expect(title(page)).toHaveText('Plans');
	await expect(outcome(page)).toHaveCount(0);
	// Coming "back" with the id of a session that was never paid, or one that does not exist.
	await open(page, `/plans?paid=${session}`, 'Plans');
	await expect(outcome(page)).toHaveText('The payment was not completed, so nothing was charged and nothing has changed.');
	await open(page, '/plans?paid=cs_test_made_up', 'Plans');
	await expect(outcome(page)).toContainText('We could not confirm the payment just now');
	const billing = await account.api.get('/api/billing');
	expect(billing.plan).toBe('free');
	expect(billing.packs).toEqual({ strategies_left: 0, runs_left: 0 });
	expect(billing.purchases).toEqual([]);
});

test('@safety nobody can use another person’s payment, or a message Stripe did not sign', async ({ page, account, someone }) => {
	await open(page, '/plans', 'Plans');
	await goToPay(page, 'pack-large', 'Buy');
	const session = page.url().split('/').pop();
	await page.getByRole('button', { name: 'Pay, message arrives late' }).click();
	await expect(outcome(page)).toContainText('Payment received');

	// Someone else comes "back from Stripe" with the same session.
	const other = await someone();
	await open(other.page, `/plans?paid=${session}`, 'Plans');
	await expect(outcome(other.page)).toContainText('We could not confirm the payment just now');
	expect((await other.api.get('/api/billing')).packs).toEqual({ strategies_left: 0, runs_left: 0 });
	expect((await account.api.get('/api/billing')).packs).toEqual({ strategies_left: 50, runs_left: 500 });

	// A message to the webhook without Stripe's signature, or with a made-up one, is refused.
	const body = JSON.stringify({ id: 'evt_forged', type: 'checkout.session.completed', data: { object: { id: session, customer: 'cus_x' } } });
	for (const headers of [{}, { 'stripe-signature': `t=${Math.floor(Date.now() / 1000)},v1=${'0'.repeat(64)}` }]) {
		const res = await other.context.request.post('/api/billing/webhook', { data: body, headers: { 'content-type': 'application/json', ...headers } });
		expect(res.status()).toBe(400);
		expect((await res.json()).error.code).toBe('invalid_signature');
	}
	expect((await other.api.get('/api/billing')).packs).toEqual({ strategies_left: 0, runs_left: 0 });
});

test('a user can download their data and delete their account', async ({ page, account }) => {
	await account.api.strategy({ name: 'Mine to take away' });
	await open(page, '/profile', 'Profile');
	const download = page.getByRole('link', { name: 'Download my data' });
	await expect(download).toHaveAttribute('href', '/api/me/export');
	const data = await (await page.request.get('/api/me/export')).json();
	expect(data.account.email).toBe(account.email);
	expect(data.strategies.map((s) => s.doc.name)).toEqual(['Mine to take away']);

	await page.getByRole('button', { name: 'Delete account…' }).click();
	await expect(main(page)).toContainText('It cannot be undone.');
	// Deleting takes the account's own address.
	await page.getByLabel('Your email address').fill('someone-else@example.com');
	await page.getByRole('button', { name: 'Delete my account for good' }).click();
	await expect(page.locator('[data-role="leaving"]').getByRole('alert')).toHaveText('That is not this account’s email address.');
	expect((await account.api.get('/api/me')).email).toBe(account.email);

	await page.getByLabel('Your email address').fill(account.email);
	await page.getByRole('button', { name: 'Delete my account for good' }).click();
	await expect(page).toHaveURL(/\/$/);
	await expect(page.getByRole('link', { name: 'Log in' })).toBeVisible();
	expect((await page.request.get('/api/me')).status()).toBe(401);
	expect((await (await page.request.get('/api/strategies')).json()).strategies.some((s) => s.name === 'Mine to take away')).toBe(false);
});
