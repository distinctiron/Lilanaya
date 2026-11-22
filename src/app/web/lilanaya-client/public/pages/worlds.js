import { getJson } from '../api.js';
import { linkButton } from '../components/button.js';
import { fold } from '../components/fold.js';
import { sectionHeading } from '../components/section-heading.js';
import { h } from '../dom.js';
import { payoffTable } from './world.js';

export async function worldsPage() {
	const { worlds } = await getJson('/api/worlds');
	return {
		title: 'Worlds',
		node: h('section', { class: 'stack' }, [
			sectionHeading('Worlds', { level: 1 }),
			h('p', { class: 'meta' }, 'Pick a world · Enter strategies · See who wins'),
			h(
				'ul',
				{ class: 'cards folds' },
				worlds.map((w) =>
					// Folded: name and key numbers. Folded out: the short description, the longer guide and the points.
					fold({
						open: worlds.length === 1, // nothing to compare with, so show it
						summary: [
							h('h2', { class: 'section-heading' }, w.metadata.name),
							h('span', { class: 'meta fold-tags' }, `${w.decisions.allowed.length} decisions · ${w.time.iterations} rounds per match`)
						],
						body: [
							h('p', { class: 'prose' }, w.metadata.description),
							(w.metadata.guide ?? '').split(/\n\s*\n/).filter(Boolean).map((text) => h('p', { class: 'prose' }, text)),
							h('div', { class: 'stack-sm' }, [h('h3', { class: 'section-heading' }, 'The points'), payoffTable(w)]),
							h('p', { class: 'meta' }, `Seed ${w.determinism.rng_seed}`),
							h('p', { class: 'row' }, [linkButton('Run simulation', `/runs/new?world=${w.id}`, { primary: true }), linkButton('View world', `/worlds/${w.id}`)])
						]
					})
				)
			)
		])
	};
}
