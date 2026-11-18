/**
 * Creates an element. `attrs`: `class`, `style` (object of CSS properties, custom
 * properties included), `on…` listeners, or plain attributes (null/false are skipped,
 * true gives an empty attribute). Children: nodes, strings, numbers, nested arrays.
 */
export function h(tag, attrs, ...children) {
	const el = document.createElement(tag);
	for (const [key, value] of Object.entries(attrs ?? {})) {
		if (value == null || value === false) continue;
		if (key === 'class') el.className = value;
		else if (key === 'style' && typeof value === 'object') {
			for (const [prop, v] of Object.entries(value)) el.style.setProperty(prop, v);
		} else if (key.startsWith('on') && typeof value === 'function') el.addEventListener(key.slice(2), value);
		else el.setAttribute(key, value === true ? '' : value);
	}
	el.append(...children.flat(Infinity).filter((c) => c != null && c !== false));
	return el;
}

/** Fixed-decimals number formatting for scores. */
export const fmt = (n, digits = 2) => Number(n).toFixed(digits);

/** A KPI-style number: whole numbers as they are, others to two decimals. */
export const fmtValue = (n) => (Number.isInteger(n) ? String(n) : fmt(n));

export const prefersReducedMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
