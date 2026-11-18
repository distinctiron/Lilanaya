import { fmt, h } from '../dom.js';

/** "Tit for Tat" → "TFT", "Win-Stay Lose-Shift" → "WSLS". */
export function abbreviate(name) {
	return name
		.split(/[\s-]+/)
		.filter(Boolean)
		.map((word) => word[0].toUpperCase())
		.join('');
}

/**
 * Head-to-head table. `actors`: [{ id, name }]; `values[row][col]`: the row actor's
 * average against the column actor, or null where no match was played.
 * Cells shade from dark grey (lowest) to white (highest) and link to that match
 * via `hrefFor(rowActor, colActor)`; `onPick(rowActor, colActor, event)` runs on click.
 * Returns { node, select(aId, bId) } — select marks the cell of the match on show.
 */
export function matrixGrid({ actors, values, hrefFor, onPick, caption }) {
	const played = values.flat().filter((v) => v != null);
	const min = Math.min(...played);
	const max = Math.max(...played);
	const links = [];

	const head = h('tr', null, [
		h('td', null),
		actors.map((a) => h('th', { scope: 'col' }, h('abbr', { title: a.name }, abbreviate(a.name))))
	]);
	const rows = actors.map((rowActor, r) =>
		h('tr', null, [
			h('th', { scope: 'row' }, h('abbr', { title: rowActor.name }, abbreviate(rowActor.name))),
			actors.map((colActor, c) => {
				const v = values[r][c];
				if (v == null) return h('td', { class: 'cell cell-empty' }, h('span', { 'aria-label': 'not played' }, '·'));
				const k = max === min ? 255 : Math.round(30 + ((v - min) / (max - min)) * 225);
				const link = h(
					'a',
					{
						href: hrefFor(rowActor, colActor),
						'aria-label': `${rowActor.name} scored ${fmt(v)} per round against ${colActor.name}. Show this match.`,
						onclick: (e) => onPick?.(rowActor, colActor, e)
					},
					fmt(v)
				);
				links.push({ a: rowActor.id, b: colActor.id, link });
				return h('td', { class: 'cell', style: { background: `rgb(${k},${k},${k})`, color: k > 140 ? '#000000' : '#FFFFFF' } }, link);
			})
		])
	);

	const node = h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': caption }, [
		h('table', { class: 'matrix', style: { '--n': String(actors.length) } }, [
			h('caption', { class: 'visually-hidden' }, caption),
			h('thead', null, head),
			h('tbody', null, rows)
		])
	]);
	const select = (aId, bId) => {
		for (const { a, b, link } of links) {
			if (a === aId && b === bId) link.setAttribute('aria-current', 'true');
			else link.removeAttribute('aria-current');
		}
	};
	return { node, select };
}
