import { h } from '../dom.js';

/**
 * A card that folds: one line that is always on show (`summary`) and the rest (`body`) behind it.
 * A native <details>, so it works with the keyboard and without script. Returns an <li> for a
 * `.cards.folds` list.
 */
export function fold({ summary, body, open = false }) {
	return h('li', { class: 'card fold' }, h('details', { open: open ? '' : null }, [h('summary', null, summary), h('div', { class: 'fold-body stack' }, body)]));
}
