import { ApiError, getJson, postJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { aiSentence, day, money, packWords, runs } from '../components/plan-words.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { getUser, refreshUser } from '../session.js';

/** The links every page about money ends with. */
export function legalLinks() {
	return h('p', { class: 'meta', 'data-role': 'legal-links' }, [h('a', { href: '/terms' }, 'Terms'), ' · ', h('a', { href: '/privacy' }, 'Privacy'), ' · ', h('a', { href: '/refunds' }, 'Refunds and cancelling')]);
}

/** Back from Stripe with `?paid=<session>`: ask the server what was paid. Returns the words to show, or null. */
async function settle(sessionId) {
	try {
		const { paid, item } = await postJson('/api/billing/confirm', { session_id: sessionId });
		await refreshUser();
		if (!paid) return { ok: false, text: 'The payment was not completed, so nothing was charged and nothing has changed.' };
		return { ok: true, text: item === 'subscription' ? 'Payment received. You are a subscriber now.' : 'Payment received. The pack has been added to your account.' };
	} catch (error) {
		if (!(error instanceof ApiError)) console.error(error);
		return { ok: false, text: 'We could not confirm the payment just now. If you paid, it will show on your profile within a few minutes.' };
	}
}

/**
 * What is free, what a subscription gives, and the packs, with their prices and the buttons to buy.
 * Open to everyone; buying needs an account.
 */
export async function plansPage({ query, replaceUrl, refresh }) {
	const paidSession = query.get('paid');
	// Say it once: a refresh should not ask again.
	if (paidSession) replaceUrl('/plans');
	let user = await getUser();
	const outcome = paidSession && user ? await settle(paidSession) : null;
	if (outcome) user = await getUser();
	const [plans, billing] = await Promise.all([getJson('/api/plans'), user ? getJson('/api/billing').catch(() => null) : null]);
	const { free, subscriber } = plans;
	const shop = plans.billing;
	const subscribed = billing?.subscription?.paid === true;

	const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
	const agree = h('input', { type: 'checkbox', id: 'plans-agree', name: 'agree' });
	const buttons = [];

	/** Sends the browser to Stripe to pay for `item`. */
	async function buy(item, pressed) {
		errors.hidden = true;
		if (!agree.checked) {
			errors.replaceChildren(h('p', null, 'Tick the box at the top first: it says what you agree to when you pay.'));
			errors.hidden = false;
			agree.focus();
			return;
		}
		for (const b of buttons) b.disabled = true;
		const label = pressed.textContent;
		pressed.textContent = 'One moment…';
		try {
			const { url } = await postJson('/api/billing/checkout', { item, agree: true });
			location.assign(url);
		} catch (error) {
			if (!(error instanceof ApiError)) console.error(error);
			errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'The payment page could not be opened.'));
			errors.hidden = false;
			for (const b of buttons) b.disabled = false;
			pressed.textContent = label;
		}
	}

	const buyButton = (label, item, primary = false) => {
		if (!user) return linkButton('Sign up to buy', `/sign-up?next=${encodeURIComponent('/plans')}`, { primary });
		const b = button(label, { primary, onclick: () => buy(item, b) });
		buttons.push(b);
		return b;
	};

	const vat = shop?.tax_added ? ' + VAT' : '';
	const planCard = (title, price, lines, action) =>
		h('div', { class: 'card stack', 'data-role': `plan-${title.toLowerCase()}` }, [
			sectionHeading(title, { level: 2 }),
			price ? h('p', null, h('strong', null, price)) : null,
			h('ul', { class: 'plain-list tight' }, lines.map((line) => h('li', null, line))),
			action ? h('div', { class: 'row' }, action) : null
		]);

	const byHand = 'Strategies you build by hand or write in code: no limit';
	const freeCard = planCard('Free', 'No charge', [aiSentence(free.ai_requests), `${runs(free.monthly_runs)} a month, ${runs(free.max_active_runs)} at a time`, byHand], user ? null : linkButton('Sign up', `/sign-up?next=${encodeURIComponent('/plans')}`));

	let subscribeAction = null;
	if (subscribed) {
		const s = billing.subscription;
		subscribeAction = [h('span', null, s.cancel_at_period_end ? `Yours until ${day(s.period_end)} · Not renewed` : `Yours · Renews on ${day(s.period_end)}`), linkButton('Manage on your profile', '/profile')];
	} else if (user?.plan === 'subscriber') {
		subscribeAction = [h('span', null, 'This is your plan')];
	} else if (shop) {
		subscribeAction = buyButton('Subscribe', 'subscription', true);
	}
	const subscriberCard = planCard(
		'Subscriber',
		shop ? `${money(shop.subscription.amount, shop.subscription.currency)} a ${shop.subscription.interval}${vat}` : null,
		[aiSentence(subscriber.ai_requests), `${runs(subscriber.monthly_runs)} a month, ${runs(subscriber.max_active_runs)} at a time`, byHand, 'Counted from the day you pay to the next payment · Cancel whenever you like, and keep it until the month you paid for ends'],
		subscribeAction
	);

	const packs = shop?.packs?.length
		? h('div', { class: 'stack' }, [
				sectionHeading('Packs', { level: 2 }),
				h('p', { class: 'prose' }, 'A pack adds to what your plan gives you, on any plan. It is used only when your plan’s own strategies or runs are used up, and it does not run out with time.'),
				h(
					'div',
					{ class: 'columns pack-list' },
					shop.packs.map((pack) => h('div', { class: 'card stack', 'data-role': `pack-${pack.id}` }, [h('p', null, h('strong', null, packWords(pack))), h('p', null, `${money(pack.amount, pack.currency)}${vat} · One payment`), h('div', null, buyButton('Buy', pack.id))]))
				)
			])
		: null;

	const mine =
		billing && (billing.packs.strategies_left > 0 || billing.packs.runs_left > 0)
			? h('p', { class: 'notice', 'data-role': 'pack-balance' }, `Left in your packs: ${packWords({ strategies: billing.packs.strategies_left, runs: billing.packs.runs_left })}`)
			: null;

	const consent =
		shop && user
			? h('label', { class: 'check', for: 'plans-agree' }, [
					agree,
					h('span', { class: 'check-text' }, [
						h('span', null, ['I accept the ', h('a', { href: '/terms' }, 'terms'), ' and ask for access to start straight away.']),
						h('span', { class: 'muted' }, ['If I use my 14-day right of withdrawal, what I have already used is not refunded. See ', h('a', { href: '/refunds' }, 'refunds and cancelling'), '.'])
					])
				])
			: null;

	return {
		title: 'Plans',
		node: h('section', { class: 'stack-lg' }, [
			sectionHeading('Plans', { level: 1 }),
			outcome ? h('p', { class: outcome.ok ? 'notice' : 'error-box', role: 'status', 'data-role': 'payment-outcome' }, outcome.text) : null,
			paidSession && !user ? h('p', { class: 'error-box', role: 'status' }, 'Log in to see what your payment added.') : null,
			mine,
			// What is agreed to comes before the buttons that take it as agreed.
			consent,
			errors,
			h('div', { class: 'columns' }, [freeCard, subscriberCard]),
			packs,
			!shop ? h('p', { class: 'notice', 'data-role': 'shop-closed' }, 'Nothing is for sale at the moment. The free plan works as described.') : null,
			h('div', { class: 'stack' }, [
				sectionHeading('How it is counted', { level: 2 }),
				h('ul', { class: 'plain-list prose' }, [
					h('li', null, 'One strategy is used every time you press “Turn into rules” and the AI writes rules from your description. Asking it to write the same strategy again uses another.'),
					h('li', null, 'Deleting a strategy does not give one back: the AI has already done the work.'),
					h('li', null, 'If the AI cannot be reached, or your description is turned away before it gets there, nothing is used.'),
					h('li', null, 'Every run you start uses one run, whatever comes of it.'),
					shop ? h('li', null, `Prices are in ${shop.subscription.currency.toUpperCase()}${shop.tax_added ? '. VAT is added at checkout according to your country' : ''}. You pay on Stripe’s page; we never see your card.`) : null
				])
			]),
			legalLinks(),
			// After a payment the header and the profile should show the new plan too.
			outcome?.ok ? h('p', null, button('See your profile', { onclick: () => (location.href = '/profile') })) : null,
			outcome && !outcome.ok ? h('p', null, button('Check again', { onclick: () => refresh() })) : null
		])
	};
}
