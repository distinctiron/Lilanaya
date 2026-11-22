import { h } from '../dom.js';

/**
 * One column per round: top square for the left player, bottom for the right.
 * Blue = the world's first decision, magenta = any other, ghost outline = not yet played
 * (the classes are still called solid and hollow).
 * `movesA`/`movesB` are decision indices for the rounds on show. `label` is the
 * text alternative read by screen readers. Returns { node, show(n) }.
 */
export function roundStrip({ movesA, movesB, label }) {
	const columns = movesA.map(() => {
		const top = h('div', { class: 'sq' });
		const bottom = h('div', { class: 'sq' });
		return { top, bottom, node: h('div', { class: 'round' }, [top, bottom]) };
	});
	const paint = (el, played, move) => {
		el.className = !played ? 'sq' : move === 0 ? 'sq sq-solid' : 'sq sq-hollow';
	};
	const show = (n) => {
		columns.forEach((col, i) => {
			paint(col.top, i < n, movesA[i]);
			paint(col.bottom, i < n, movesB[i]);
		});
	};
	const node = h('div', { class: 'scroll-box', tabindex: '0', role: 'img', 'aria-label': label }, [
		h(
			'div',
			{ class: 'round-strip', 'aria-hidden': 'true', style: { '--rounds': String(columns.length) } },
			columns.map((c) => c.node)
		)
	]);
	show(columns.length);
	return { node, show };
}

/** The always-visible key for the two kinds of square. */
export function stripLegend({ top, bottom, solid, hollow }) {
	return h('div', { class: 'legend meta' }, [
		h('div', null, `Top row ${top} · Bottom row ${bottom}`),
		h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-solid', 'aria-hidden': 'true' }), solid]),
		h('div', { class: 'legend-item' }, [h('span', { class: 'swatch swatch-hollow', 'aria-hidden': 'true' }), hollow])
	]);
}
