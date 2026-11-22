import { getJson } from '../api.js';
import { avatarName } from '../components/avatar.js';
import { linkButton } from '../components/button.js';
import { sectionHeading } from '../components/section-heading.js';
import { fmt, h } from '../dom.js';

export async function runsPage() {
	const { runs } = await getJson('/api/runs');
	const body = runs.length
		? h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Runs' }, [
				h('table', { class: 'grid-table' }, [
					h('thead', null, h('tr', null, ['Run', 'Status', 'World', 'Seed', 'Rounds', 'Actors', 'Winner'].map((t) => h('th', { scope: 'col' }, t)))),
					h(
						'tbody',
						null,
						runs.map((r) =>
							h('tr', null, [
								h('th', { scope: 'row' }, h('a', { href: `/runs/${r.id}` }, `Run ${r.id}`)),
								h('td', { class: 'meta' }, r.status),
								h('td', null, r.world_name),
								h('td', { class: 'num' }, String(r.seed)),
								h('td', { class: 'num' }, String(r.iterations)),
								h('td', { class: 'num' }, String(r.actor_count)),
								h('td', null, r.winner ? avatarName(r.winner.id, `${r.winner.name} (${fmt(r.winner.average)})`) : '')
							])
						)
					)
				])
			])
		: h('p', null, 'No runs yet.');
	return {
		title: 'Runs',
		node: h('section', { class: 'stack' }, [
			sectionHeading('Runs', { level: 1 }),
			h('p', { class: 'meta' }, 'Newest first'),
			body,
			h('p', null, linkButton('New run', '/runs/new', { primary: true }))
		])
	};
}
