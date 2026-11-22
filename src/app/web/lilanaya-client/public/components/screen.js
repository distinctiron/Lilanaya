import { h } from '../dom.js';

/**
 * Every page is a column of screens. A screen fills the window and holds one thing to look at;
 * letting go of a scroll settles on the nearest screen (see `.screens` in style.css). A page
 * that returns anything else is put in a single screen by the router.
 */
export function screens(children, { class: extra = '', ...attrs } = {}) {
	return h('div', { ...attrs, class: `screens ${extra}`.trim() }, children);
}

/** One screen: a <section> whose content sits in the middle of the window. `class` sets the spacing between its parts. */
export function screen(children, { class: spacing = 'stack-md', ...attrs } = {}) {
	return h('section', { ...attrs, class: `screen ${spacing}`.trim() }, children);
}

/**
 * Screens that share the window where it is wide enough (a laptop): side by side, as one screen.
 * On a narrow window each is a screen of its own again. `class`: 'wide-left' gives the first one
 * two thirds of the width; a screen with the class `span` takes a row of its own under the others.
 */
export function screenGroup(children, { class: extra = '', ...attrs } = {}) {
	return h('div', { ...attrs, class: `screen-group ${extra}`.trim() }, children);
}
