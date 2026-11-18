import { h } from '../dom.js';

/** A real <button>. `primary` gives the solid white block; otherwise black with a white border. */
export function button(label, { primary = false, type = 'button', ...attrs } = {}) {
	return h('button', { type, class: primary ? 'btn btn-primary' : 'btn', ...attrs }, label);
}

/** A link that looks like a button, for navigation. */
export function linkButton(label, href, { primary = false, ...attrs } = {}) {
	return h('a', { href, class: primary ? 'btn btn-primary' : 'btn', ...attrs }, label);
}
