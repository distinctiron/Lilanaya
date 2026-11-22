import { ApiError, patchJson } from '../api.js';
import { button } from '../components/button.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { refreshUser, setUser, signOut } from '../session.js';
import { logInPrompt } from './sign-in.js';

const ENTITLEMENTS = [
	['analytics_access', 'Analytics and replays'],
	['quick_test_access', 'Quick test'],
	['private_worlds', 'Private worlds']
];

/** AI requests used against the plan: per world for good, or per month across worlds. */
function aiRequests(ai) {
	if (!ai) return null;
	const used = Object.entries(ai.used_by_world);
	const lines =
		ai.per === 'world'
			? [
					h('p', null, [h('strong', null, `${ai.limit} per world`), ' · They never come back']),
					used.length ? h('ul', { class: 'plain-list tight' }, used.map(([world, n]) => h('li', null, `${world} · ${Math.min(n, ai.limit)} of ${ai.limit} used`))) : h('p', { class: 'meta' }, 'None used yet')
				]
			: [h('p', null, [h('strong', null, `${Math.max(0, ai.limit - ai.used_this_month)} left`), ` · ${ai.used_this_month} of ${ai.limit} used this month`]), h('p', { class: 'meta' }, 'Across all worlds · Resets on the 1st')];
	return h('div', { class: 'stack' }, [
		sectionHeading('AI requests'),
		...lines,
		h('p', { class: 'meta' }, 'One is used every time the AI turns a description into rules, rewrites included · Rules built by hand and code are not limited')
	]);
}

export async function profilePage({ navigate, refresh }) {
	// Ask again rather than trust the cached copy: runs started since change the count.
	const user = await refreshUser();
	if (!user) return { title: 'Profile', node: logInPrompt('see your profile') };

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

	return {
		title: 'Profile',
		node: h('section', { class: 'stack-lg' }, [
			sectionHeading('Profile', { level: 1 }),
			// Side by side where there is room, so the whole profile is one screen: who you are, and what your plan gives you.
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
					form
				]),
				h('div', { class: 'stack-lg' }, [
					h('div', { class: 'stack' }, [
						sectionHeading('Runs this month'),
						h('p', null, [h('strong', null, `${usage.remaining} left`), ` · ${usage.used} of ${usage.monthly_runs} used`]),
						h('div', { class: 'meter', role: 'img', 'aria-label': `${usage.used} of ${usage.monthly_runs} runs used` }, h('div', { style: { width: `${share * 100}%` } })),
						h('p', { class: 'meta' }, `Month ${usage.period} · Resets on the 1st · ${usage.max_active_runs} ${usage.max_active_runs === 1 ? 'run' : 'runs'} at a time`)
					]),
					aiRequests(usage.ai_requests),
					h('div', { class: 'stack' }, [
						sectionHeading('Your plan includes'),
						h(
							'ul',
							{ class: 'plain-list tight' },
							ENTITLEMENTS.map(([key, label]) => h('li', null, [label, ' · ', h('span', { class: 'muted' }, user.entitlements[key] ? 'Included' : 'Subscribers only')]))
						)
					])
				])
			]),
			h(
				'div',
				null,
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
			)
		])
	};
}
