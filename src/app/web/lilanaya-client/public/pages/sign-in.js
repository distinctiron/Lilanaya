import { ApiError, postJson } from '../api.js';
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
		// Storage is unavailable (private window): fall back to the profile page after sign-in.
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

/** A notice for pages that need a signed-in user. */
export function signInPrompt(what) {
	return h('section', { class: 'stack' }, [
		sectionHeading('Sign in first', { level: 1 }),
		h('p', { class: 'prose' }, `You need to be signed in to ${what}.`),
		h('p', null, linkButton('Sign in', `/sign-in?next=${encodeURIComponent(location.pathname + location.search)}`, { primary: true }))
	]);
}

export async function signInPage({ query, navigate }) {
	const next = safeNext(query.get('next'));
	if (await getUser()) {
		await navigate(next ?? '/profile');
		return { title: 'Sign in', node: h('p', { class: 'meta' }, 'Signed in') };
	}

	const email = h('input', { id: 'sign-in-email', name: 'email', type: 'email', autocomplete: 'email', required: true, 'aria-describedby': 'sign-in-hint' });
	const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
	const submit = button('Email me a link', { primary: true, type: 'submit' });
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
					const sent = await postJson('/api/auth/email', { email: email.value });
					rememberNext(next);
					const heading = sectionHeading('Check your email', { level: 1 });
					section.replaceChildren(
						heading,
						h('p', { class: 'prose' }, ['A sign-in link is on its way to ', h('strong', null, email.value.trim()), '.']),
						h('p', { class: 'meta' }, `The link works once · Valid for ${sent.expires_in_minutes} minutes`),
						h('p', null, button('Use another address', { onclick: () => navigate(location.pathname + location.search) }))
					);
					heading.focus();
				} catch (error) {
					submit.disabled = false;
					submit.textContent = 'Email me a link';
					if (!(error instanceof ApiError)) console.error(error);
					errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'The link could not be sent.'));
					errors.hidden = false;
					email.focus();
				}
			}
		},
		[
			h('div', { class: 'field' }, [
				h('label', { class: 'field-label', for: 'sign-in-email' }, 'Email address'),
				email,
				h('p', { class: 'meta', id: 'sign-in-hint' }, 'No password · We email you a one-time link')
			]),
			errors,
			h('div', null, submit)
		]
	);
	section.append(sectionHeading('Sign in', { level: 1 }), form);
	return { title: 'Sign in', node: section };
}

/** Where the emailed link lands. Signing in takes a click, so mail scanners that open links do not use the link up. */
export async function confirmSignInPage({ query, navigate, replaceUrl }) {
	const token = query.get('token');
	// Keep the one-time token out of the address bar and the history.
	replaceUrl('/sign-in/confirm');
	if (!token) {
		return {
			title: 'Sign in',
			node: h('section', { class: 'stack' }, [
				sectionHeading('This link is not complete', { level: 1 }),
				h('p', { class: 'prose' }, 'Open the link from your email again, or ask for a new one.'),
				h('p', null, linkButton('Ask for a new link', '/sign-in', { primary: true }))
			])
		};
	}
	const errors = h('div', { class: 'error-box stack-sm', role: 'alert', hidden: true });
	const finish = button('Finish signing in', {
		primary: true,
		onclick: async () => {
			finish.disabled = true;
			finish.textContent = 'Signing in…';
			try {
				setUser(await postJson('/api/auth/email/confirm', { token }));
				await navigate(takeNext() ?? '/profile');
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
				finish.hidden = true;
				errors.replaceChildren(
					h('p', null, error instanceof ApiError ? error.message : 'Signing in did not work.'),
					h('p', null, linkButton('Ask for a new link', '/sign-in', { primary: true }))
				);
				errors.hidden = false;
			}
		}
	});
	return {
		title: 'Sign in',
		node: h('section', { class: 'stack-lg' }, [sectionHeading('Almost there', { level: 1 }), h('p', { class: 'prose' }, 'Press the button to finish signing in on this device.'), h('div', null, finish), errors])
	};
}
