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
						group: 'world',
						summary: [
							h('h2', { class: 'section-heading' }, w.metadata.name),
							h('span', { class: 'meta fold-tags' }, `${w.decisions.allowed.length} decisions · ${w.time.iterations} rounds per match`)
						],
						body: [
							// Side by side where there is room: what the world is, and its points with the way in.
							h('div', { class: 'columns' }, [
								h('div', { class: 'stack' }, [
									h('p', { class: 'prose' }, w.metadata.description),
									// The longer read is a press away, so the world and its points share one screen.
									w.metadata.guide ? h('details', null, [h('summary', null, 'The story behind it'), h('div', { class: 'stack' }, w.metadata.guide.split(/\n\s*\n/).filter(Boolean).map((text) => h('p', { class: 'prose' }, text)))]) : null
								]),
								h('div', { class: 'stack' }, [
									h('div', { class: 'stack-sm' }, [h('h3', { class: 'section-heading' }, 'The points'), payoffTable(w)]),
									h('p', { class: 'meta' }, `Seed ${w.determinism.rng_seed}`),
									h('p', { class: 'row' }, [linkButton('Run simulation', `/runs/new?world=${w.id}`, { primary: true }), linkButton('View world', `/worlds/${w.id}`)])
								])
							])
						]
					})
				)
			)
		])
	};
}
