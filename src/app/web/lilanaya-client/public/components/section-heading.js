import { h } from '../dom.js';

/** Display-font heading. Level 1 is the page title (focusable, so the router can move focus to it). */
export function sectionHeading(text, { level = 2 } = {}) {
	return h(`h${level}`, { class: 'section-heading', tabindex: level === 1 ? '-1' : null }, text);
}
