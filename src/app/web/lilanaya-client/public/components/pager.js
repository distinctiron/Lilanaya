import { h } from '../dom.js';
import { button } from './button.js';

/**
 * A long list shown one windowful at a time, so the screen it is on never grows past the window.
 * `rows`: the nodes of the list; `wrap(rows)`: the list (a table, a <ul>) around the rows on show;
 * `rowHeight` and `reserve`: a first guess at a row's height and at what else is on the screen,
 * in pixels. After drawing, rows are taken off the page until its screen fits the window.
 * `start`: the index of a row that must be on show first.
 */
export function pager({ rows, wrap, rowHeight = 48, reserve = 320, min = 3, start = 0, label = 'Page' }) {
	const list = h('div');
	const status = h('span', { class: 'meta', role: 'status' });
	const previous = button('Previous', { onclick: () => go(page - 1) });
	const next = button('Next', { onclick: () => go(page + 1) });
	const nav = h('div', { class: 'row pager-nav' }, [previous, status, next]);
	const node = h('div', { class: 'pager stack' }, [list, nav]);
	let per = rows.length;
	let page = 0;
	let first = start; // a row that stays on show when the page size changes

	const draw = () => {
		const pages = Math.max(1, Math.ceil(rows.length / per));
		page = Math.min(Math.max(0, Math.floor(first / per)), pages - 1);
		list.replaceChildren(wrap(rows.slice(page * per, (page + 1) * per)));
		nav.hidden = pages === 1;
		status.textContent = `${label} ${page + 1} of ${pages}`;
		previous.disabled = page === 0;
		next.disabled = page === pages - 1;
	};
	const go = (to) => {
		first = to * per;
		draw();
	};
	const fit = () => {
		per = Math.min(rows.length, Math.max(min, Math.floor((window.innerHeight - reserve) / rowHeight)));
		draw();
		// The screen it is on: a group of screens sharing the window, or else its own.
		const group = node.closest('.screen-group');
		const frame = group && getComputedStyle(group).display !== 'contents' ? group : node.closest('.screen');
		if (!frame) return;
		const room = parseFloat(getComputedStyle(frame).minHeight) || window.innerHeight;
		while (per > min && frame.offsetHeight > room + 1) {
			per -= 1;
			draw();
		}
	};
	const onResize = () => (node.isConnected ? fit() : window.removeEventListener('resize', onResize));
	window.addEventListener('resize', onResize);
	if (rows.length) {
		fit();
		requestAnimationFrame(() => node.isConnected && fit()); // again once it is on the page and can be measured
	}
	return node;
}
