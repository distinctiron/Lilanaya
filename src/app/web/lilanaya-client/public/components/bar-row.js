import { h } from '../dom.js';

/**
 * One leaderboard row (a <tr>): rank, name (text or a node), bar, score, and an optional text label
 * so that the bar's colour is never the only cue.
 * `share` is the score as a fraction of the best score. Returns { row, grow(g) },
 * where grow(0..1) scales the bar for the build-in animation.
 */
export function barRow({ rank, name, score, share, solid, label }) {
	const bar = h('div', { class: solid ? 'bar bar-solid' : 'bar', 'aria-hidden': 'true' });
	const row = h('tr', null, [
		h('td', { class: 'rank' }, String(rank)),
		h('th', { scope: 'row', class: 'name' }, name),
		h('td', { class: 'bar-cell' }, bar),
		h('td', { class: 'score' }, score),
		label == null ? null : h('td', { class: 'label' }, label)
	]);
	const grow = (g) => {
		bar.style.width = `${Math.max(2, share * 100 * g).toFixed(1)}%`;
	};
	grow(1);
	return { row, grow };
}
