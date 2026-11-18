import { getJson } from '../api.js';
import { linkButton } from '../components/button.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';

export async function worldsPage() {
	const { worlds } = await getJson('/api/worlds');
	return {
		title: 'Worlds',
		node: h('section', { class: 'stack' }, [
			sectionHeading('Worlds', { level: 1 }),
			h('p', { class: 'meta' }, 'Pick a world · Enter strategies · See who wins'),
			h(
				'ul',
				{ class: 'cards' },
				worlds.map((w) =>
					h('li', { class: 'card stack' }, [
						h('h2', { class: 'section-heading' }, h('a', { href: `/worlds/${w.id}` }, w.metadata.name)),
						h('p', null, w.metadata.description),
						h('p', { class: 'meta' }, `${w.decisions.allowed.length} decisions · ${w.time.iterations} rounds per match · Seed ${w.determinism.rng_seed}`),
						h('p', { class: 'row' }, [linkButton('View world', `/worlds/${w.id}`), linkButton('Run simulation', `/runs/new?world=${w.id}`, { primary: true })])
					])
				)
			)
		])
	};
}
