import { ApiError, deleteJson, getJson } from '../api.js';
import { button, linkButton } from '../components/button.js';
import { codeBlock, machineTable } from '../components/machine.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { getUser, signInUrl } from '../session.js';

function card(s, { refresh }) {
	const tags = [s.mode === 'code' ? 'Code' : 'Rules'];
	if (s.mine) tags.push(s.visibility === 'public' ? 'Public' : 'Private');
	else if (!s.builtin && s.owner_name) tags.push(`By ${s.owner_name}`);
	const errors = h('p', { role: 'alert', hidden: true });
	const actions = s.mine
		? h('p', { class: 'row' }, [
				linkButton('Edit', `/strategies/${s.id}/edit`, { 'aria-label': `Edit ${s.name}` }),
				button('Delete', {
					'aria-label': `Delete ${s.name}`,
					onclick: async () => {
						if (!window.confirm(`Delete "${s.name}"? Runs it already played in stay as they are.`)) return;
						try {
							await deleteJson(`/api/strategies/${s.id}`);
							refresh();
						} catch (error) {
							if (!(error instanceof ApiError)) console.error(error);
							errors.textContent = error instanceof ApiError ? error.message : 'The strategy could not be deleted.';
							errors.hidden = false;
						}
					}
				})
			])
		: null;
	return h('li', { class: 'card stack' }, [
		h('h3', { class: 'section-heading' }, s.name),
		h('p', { class: 'meta' }, tags.join(' · ')),
		s.description ? h('p', null, s.description) : null,
		s.mode === 'code' ? codeBlock(s) : machineTable(s),
		actions,
		errors
	]);
}

export async function strategiesPage(ctx) {
	const [user, { strategies }] = await Promise.all([getUser(), getJson('/api/strategies')]);
	const mine = strategies.filter((s) => s.mine);
	const builtin = strategies.filter((s) => s.builtin);
	const others = strategies.filter((s) => !s.mine && !s.builtin);
	const group = (title, list) => (list.length ? h('div', { class: 'stack' }, [sectionHeading(title), h('ul', { class: 'cards' }, list.map((s) => card(s, ctx)))]) : null);

	return {
		title: 'Strategies',
		node: h('section', { class: 'stack-lg' }, [
			h('div', { class: 'stack' }, [
				sectionHeading('Strategies', { level: 1 }),
				h('p', { class: 'prose' }, 'A strategy decides what to play each round from what has happened so far. Most are rules: a few states, one decision per state, and conditions for moving between them. A strategy can also be written as code.'),
				h('p', null, user ? linkButton('New strategy', '/strategies/new', { primary: true }) : linkButton('Sign in to make your own', signInUrl('/strategies/new')))
			]),
			user && !mine.length ? h('p', { class: 'meta' }, 'You have no strategies yet') : null,
			group('Your strategies', mine),
			group('Built in', builtin),
			group('From other players', others)
		])
	};
}
