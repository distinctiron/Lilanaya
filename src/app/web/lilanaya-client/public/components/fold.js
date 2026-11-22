import { h } from '../dom.js';

/**
 * A card that folds: one line that is always on show (`summary`) and the rest (`body`) behind it.
 * A native <details>, so it works with the keyboard and without script. Folds that share a `group` open one at a time, so a list stays about as tall as it was. Returns an <li> for a
 * `.cards.folds` list.
 */
export function fold({ summary, body, open = false, group = null }) {
	return h('li', { class: 'card fold' }, h('details', { open: open ? '' : null, name: group }, [h('summary', null, summary), h('div', { class: 'fold-body stack' }, body)]));
}
