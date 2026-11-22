import { h } from '../dom.js';

/** Display-font heading. Focusable from code, so the router can move focus to the page title or to a linked section. */
export function sectionHeading(text, { level = 2 } = {}) {
	return h(`h${level}`, { class: 'section-heading', tabindex: '-1' }, text);
}
