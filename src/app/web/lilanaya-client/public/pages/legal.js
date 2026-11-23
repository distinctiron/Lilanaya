// The terms, the privacy notice, and refunds and cancelling.
//
// DRAFTS. They say what the site does today, in plain words, and are written to fit Danish and EU
// consumer rules as the author understands them. They have not been read by a lawyer. Have them
// checked, and fill in SELLER below, before taking money.

import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { legalLinks } from './plans.js';

/** Who sells. Empty values are shown as gaps to be filled, with a notice at the top of each page. */
export const SELLER = {
	name: '',
	// The Danish company registration number.
	cvr: '',
	address: '',
	email: ''
};

const UPDATED = '23 November 2026';

const gap = (value, what) => value || `[${what}]`;
const incomplete = () => Object.values(SELLER).some((value) => !value);

function seller() {
	return `${gap(SELLER.name, 'company name')}, CVR ${gap(SELLER.cvr, 'CVR number')}, ${gap(SELLER.address, 'address')}`;
}

const contact = () => gap(SELLER.email, 'contact email address');

/** A page of headed sections. Each section is `[heading, ...blocks]`; a block is a paragraph (string or nodes) or a list (`{ list: [...] }`). */
function page(title, intro, sections) {
	const block = (b) => (b && typeof b === 'object' && !Array.isArray(b) && b.list ? h('ul', { class: 'plain-list' }, b.list.map((item) => h('li', null, item))) : h('p', null, b));
	return async () => ({
		title,
		node: h('section', { class: 'stack-lg legal' }, [
			sectionHeading(title, { level: 1 }),
			incomplete() ? h('p', { class: 'error-box', 'data-role': 'legal-draft' }, 'Draft: the seller’s details are not filled in yet, and this text has not been checked by a lawyer.') : null,
			h('div', { class: 'stack prose' }, [h('p', { class: 'meta' }, `Last changed ${UPDATED}`), ...intro.map((text) => h('p', null, text))]),
			...sections.map(([heading, ...blocks]) => h('div', { class: 'stack prose' }, [sectionHeading(heading, { level: 2 }), ...blocks.map(block)])),
			legalLinks()
		])
	});
}

const link = (text, href) => h('a', { href }, text);

export const termsPage = page(
	'Terms',
	['These are the terms for using Lilanaya and for buying a subscription or a pack. By making an account you accept them.'],
	[
		['Who we are', `Lilanaya is run by ${seller()}. Write to us at ${contact()}.`],
		[
			'What Lilanaya is',
			'A place to write strategies for game-theory worlds and run them against each other in simulations. A strategy can be built by hand, written in code, or written by the AI from your description.',
			'We work to keep the site running and correct, but we do not promise that it is always available or free of faults. Simulations are games and experiments; do not rely on them for decisions that matter.'
		],
		[
			'Your account',
			{
				list: [
					'You log in with your email address and a password you choose, or through an account you have elsewhere. Keep the password to yourself, and keep access to the address: a link sent to it is how a new password is chosen.',
					'One account per person. You must be at least 18, or have a parent’s or guardian’s permission, to buy anything.',
					['You can delete your account at any time from your ', link('profile', '/profile'), '.']
				]
			}
		],
		[
			'Plans, packs and what is counted',
			['What each plan includes, what the packs contain and what they cost is on the ', link('Plans', '/plans'), ' page. That page is part of these terms.'],
			{
				list: [
					'A strategy is used each time the AI writes rules from your description, including when you ask it to write the same strategy again. Deleting a strategy does not give one back.',
					'If the AI cannot be reached, or your description is turned away before reaching it, nothing is used.',
					'A run is used each time you start a simulation.',
					'On the free plan the strategies the AI writes are counted per world and do not come back. A subscription’s strategies and runs are counted from the day of each payment to the next.',
					'A pack adds strategies and runs to your account. It is used only when your plan’s own are used up. It does not expire while you have your account, cannot be moved to another account or exchanged for money, and is lost if you delete your account.',
					'To keep the service working for everyone there are also limits that apply to all accounts, paid or not: how many strategies the AI writes for one account in an hour, how many it writes for all accounts together in a day, and how many runs one account has waiting at once. If such a limit stops you, nothing is used.'
				]
			}
		],
		[
			'Paying',
			{
				list: [
					'Prices are shown on the Plans page before you pay. VAT is added at checkout according to your country, and the total is shown before you confirm.',
					'Payment is handled by Stripe. We never see or store your card number.',
					'A subscription is paid in advance, a month at a time, and renews each month until you cancel it.',
					'If a renewal payment fails, Stripe tries again. Until it succeeds you keep the plan for at most a day after the paid month ends, without a new month’s strategies and runs; after that your account is on the free plan until the payment goes through.',
					'If we change the price of the subscription, we tell you by email at least 30 days before it applies to you, and you can cancel before then.'
				]
			}
		],
		[
			'Cancelling and refunds',
			['You can cancel a subscription at any time from your profile. It then runs to the end of the month you have paid for and is not renewed. Your right to withdraw from a purchase within 14 days, and when we refund, is described under ', link('Refunds and cancelling', '/refunds'), '.']
		],
		[
			'What you may not do',
			{
				list: [
					'Use the description box for anything but describing a strategy. Descriptions are screened before they reach the AI, and an account whose descriptions are turned away repeatedly is shut out of that feature for a day.',
					'Try to get around the limits, for example with several accounts, or to break or overload the service.',
					'Publish strategies whose names or descriptions are unlawful, hateful or harassing.'
				]
			},
			'We may suspend or close an account that does these things. If we close an account for another reason than a breach of these terms, we refund what is unused of what you have paid for.'
		],
		[
			'What you make',
			'Your strategies are yours. A strategy you make public is shown to everyone with your display name, and anyone may enter it in their runs. A private strategy is seen only by you. Results of runs that use only public strategies are public.'
		],
		[
			'Our responsibility',
			'If we fail to deliver what you paid for, you have the rights the law gives you, including to have the fault put right or to get money back. Beyond that, and as far as the law allows, we are not liable for indirect loss, and our total liability to you is limited to what you have paid us in the twelve months before the loss. Nothing here limits rights you have as a consumer that cannot be given up by agreement.'
		],
		[
			'Changes to these terms',
			'If we change these terms in a way that matters to you, we tell you by email at least 30 days ahead. If you do not accept the change you can cancel and delete your account.'
		],
		[
			'Law and complaints',
			'Danish law applies. If you are a consumer living in another EU country, you also keep the protection of the mandatory rules of your own country.',
			`If you have a complaint, write to us first at ${contact()}. If we cannot settle it, consumers can complain to the Danish Centre for Complaint Resolution (Center for Klageløsning, Nævnenes Hus, Toldboden 2, 8800 Viborg, naevneneshus.dk).`
		]
	]
);

export const refundsPage = page(
	'Refunds and cancelling',
	['How to cancel, what your right of withdrawal is, and when you get money back.'],
	[
		[
			'Cancelling a subscription',
			['Open your ', link('profile', '/profile'), ' and choose “Manage subscription and invoices”. Cancelling takes effect at the end of the month you have paid for: you keep the subscription until then, and no further payment is taken. There is no notice period and no fee.'],
			'Months that have been paid for and have begun are not refunded when you cancel, except as described below.'
		],
		[
			'Your 14-day right of withdrawal',
			'If you buy as a consumer in the EU, you may withdraw from a purchase within 14 days without giving a reason. For a subscription the 14 days run from your first payment; later monthly renewals do not start a new period.',
			'When you buy, you ask us to give you access straight away. That means:',
			{
				list: [
					'A subscription: if you withdraw within the 14 days, we refund the payment less a share for the days you have had the subscription, counted by the day.',
					'A pack you have not used anything from: we refund it in full.',
					'A pack you have used part of: the part you have used is not refunded. We refund the share of the price that matches what is left.'
				]
			},
			`To withdraw, write to ${contact()} from the email address of your account and say which purchase you are withdrawing from. You do not have to give a reason. We refund to the card you paid with within 14 days of hearing from you, and what the refund covers is then removed from your account.`
		],
		[
			'If something went wrong',
			`If you were charged twice, charged after cancelling, or could not use what you paid for because of a fault on our side, write to ${contact()}. We put it right or refund you. This is in addition to your right of withdrawal and to your rights under the law.`
		],
		['Deleting your account', 'Deleting your account ends your subscription at once and gives up what is left of your packs. It does not by itself give a refund; if you are within the 14 days, write to us before you delete the account.'],
		['Who you are buying from', `${seller()}.`]
	]
);

export const privacyPage = page(
	'Privacy',
	['What we know about you, why, who else handles it, and what you can do about it.'],
	[
		['Who is responsible', `${seller()} is the data controller. Write to us at ${contact()}.`],
		[
			'What we keep',
			{
				list: [
					'Your email address and the display name you choose. If you log in through Google, Apple, Facebook or GitHub, also that account’s id with them, and the name they give us the first time.',
					'Your password, only as a one-way fingerprint (hash) that lets us check it and cannot be turned back into the password.',
					'What you make: your strategies, the descriptions you gave the AI, and your runs and their results.',
					'What you have used: when the AI wrote a strategy for you and in which world, what the screening step concluded about the description, and when you started runs. This is how limits and plans are kept.',
					'What you have bought: your customer number at Stripe, the status and paid period of a subscription, and each pack with its date and amount. We never receive your card number.',
					'A session cookie that keeps you logged in on this device for up to 30 days. It is the only cookie, and it is necessary for the site to work. There are no advertising or statistics cookies and nothing on the pages is loaded from other sites.',
					'The server’s logs, which hold the address your requests came from, for a short time, to keep the service safe.'
				]
			}
		],
		[
			'Why, and on what ground',
			{
				list: [
					'To give you the service you signed up for and have paid for (performance of our agreement with you).',
					'To keep accounts for what was sold, as bookkeeping law requires of us (legal obligation).',
					'To stop misuse and keep costs in hand, by counting use and screening descriptions (our legitimate interest in a safe and working service).'
				]
			}
		],
		[
			'Who else handles it',
			'We use these companies to run the service. They handle your data only on our behalf or as needed to do their part.',
			{
				list: [
					'Render: runs the servers and the database, in Frankfurt, Germany.',
					'Stripe: takes payments, sends receipts and works out VAT. Stripe gets your email address, your country and address for VAT, and your card details, which you give to Stripe directly.',
					'Vercel AI Gateway and the AI model providers behind it: receive the description you write when you ask the AI for a strategy, and an id that tells accounts apart without saying who you are. Not your name or email address. We ask them to keep nothing of a request once it has been answered. This step may take place in the United States, under the EU’s standard contractual clauses or the EU–US Data Privacy Framework.',
					'Our email delivery service: receives your email address to send you the links for signing up and for choosing a new password.',
					'Google, Apple, Facebook or GitHub, only if you choose to log in through one of them.'
				]
			},
			'We do not sell your data and do not use it for advertising.'
		],
		[
			'What others can see',
			'A strategy you make public is shown with your display name. Your email address is never shown to other users. Choose a display name that does not give away more than you want.'
		],
		[
			'How long we keep it',
			{
				list: [
					'Your account and what is in it: until you delete the account.',
					'When you delete the account we delete your profile, your strategies, your private runs, your sessions and our record of what you bought. Public runs stay as part of the world’s history, without any link to you.',
					'One thing stays: a one-way fingerprint (hash) of your email address with the number of free strategies the AI had written for it, so that deleting an account and signing up again does not hand them out anew. It cannot be turned back into your address.',
					'Invoices and payment records are kept by Stripe and in our accounts for five years after the end of the financial year, as the Danish Bookkeeping Act requires.'
				]
			}
		],
		[
			'Your rights',
			[
				'You can see and take a copy of what we hold with “Download my data” on your ',
				link('profile', '/profile'),
				', correct your display name there, and delete everything with “Delete account”. You can also ask us to correct or restrict something, or object to how we use it, by writing to us.'
			],
			'If you think we handle your data wrongly, you can complain to the Danish Data Protection Agency (Datatilsynet, datatilsynet.dk), or to the authority in your own country.'
		]
	]
);
