import { getJson } from '../api.js';
import { avatarName } from '../components/avatar.js';
import { linkButton } from '../components/button.js';
import { pager } from '../components/pager.js';
import { sectionHeading } from '../components/section-heading.js';
import { fmt, h } from '../dom.js';

export async function runsPage() {
	const { runs } = await getJson('/api/runs');
	const body = runs.length
		? // As many runs as fit the window; the rest are a page away.
			pager({
				rowHeight: 50,
				reserve: 450,
				rows: runs.map((r) =>
					h('tr', null, [
						h('th', { scope: 'row' }, h('a', { href: `/runs/${r.id}` }, `Run ${r.id}`)),
						h('td', { class: 'meta', 'data-label': 'Status' }, r.status),
						h('td', { 'data-label': 'World' }, r.world_name),
						h('td', { class: 'num', 'data-label': 'Seed' }, String(r.seed)),
						h('td', { class: 'num', 'data-label': 'Rounds' }, String(r.iterations)),
						h('td', { class: 'num', 'data-label': 'Actors' }, String(r.actor_count)),
						h('td', { 'data-label': 'Winner' }, r.winner ? avatarName(r.winner.id, `${r.winner.name} (${fmt(r.winner.average)})`) : '')
					])
				),
				wrap: (rows) =>
					h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Runs' }, [
						h('table', { class: 'grid-table stacks runs-list' }, [
							h('thead', null, h('tr', null, ['Run', 'Status', 'World', 'Seed', 'Rounds', 'Actors', 'Winner'].map((t) => h('th', { scope: 'col' }, t)))),
							h('tbody', null, rows)
						])
					])
			})
		: h('p', null, 'No runs yet. Start one to see which strategy wins.');
	return {
		title: 'Runs',
		node: h('section', { class: 'stack' }, [
			h('div', { class: 'row spread' }, [sectionHeading('Runs', { level: 1 }), linkButton('New run', '/runs/new', { primary: true })]),
			h('p', { class: 'prose runs-intro' }, 'A run plays strategies against each other. Open one to see the ranking and every match.'),
			h('p', { class: 'meta' }, 'Newest first'),
			body
		])
	};
}
