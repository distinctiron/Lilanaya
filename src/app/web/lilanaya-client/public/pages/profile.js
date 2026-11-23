import { ApiError, deleteJson, getJson, patchJson, postJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { day, money, packWords, strategies } from '../components/plan-words.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { refreshUser, setUser, signOut } from '../session.js';
import { legalLinks } from './plans.js';
import { logInPrompt } from './sign-in.js';

/** Strategies the AI has written against the plan: per world for good, or per paid month across worlds. */
function aiStrategies(usage) {
	const ai = usage.ai_requests;
	if (!ai) return null;
	const used = Object.entries(ai.used_by_world);
	const lines =
		ai.per === 'world'
			? [
					h('p', null, [h('strong', null, `${ai.limit} per world`), ' · They never come back']),
					used.length ? h('ul', { class: 'plain-list tight' }, used.map(([world, n]) => h('li', null, `${world} · ${Math.min(n, ai.limit)} of ${ai.limit} used`))) : h('p', { class: 'meta' }, 'None used yet')
				]
			: [h('p', null, [h('strong', null, `${Math.max(0, ai.limit - ai.used_this_month)} left`), ` · ${ai.used_this_month} of ${ai.limit} used`]), h('p', { class: 'meta' }, `Across all worlds · They come back on ${day(usage.period_end)}`)];
	return h('div', { class: 'stack', 'data-role': 'ai-strategies' }, [
		sectionHeading('Strategies the AI writes'),
		...lines,
		h('p', { class: 'meta' }, 'One is used every time the AI turns a description into rules, writing one again included · Rules built by hand and code are not limited')
	]);
}

/** What the user pays for and has bought, with the way to change it. Null when there is nothing to say. */
function payment(user, billing, errors) {
	if (!billing) return null;
	const sub = billing.subscription;
	const manage = button('Manage subscription and invoices', {
		onclick: async () => {
			errors.hidden = true;
			manage.disabled = true;
			try {
				location.assign((await postJson('/api/billing/portal', {})).url);
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
				errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'The page could not be opened.'));
				errors.hidden = false;
				manage.disabled = false;
			}
		}
	});
	let status;
	if (sub?.paid) {
		status = sub.cancel_at_period_end
			? `Subscriber until ${day(sub.period_end)} · Cancelled, so it is not renewed`
			: sub.status === 'past_due'
				? `Your last payment did not go through · Update your card to keep the subscription`
				: `Subscriber · Paid until ${day(sub.period_end)} · Renews then`;
	} else if (user.plan === 'subscriber') {
		status = 'Subscriber';
	} else if (sub) {
		status = sub.status === 'past_due' || sub.status === 'unpaid' ? 'Free · Your subscription is on hold because the last payment did not go through' : 'Free · Your subscription has ended';
	} else {
		status = 'Free';
	}
	const packs = billing.packs;
	const bought = billing.purchases;
	return h('div', { class: 'stack', 'data-role': 'payment' }, [
		sectionHeading('Plan and payment'),
		h('p', { 'data-role': 'plan-status' }, status),
		packs.strategies_left > 0 || packs.runs_left > 0 ? h('p', { 'data-role': 'pack-balance' }, [h('strong', null, 'Left in your packs'), ` · ${packWords({ strategies: packs.strategies_left, runs: packs.runs_left })}`]) : null,
		bought.length
			? h(
					'ul',
					{ class: 'plain-list tight', 'data-role': 'purchases' },
					bought.map((p) => h('li', null, `${day(p.created_at)} · ${packWords(p)} · ${money(p.amount, p.currency)}${p.refunded ? ' · Refunded' : ''}`))
				)
			: null,
		h('div', { class: 'row' }, [billing.enabled ? linkButton(sub?.paid || user.plan === 'subscriber' ? 'Plans and packs' : 'Subscribe or buy a pack', '/plans') : null, billing.can_manage ? manage : null])
	]);
}

/** Whether a password has been chosen, and the way to choose another: a link to the account's own address. */
function password(user) {
	const status = h('p', { class: 'meta', role: 'status' });
	const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
	const ask = button(user.has_password ? 'Email me a link to change it' : 'Email me a link to choose one', {
		onclick: async () => {
			errors.hidden = true;
			status.textContent = '';
			ask.disabled = true;
			try {
				const sent = await postJson('/api/auth/email', { email: user.email, intent: 'log_in' });
				status.textContent = `The link is on its way to ${user.email} · Valid for ${sent.expires_in_minutes} minutes`;
			} catch (error) {
				if (!(error instanceof ApiError)) console.error(error);
				errors.replaceChildren(h('p', null, error instanceof ApiError ? error.message : 'The link could not be sent.'));
				errors.hidden = false;
				ask.disabled = false;
			}
		}
	});
	return h('div', { class: 'stack', 'data-role': 'password' }, [
		sectionHeading('Password'),
		h('p', null, user.has_password ? 'You log in with your email address and your password.' : 'You have not chosen a password yet, so you cannot log in with your email address.'),
		h('p', { class: 'meta' }, 'Choosing a new password logs you out on every other device'),
		errors,
		h('div', { class: 'row' }, [ask, status])
	]);
}

/** Taking the data away, and deleting the account. */
function leaving(user, navigate) {
	const email = h('input', { id: 'delete-email', name: 'email', type: 'email', autocomplete: 'off', required: true, 'aria-describedby': 'delete-hint' });
	const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
	const submit = button('Delete my account for good', { type: 'submit' });
	const form = h(
		'form',
		{
			class: 'stack form',
			hidden: true,
			onsubmit: async (e) => {
				e.preventDefault();
				errors.hidden = true;
				submit.disabled = true;
				try {
					await deleteJson('/api/me', { email: email.value });
					setUser(null);
					await navigate('/');
				} catch (error) {
					if (!(error instanceof ApiError)) console.error(error);
					errors.replaceChildren(h('p', null, error instanceof ApiError ? (error.details[0] ? 'That is not this account’s email address.' : error.message) : 'The account could not be deleted.'));
					errors.hidden = false;
					submit.disabled = false;
				}
			}
		},
		[
			h('p', { class: 'prose', id: 'delete-hint' }, 'This deletes your account, your strategies and your private runs, ends your subscription, and gives up whatever is left of your packs. It cannot be undone. Type your email address to confirm.'),
			h('div', { class: 'field' }, [h('label', { class: 'field-label', for: 'delete-email' }, 'Your email address'), email]),
			errors,
			h('div', null, submit)
		]
	);
	const open = button('Delete account…', {
		'aria-expanded': 'false',
		onclick: () => {
			form.hidden = !form.hidden;
			open.setAttribute('aria-expanded', String(!form.hidden));
			if (!form.hidden) email.focus();
		}
	});
	return h('div', { class: 'stack', 'data-role': 'leaving' }, [
		sectionHeading('Your data'),
		h('div', { class: 'row' }, [h('a', { class: 'btn', href: '/api/me/export', download: 'lilanaya-my-data.json' }, 'Download my data'), open]),
		form
	]);
}

export async function profilePage({ navigate, refresh }) {
	// Ask again rather than trust the cached copy: runs started since change the count.
	const user = await refreshUser();
	if (!user) return { title: 'Profile', node: logInPrompt('see your profile') };
	let billing = null;
	try {
		billing = await getJson('/api/billing');
	} catch (error) {
		if (!(error instanceof ApiError)) console.error(error);
	}

	const { usage } = user;
	const share = usage.monthly_runs > 0 ? Math.min(1, usage.used / usage.monthly_runs) : 1;

	const name = h('input', { id: 'profile-name', name: 'display_name', type: 'text', value: user.display_name, maxlength: '60', required: true, autocomplete: 'nickname' });
	const status = h('p', { class: 'meta', role: 'status' });
	const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
	const save = button('Save name', { type: 'submit' });
	const form = h(
		'form',
		{
			class: 'stack form',
			onsubmit: async (e) => {
				e.preventDefault();
				errors.hidden = true;
				status.textContent = '';
				save.disabled = true;
				try {
					setUser(await patchJson('/api/me', { display_name: name.value }));
					status.textContent = 'Saved';
				} catch (error) {
					if (!(error instanceof ApiError)) console.error(error);
					errors.replaceChildren(h('p', null, error instanceof ApiError ? (error.details[0] ? `Display name ${error.details[0].message}.` : error.message) : 'The name could not be saved.'));
					errors.hidden = false;
				}
				save.disabled = false;
			}
		},
		[h('div', { class: 'field' }, [h('label', { class: 'field-label', for: 'profile-name' }, 'Display name'), name, h('p', { class: 'meta' }, 'Shown next to strategies you publish')]), errors, h('div', { class: 'row' }, [save, status])]
	);
	const paymentErrors = h('div', { class: 'error-box', role: 'alert', hidden: true });

	return {
		title: 'Profile',
		node: h('section', { class: 'stack-lg' }, [
			sectionHeading('Profile', { level: 1 }),
			// Side by side where there is room: who you are and what you pay for, and what your plan gives you.
			h('div', { class: 'columns' }, [
				h('div', { class: 'stack-lg' }, [
					h('dl', { class: 'rules' }, [
						h('dt', { class: 'meta' }, 'Email'),
						h('dd', null, user.email),
						h('dt', { class: 'meta' }, 'Roles'),
						h('dd', null, user.roles.join(', ')),
						h('dt', { class: 'meta' }, 'Plan'),
						h('dd', null, user.plan === 'subscriber' ? 'Subscriber' : 'Free')
					]),
					form,
					password(user),
					payment(user, billing, paymentErrors),
					paymentErrors
				]),
				h('div', { class: 'stack-lg' }, [
					h('div', { class: 'stack' }, [
						sectionHeading('Runs'),
						h('p', null, [h('strong', null, `${usage.remaining} left`), ` · ${usage.used} of ${usage.monthly_runs} used`]),
						h('div', { class: 'meter', role: 'img', 'aria-label': `${usage.used} of ${usage.monthly_runs} runs used` }, h('div', { style: { width: `${share * 100}%` } })),
						h('p', { class: 'meta' }, `They come back on ${day(usage.period_end)} · ${usage.max_active_runs} ${usage.max_active_runs === 1 ? 'run' : 'runs'} at a time`)
					]),
					aiStrategies(usage),
					usage.packs && usage.packs.strategies_left + usage.packs.runs_left > 0 && !billing
						? h('p', null, `Left in your packs: ${strategies(usage.packs.strategies_left)}, ${usage.packs.runs_left} runs`)
						: null
				])
			]),
			leaving(user, navigate),
			h('div', { class: 'row' }, [
				button('Log out', {
					onclick: async () => {
						try {
							await signOut();
							await navigate('/worlds');
						} catch (error) {
							console.error(error);
							refresh();
						}
					}
				})
			]),
			legalLinks()
		])
	};
}
