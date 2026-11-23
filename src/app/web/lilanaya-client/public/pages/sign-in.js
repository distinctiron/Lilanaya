import { ApiError, getJson, postJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { aiSentence } from '../components/plan-words.js';
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
	no_email: 'That account did not give us a verified email address, so we cannot tell who you are. Verify the address there, or use your email address here.',
	state_mismatch: 'That was started in another browser or took too long. Try again.',
	provider_unavailable: 'That way of getting in is not switched on here. Use another one.',
	provider_error: 'The other site did not confirm who you are. Try again, or use your email address here.',
	server_error: 'Something went wrong on the server. Try again.'
};

/** The providers that are switched on: [{ id, name }]. The email address works without them, so a failure here is not shown. */
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
			item('What counts as one', 'Every time you ask the AI to turn a description into rules, one is used. Asking it to write the same strategy again uses another. Deleting a strategy does not give one back.'),
			item('Strategies you write yourself', 'No limit. Build the rules by hand or write code as often as you like.'),
			item('Simulation runs', `${count(free.monthly_runs, 'run', 'runs')} a month, ${count(free.max_active_runs, 'run', 'runs')} at a time. The month’s runs come back on the 1st.`)
		]),
		h('p', { class: 'meta' }, [`For subscribers: ${aiSentence(subscriber.ai_requests)} They also get ${count(subscriber.monthly_runs, 'run', 'runs')} a month. `, h('a', { href: '/plans' }, 'Plans and prices')])
	]);
}

/** The least and the most a password may hold; the API checks the same. */
const PASSWORD_MIN = 8;
const PASSWORD_MAX = 200;

/** A password field with its label, and a box to see what is being typed. `fresh` is for choosing one, not for giving the one you have. */
function passwordField({ id, fresh, hint }) {
	const input = h('input', { id, name: 'password', type: 'password', autocomplete: fresh ? 'new-password' : 'current-password', required: true, maxlength: String(PASSWORD_MAX), ...(fresh ? { minlength: String(PASSWORD_MIN) } : {}), ...(hint ? { 'aria-describedby': `${id}-hint` } : {}) });
	const show = h('input', { type: 'checkbox', id: `${id}-show`, onchange: () => (input.type = show.checked ? 'text' : 'password') });
	const field = h('div', { class: 'field' }, [
		h('label', { class: 'field-label', for: id }, 'Password'),
		input,
		hint ? h('p', { class: 'meta', id: `${id}-hint` }, hint) : null,
		h('label', { class: 'check', for: `${id}-show` }, [show, h('span', null, 'Show password')])
	]);
	return { input, field };
}

/**
 * The ways in share one page. Signing up takes an address: a link is emailed, and whoever opens it chooses a password.
 * Logging in takes the address and that password. Someone who has forgotten it, or never chose one, asks for a link again.
 * Only signing up makes an account: for an address without one the email says so, and points to the sign-up page.
 */
function accountPage(mode) {
	const signUp = mode === 'sign-up';
	const forgot = mode === 'forgot';
	const here = signUp ? '/sign-up' : forgot ? '/log-in/forgot' : '/log-in';
	const title = signUp ? 'Sign up' : forgot ? 'Forgot password' : 'Log in';
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
		const [providers, summary] = await Promise.all([forgot ? [] : loadProviders(), signUp ? planSummary() : null]);

		const email = h('input', { id: 'account-email', name: 'email', type: 'email', autocomplete: signUp || forgot ? 'email' : 'username', required: true, ...(signUp || forgot ? { 'aria-describedby': 'account-email-hint' } : {}) });
		const password = signUp || forgot ? null : passwordField({ id: 'account-password', fresh: false });
		const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
		const send = signUp ? 'Sign up' : forgot ? 'Email me a link' : 'Log in';
		const submit = button(send, { primary: true, type: 'submit' });
		const section = h('section', { class: 'stack-lg' });
		const fail = (error, fallback, focus) => {
			submit.disabled = false;
			submit.textContent = send;
			if (!(error instanceof ApiError)) console.error(error);
			errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : fallback));
			errors.hidden = false;
			focus.focus();
		};

		/** Logging in: the address and the password, and straight on to where the visitor was going. */
		const logIn = async () => {
			submit.textContent = 'One moment…';
			try {
				setUser(await postJson('/api/auth/log-in', { email: email.value, password: password.input.value }));
			} catch (error) {
				fail(error, 'Logging in did not work.', password.input);
				return;
			}
			await navigate(next ?? '/profile');
		};

		/** Signing up, or a forgotten password: the link goes out by email. */
		const askForLink = async () => {
			submit.textContent = 'Sending…';
			try {
				const sent = await postJson('/api/auth/email', { email: email.value, intent: signUp ? 'sign_up' : 'log_in' });
				rememberNext(next);
				const heading = sectionHeading('Check your email', { level: 1 });
				section.replaceChildren(
					heading,
					h(
						'p',
						{ class: 'prose' },
						signUp
							? ['A link to choose your password and finish signing up is on its way to ', h('strong', null, email.value.trim()), '.']
							: ['We have sent an email to ', h('strong', null, email.value.trim()), '. If the address has an account, it holds a link to choose a new password.']
					),
					h('p', { class: 'meta' }, `The link works once · Valid for ${sent.expires_in_minutes} minutes`),
					h('p', null, button('Use another address', { onclick: () => navigate(location.pathname + location.search) }))
				);
				heading.focus();
			} catch (error) {
				fail(error, 'The link could not be sent.', email);
			}
		};

		const form = h(
			'form',
			{
				class: 'stack-lg form',
				onsubmit: async (e) => {
					e.preventDefault();
					errors.hidden = true;
					submit.disabled = true;
					await (password ? logIn() : askForLink());
				}
			},
			[
				h('div', { class: 'field' }, [
					h('label', { class: 'field-label', for: 'account-email' }, 'Email address'),
					email,
					signUp || forgot ? h('p', { class: 'meta', id: 'account-email-hint' }, signUp ? 'We email you a link · You choose your password there' : 'We email you a one-time link · You choose a new password there') : null
				]),
				password?.field,
				errors,
				h('div', null, submit),
				password ? h('p', { class: 'meta' }, [h('a', { href: withNext('/log-in/forgot') }, 'Forgot your password, or never chose one?')]) : null
			]
		);
		section.append(sectionHeading(title, { level: 1 }));
		if (failure) {
			section.append(h('div', { class: 'error-box', role: 'alert' }, h('p', null, PROVIDER_ERRORS[failure] ?? PROVIDER_ERRORS.provider_error)));
		}
		if (forgot) section.append(h('p', { class: 'prose' }, 'Enter the address of your account. The link we send lets you choose a new password and logs you in. It also works if you signed up before passwords, or through another site.'));
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
				h('div', { class: 'stack' }, [sectionHeading(signUp ? 'Or with your email address' : 'Or with your email address and password', { level: 2 }), form])
			);
		} else {
			section.append(form);
		}
		if (signUp) section.append(h('p', { class: 'meta', 'data-role': 'legal-links' }, ['By signing up you accept the ', h('a', { href: '/terms' }, 'terms'), ' · How we handle your data: ', h('a', { href: '/privacy' }, 'privacy')]));
		section.append(
			h(
				'p',
				{ class: 'meta' },
				signUp ? ['Already have an account? ', h('a', { href: withNext('/log-in') }, 'Log in')] : forgot ? ['Remembered it? ', h('a', { href: withNext('/log-in') }, 'Log in')] : ['No account yet? ', h('a', { href: withNext('/sign-up') }, 'Sign up')]
			)
		);
		return { title, node: section };
	};
}

export const signUpPage = accountPage('sign-up');
export const logInPage = accountPage('log-in');
export const forgotPasswordPage = accountPage('forgot');

/**
 * Where the emailed link lands: the visitor chooses the password they will log in with from now on, and is logged in.
 * Getting in takes a form, so mail scanners that open links do not use the link up.
 */
export async function confirmLinkPage({ query, navigate, replaceUrl }) {
	const token = query.get('token');
	// Keep the one-time token out of the address bar and the history.
	replaceUrl('/log-in/confirm');
	if (!token) {
		return {
			title: 'Choose a password',
			node: h('section', { class: 'stack' }, [
				sectionHeading('This link is not complete', { level: 1 }),
				h('p', { class: 'prose' }, 'Open the link from your email again, or ask for a new one.'),
				h('p', null, linkButton('Ask for a new link', '/log-in/forgot', { primary: true }))
			])
		};
	}
	const password = passwordField({ id: 'new-password', fresh: true, hint: `At least ${PASSWORD_MIN} characters · Anything you like, a few words work well` });
	const errors = h('div', { class: 'error-box stack-sm', role: 'alert', hidden: true });
	const finish = button('Continue', { primary: true, type: 'submit' });
	const form = h(
		'form',
		{
			class: 'stack-lg form',
			onsubmit: async (e) => {
				e.preventDefault();
				errors.hidden = true;
				finish.disabled = true;
				finish.textContent = 'One moment…';
				try {
					setUser(await postJson('/api/auth/email/confirm', { token, password: password.input.value }));
				} catch (error) {
					if (!(error instanceof ApiError)) console.error(error);
					if (error instanceof ApiError && error.details[0]?.path === 'password') {
						// The link is still good: only the password has to be chosen again.
						finish.disabled = false;
						finish.textContent = 'Continue';
						errors.replaceChildren(h('p', null, `The password ${error.details[0].message}.`));
						errors.hidden = false;
						password.input.focus();
						return;
					}
					form.hidden = true;
					errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'That did not work.'), h('p', null, linkButton('Ask for a new link', '/log-in/forgot', { primary: true })));
					errors.hidden = false;
					return;
				}
				await navigate(takeNext() ?? '/profile');
			}
		},
		[password.field, h('div', null, finish)]
	);
	return {
		title: 'Choose a password',
		node: h('section', { class: 'stack-lg' }, [
			sectionHeading('Choose a password', { level: 1 }),
			h('p', { class: 'prose' }, 'From now on you log in with your email address and this password. Choosing it also logs you in on this device, and logs you out everywhere else.'),
			form,
			errors
		])
	};
}
