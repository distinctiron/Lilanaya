// Stands in for Stripe in the UI tests: its API, its Checkout page and its customer portal.
// No dependencies; run with `node server.mjs`. Nothing here is real money.
//
// The site sends the browser to http://stripe:8091/pay/<session>, a page with three buttons:
//
//   "Pay"                         the session is paid; Stripe's message (webhook) is sent to the site, signed;
//                                 the browser goes back to the site's success address
//   "Pay, message arrives late"   the same, but no message is sent: the site must find out when the browser returns
//   "Go back without paying"      the browser goes back to the site's cancel address; nothing is paid
//
// The portal (http://stripe:8091/portal/<customer>) has one button, "Cancel subscription": the subscription
// ends with the paid period, the site is told, and the browser goes back.
//
// POST /__refund {"payment_intent"}   pays a pack's money back and tells the site
// GET  /__calls                        every request the site made to the API: [{ method, path, form }]
import { createHmac } from 'node:crypto';
import { createServer } from 'node:http';

const PORT = 8091;
const KEY = 'sk_test_fake';
const WEBHOOK_SECRET = process.env.WEBHOOK_SECRET ?? 'whsec_test';
const WEBHOOK_URL = process.env.WEBHOOK_URL ?? 'http://web/api/billing/webhook';
const HERE = process.env.STRIPE_PUBLIC_URL ?? `http://stripe:${PORT}`;
const DAY = 24 * 60 * 60;

const PRICES = {
	price_sub: { unit_amount: 899, recurring: { interval: 'month' } },
	price_small: { unit_amount: 599, recurring: null },
	price_medium: { unit_amount: 999, recurring: null },
	price_large: { unit_amount: 2199, recurring: null }
};

const sessions = new Map();
const subscriptions = new Map();
const calls = [];
let next = 1;
const id = (prefix) => `${prefix}_test_${Date.now().toString(36)}${next++}`;
const now = () => Math.floor(Date.now() / 1000);

function read(req) {
	return new Promise((resolve) => {
		let body = '';
		req.on('data', (chunk) => (body += chunk));
		req.on('end', () => resolve(body));
	});
}

const escape = (text) => String(text).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

function page(res, title, body) {
	res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
	res.end(`<!doctype html><html lang="en"><head><meta charset="utf-8"><title>${escape(title)}</title></head><body><main><h1>${escape(title)}</h1>${body}</main></body></html>`);
}

function json(res, status, body) {
	res.writeHead(status, { 'content-type': 'application/json' });
	res.end(JSON.stringify(body));
}

const missing = (res) => json(res, 404, { error: { message: 'No such object' } });

function redirect(res, to) {
	res.writeHead(303, { location: to });
	res.end();
}

/** Sends the site a message as Stripe does: signed over the time and the body. */
async function tell(type, object) {
	const body = JSON.stringify({ id: id('evt'), type, data: { object } });
	const time = now();
	const signature = createHmac('sha256', WEBHOOK_SECRET).update(`${time}.${body}`).digest('hex');
	const res = await fetch(WEBHOOK_URL, { method: 'POST', headers: { 'content-type': 'application/json', 'stripe-signature': `t=${time},v1=${signature}` }, body });
	if (!res.ok) console.error(`the site answered ${res.status} to ${type}: ${await res.text()}`);
}

/** The buyer pays: the session is complete, and what it bought exists. */
function pay(session) {
	session.status = 'complete';
	session.payment_status = 'paid';
	const price = PRICES[session.price];
	// With VAT on top, as Stripe Tax would add it for a Danish buyer.
	session.amount_total = Math.round(price.unit_amount * 1.25);
	if (session.mode === 'subscription') {
		const sub = { id: id('sub'), object: 'subscription', customer: session.customer, status: 'active', current_period_start: now() - 5, current_period_end: now() + 30 * DAY, cancel_at_period_end: false, latest_invoice: { id: id('in'), status: 'paid' } };
		subscriptions.set(sub.id, sub);
		session.subscription = sub.id;
	} else {
		session.payment_intent = id('pi');
	}
}

const server = createServer(async (req, res) => {
	const url = new URL(req.url, HERE);
	const path = url.pathname;
	const body = req.method === 'GET' ? '' : await read(req);

	// --- what the browser sees ---
	const payPage = path.match(/^\/pay\/(\w+)$/);
	if (payPage) {
		const session = sessions.get(payPage[1]);
		if (!session) return page(res, 'No such payment', '');
		if (req.method === 'GET') {
			const price = PRICES[session.price];
			return page(
				res,
				'Stand-in for Stripe Checkout',
				`<p data-role="item">${escape(session.metadata.item)}</p><p data-role="amount">${(price.unit_amount / 100).toFixed(2)} EUR${price.recurring ? ' a month' : ''}, VAT on top</p><p data-role="note">${escape(session.note)}</p>
				<form method="post"><button name="how" value="pay">Pay</button> <button name="how" value="late">Pay, message arrives late</button> <button name="how" value="back">Go back without paying</button></form>`
			);
		}
		const how = new URLSearchParams(body).get('how');
		if (how === 'back') return redirect(res, session.cancel_url);
		pay(session);
		if (how === 'pay') await tell('checkout.session.completed', { id: session.id, object: 'checkout.session', customer: session.customer });
		return redirect(res, session.success_url.replace('{CHECKOUT_SESSION_ID}', session.id));
	}
	const portalPage = path.match(/^\/portal\/(\w+)$/);
	if (portalPage) {
		const back = url.searchParams.get('return');
		if (req.method === 'GET') return page(res, 'Stand-in for Stripe’s customer portal', `<form method="post"><button>Cancel subscription</button></form><p><a href="${escape(back)}">Back</a></p>`);
		for (const sub of subscriptions.values()) {
			if (sub.customer === portalPage[1] && sub.status === 'active') {
				sub.cancel_at_period_end = true;
				await tell('customer.subscription.updated', { id: sub.id, object: 'subscription', customer: sub.customer });
			}
		}
		return redirect(res, back);
	}

	// --- what the tests ask ---
	if (path === '/__calls') return json(res, 200, calls);
	if (path === '/__refund' && req.method === 'POST') {
		const { payment_intent } = JSON.parse(body);
		const session = [...sessions.values()].find((s) => s.payment_intent === payment_intent);
		if (!session) return missing(res);
		await tell('charge.refunded', { id: id('ch'), object: 'charge', customer: session.customer, payment_intent, refunded: true, amount: session.amount_total, amount_refunded: session.amount_total });
		return json(res, 200, { refunded: true });
	}

	// --- the API the site calls ---
	if (!path.startsWith('/v1/')) return missing(res);
	if (req.headers.authorization !== `Bearer ${KEY}` || !req.headers['stripe-version']) return json(res, 401, { error: { message: 'Invalid API Key provided' } });
	const form = Object.fromEntries(new URLSearchParams(body));
	if (req.method !== 'GET') calls.push({ method: req.method, path, form });

	if (path === '/v1/customers' && req.method === 'POST') return json(res, 200, { id: id('cus'), object: 'customer', email: form.email });
	if (path === '/v1/checkout/sessions' && req.method === 'POST') {
		const session = {
			id: id('cs'),
			object: 'checkout.session',
			status: 'open',
			payment_status: 'unpaid',
			mode: form.mode,
			customer: form.customer,
			client_reference_id: form.client_reference_id,
			metadata: { item: form['metadata[item]'], user_id: form['metadata[user_id]'] },
			subscription: null,
			payment_intent: null,
			amount_total: 0,
			currency: 'eur',
			price: form['line_items[0][price]'],
			note: form['custom_text[submit][message]'] ?? '',
			success_url: form.success_url,
			cancel_url: form.cancel_url
		};
		if (!PRICES[session.price]) return json(res, 400, { error: { message: `No such price: ${session.price}` } });
		session.url = `${HERE}/pay/${session.id}`;
		sessions.set(session.id, session);
		return json(res, 200, session);
	}
	const one = path.match(/^\/v1\/(checkout\/sessions|subscriptions|prices)\/(\w+)$/);
	if (one) {
		const [, kind, which] = one;
		if (kind === 'prices') return PRICES[which] ? json(res, 200, { id: which, object: 'price', currency: 'eur', ...PRICES[which] }) : missing(res);
		const found = (kind === 'subscriptions' ? subscriptions : sessions).get(which);
		if (!found) return missing(res);
		if (req.method === 'DELETE') found.status = 'canceled';
		return json(res, 200, found);
	}
	if (path === '/v1/billing_portal/sessions' && req.method === 'POST') return json(res, 200, { id: id('bps'), url: `${HERE}/portal/${form.customer}?return=${encodeURIComponent(form.return_url)}` });
	return missing(res);
});

server.listen(PORT, () => console.log(`stand-in for Stripe listening on ${PORT}`));
