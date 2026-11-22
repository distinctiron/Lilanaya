import { ApiError, getJson, postJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { getUser, setUser } from '../session.js';

const NEXT_KEY = 'lilanaya.next';

/** Only ever send people back to a page of this site. */
const safeNext = (value) => (value && value.startsWith('/') && !value.startsWith('//') ? value : null);

// The link in the email may open in another tab, so where to go afterwards is kept in localStorage.
function rememberNext(next) {
	try {
		if (next) localStorage.setItem(NEXT_KEY, next);
		else localStorage.removeItem(NEXT_KEY);
	} catch {
		// Storage is unavailable (private window): fall back to the profile page after logging in.
	}
}

function takeNext() {
	try {
		const next = safeNext(localStorage.getItem(NEXT_KEY));
		localStorage.removeItem(NEXT_KEY);
		return next;
	} catch {
		return null;
	}
}

// Why coming in through Google, Apple, Facebook or GitHub ended back here (`/log-in?error=…`, set by the API).
const PROVIDER_ERRORS = {
	cancelled: 'That was cancelled. Nothing was changed.',
	no_email: 'That account did not give us a verified email address, so we cannot tell who you are. Verify the address there, or use a link by email.',
	state_mismatch: 'That was started in another browser or took too long. Try again.',
	provider_unavailable: 'That way of getting in is not switched on here. Use another one.',
	provider_error: 'The other site did not confirm who you are. Try again, or use a link by email.',
	server_error: 'Something went wrong on the server. Try again.'
};

/** The providers that are switched on: [{ id, name }]. A link by email works without them, so a failure here is not shown. */
async function loadProviders() {
	try {
		return (await getJson('/api/auth/providers')).providers;
	} catch (error) {
		if (!(error instanceof ApiError)) console.error(error);
		return [];
	}
}

/** A notice for pages that need a logged-in user. */
export function logInPrompt(what) {
	const next = encodeURIComponent(location.pathname + location.search);
	return h('section', { class: 'stack' }, [
		sectionHeading('Log in first', { level: 1 }),
		h('p', { class: 'prose' }, `You need to be logged in to ${what}.`),
		h('p', { class: 'row' }, [linkButton('Log in', `/log-in?next=${next}`, { primary: true }), linkButton('Sign up', `/sign-up?next=${next}`)])
	]);
}

const count = (n, one, many) => `${n.toLocaleString('en')} ${n === 1 ? one : many}`;

/** One plan's AI requests in a sentence. */
function aiSentence(ai) {
	return ai.per === 'world' ? `${count(ai.limit, 'AI request', 'AI requests')} per world. They never come back.` : `${count(ai.limit, 'AI request', 'AI requests')} a month, across all worlds. They come back on the 1st.`;
}

/** What an account includes and what it does not, shown before anyone signs up. Null if the numbers cannot be fetched. */
async function planSummary() {
	let plans;
	try {
		plans = await getJson('/api/plans');
	} catch (error) {
		if (!(error instanceof ApiError)) console.error(error);
		return null;
	}
	const { free, subscriber } = plans;
	const item = (title, text) => h('li', { class: 'stack-sm' }, [h('strong', null, title), text]);
	return h('div', { class: 'card stack', 'data-role': 'plan-summary' }, [
		sectionHeading('What a free account includes', { level: 2 }),
		h('ul', { class: 'plain-list stack' }, [
			item('Strategies written by the AI', aiSentence(free.ai_requests)),
			item('What counts as an AI request', 'Every time you ask the AI to turn a description into rules, one is used. Asking it to rewrite a strategy uses another. Deleting a strategy does not give one back.'),
			item('Strategies you write yourself', 'No limit. Build the rules by hand or write code as often as you like.'),
			item('Simulation runs', `${count(free.monthly_runs, 'run', 'runs')} a month, ${count(free.max_active_runs, 'run', 'runs')} at a time. The month’s runs come back on the 1st.`)
		]),
		h('p', { class: 'meta' }, `Subscribers get ${aiSentence(subscriber.ai_requests)} They also get ${count(subscriber.monthly_runs, 'run', 'runs')} a month.`)
	]);
}

/**
 * The two ways in share one form. Signing up makes an account for an address that has none; logging in never does:
 * for an address without an account the email says so, and points to the sign-up page.
 */
function accountPage({ signUp }) {
	const here = signUp ? '/sign-up' : '/log-in';
	const title = signUp ? 'Sign up' : 'Log in';
	return async ({ query, navigate, replaceUrl }) => {
		const next = safeNext(query.get('next'));
		if (await getUser()) {
			await navigate(next ?? '/profile');
			return { title, node: h('p', { class: 'meta' }, 'Logged in') };
		}
		const withNext = (path) => (next ? `${path}?next=${encodeURIComponent(next)}` : path);
		const failure = query.get('error');
		// Say it once: a refresh should not repeat the message.
		if (failure) replaceUrl(withNext(here));
		const [providers, summary] = await Promise.all([loadProviders(), signUp ? planSummary() : null]);

		const email = h('input', { id: 'account-email', name: 'email', type: 'email', autocomplete: 'email', required: true, 'aria-describedby': 'account-email-hint' });
		const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
		const send = signUp ? 'Sign up' : 'Email me a link';
		const submit = button(send, { primary: true, type: 'submit' });
		const section = h('section', { class: 'stack-lg' });

		const form = h(
			'form',
			{
				class: 'stack-lg form',
				onsubmit: async (e) => {
					e.preventDefault();
					errors.hidden = true;
					submit.disabled = true;
					submit.textContent = 'Sending…';
					try {
						const sent = await postJson('/api/auth/email', { email: email.value, intent: signUp ? 'sign_up' : 'log_in' });
						rememberNext(next);
						const heading = sectionHeading('Check your email', { level: 1 });
						section.replaceChildren(
							heading,
							h('p', { class: 'prose' }, signUp ? ['A link to finish signing up is on its way to ', h('strong', null, email.value.trim()), '.'] : ['We have sent an email to ', h('strong', null, email.value.trim()), '. If the address has an account, it holds your log-in link.']),
							h('p', { class: 'meta' }, `The link works once · Valid for ${sent.expires_in_minutes} minutes`),
							h('p', null, button('Use another address', { onclick: () => navigate(location.pathname + location.search) }))
						);
						heading.focus();
					} catch (error) {
						submit.disabled = false;
						submit.textContent = send;
						if (!(error instanceof ApiError)) console.error(error);
						errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'The link could not be sent.'));
						errors.hidden = false;
						email.focus();
					}
				}
			},
			[
				h('div', { class: 'field' }, [
					h('label', { class: 'field-label', for: 'account-email' }, 'Email address'),
					email,
					h('p', { class: 'meta', id: 'account-email-hint' }, signUp ? 'No password · We email you a link to finish' : 'No password · We email you a one-time link')
				]),
				errors,
				h('div', null, submit)
			]
		);
		section.append(sectionHeading(title, { level: 1 }));
		if (failure) {
			section.append(h('div', { class: 'error-box', role: 'alert' }, h('p', null, PROVIDER_ERRORS[failure] ?? PROVIDER_ERRORS.provider_error)));
		}
		if (summary) section.append(summary);
		if (providers.length > 0) {
			// Plain links: the browser itself goes to the API, which sends it on to the provider and back.
			const start = (id) => `/api/auth/${id}/start${next ? `?next=${encodeURIComponent(next)}` : ''}`;
			section.append(
				h('div', { class: 'stack' }, [
					sectionHeading('With an account you have elsewhere', { level: 2 }),
					h('div', { class: 'row' }, providers.map((p) => linkButton(`Continue with ${p.name}`, start(p.id)))),
					h('p', { class: 'meta' }, 'We receive your name and email address · Nothing is posted')
				]),
				h('div', { class: 'stack' }, [sectionHeading('Or with a link by email', { level: 2 }), form])
			);
		} else {
			section.append(form);
		}
		section.append(
			h('p', { class: 'meta' }, signUp ? ['Already have an account? ', h('a', { href: withNext('/log-in') }, 'Log in')] : ['No account yet? ', h('a', { href: withNext('/sign-up') }, 'Sign up')])
		);
		return { title, node: section };
	};
}

export const signUpPage = accountPage({ signUp: true });
export const logInPage = accountPage({ signUp: false });

/** Where the emailed link lands. Getting in takes a click, so mail scanners that open links do not use the link up. */
export async function confirmLinkPage({ query, navigate, replaceUrl }) {
	const token = query.get('token');
	// Keep the one-time token out of the address bar and the history.
	replaceUrl('/log-in/confirm');
	if (!token) {
		return {
			title: 'Log in',
			node: h('section', { class: 'stack' }, [
				sectionHeading('This link is not complete', { level: 1 }),
				h('p', { class: 'prose' }, 'Open the link from your email again, or ask for a new one.'),
				h('p', null, linkButton('Ask for a new link', '/log-in', { primary: true }))
			])
		};
	}
	const errors = h('div', { class: 'error-box stack-sm', role: 'alert', hidden: true });
	const finish = button('Continue', {
		primary: true,
		onclick: async () => {
			finish.disabled = true;
			finish.textContent = 'One moment…';
			try {
				setUser(await postJson('/api/auth/email/confirm', { token }));
				await navigate(takeNext() ?? '/profile');
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
				finish.hidden = true;
				errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'That did not work.'), h('p', null, linkButton('Ask for a new link', '/log-in', { primary: true })));
				errors.hidden = false;
			}
		}
	});
	return {
		title: 'Log in',
		node: h('section', { class: 'stack-lg' }, [sectionHeading('Almost there', { level: 1 }), h('p', { class: 'prose' }, 'Press the button to open your account on this device.'), h('div', null, finish), errors])
	};
}
