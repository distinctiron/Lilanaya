// Minimal client-side router on the History API. nginx serves index.html for
// every path that is not a file, so deep links and reloads work.

import { screen, screens } from './components/screen.js';

/** Turns '/runs/:id' into a matcher returning the params, or null. */
function compile(pattern) {
	const names = [];
	const source = pattern.replace(/:([a-z]+)/g, (_, name) => {
		names.push(name);
		return '([^/]+)';
	});
	const re = new RegExp(`^${source}/?$`);
	return (path) => {
		const m = re.exec(path);
		if (!m) return null;
		return Object.fromEntries(names.map((name, i) => [name, decodeURIComponent(m[i + 1])]));
	};
}

/**
 * routes: [[pattern, page]], first match wins. A page is
 * `async (ctx) => ({ title, node })`, with ctx = { params, query, navigate, refresh, replaceUrl, onLeave }.
 * hooks: { main, onNavigate(path), notFound, errorPage(error, retry) }.
 */
export function startRouter(routes, { main, onNavigate, notFound, errorPage }) {
	const table = routes.map(([pattern, page]) => [compile(pattern), page]);
	let leave = [];
	let token = 0;
	let shown = null; // path and query of the page on show; a change of only the #section does not draw it again
	const here = () => location.pathname + location.search;

	/** Scrolls to the section the address names (`/#ranking`) and puts focus on its heading. False if there is none. */
	function toSection() {
		const target = location.hash.length > 1 ? document.getElementById(decodeURIComponent(location.hash.slice(1))) : null;
		if (!target || !main.contains(target)) return false;
		target.scrollIntoView({ block: 'start' });
		(target.querySelector('h1, h2, h3') ?? target).focus({ preventScroll: true });
		return true;
	}

	async function render({ initial = false } = {}) {
		const mine = ++token;
		for (const fn of leave.splice(0)) fn();
		const path = location.pathname;
		onNavigate(path);

		let page = notFound;
		let params = {};
		for (const [match, candidate] of table) {
			const found = match(path);
			if (found) {
				page = candidate;
				params = found;
				break;
			}
		}
		const pending = [];
		const ctx = {
			params,
			query: new URLSearchParams(location.search),
			navigate,
			refresh: () => render(),
			replaceUrl: (url) => history.replaceState(null, '', url),
			onLeave: (fn) => pending.push(fn)
		};

		let view;
		try {
			view = await page(ctx);
		} catch (error) {
			// API errors (not found, unreachable) are expected and shown on the page; anything else is a bug.
			if (error?.name !== 'ApiError') console.error(error);
			view = errorPage(error, () => render());
		}
		if (mine !== token) {
			// The user navigated again while this page was loading.
			for (const fn of pending) fn();
			return;
		}
		leave = pending;
		document.title = view.title ? `${view.title} · Lilanaya` : 'Lilanaya';
		// Every page is made of screens; one that is not gets a single screen to itself.
		main.replaceChildren(view.node.matches?.('.screens, .landing') ? view.node : screens([screen([view.node], { class: '' })]));
		shown = here();
		if (toSection()) return;
		if (!initial) {
			window.scrollTo(0, 0);
			// Move keyboard and screen-reader focus to the new page.
			(main.querySelector('h1') ?? main).focus({ preventScroll: true });
		}
	}

	function navigate(url) {
		history.pushState(null, '', url);
		return render();
	}

	window.addEventListener('popstate', () => (here() === shown ? toSection() || window.scrollTo(0, 0) : render()));
	document.addEventListener('click', (e) => {
		if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
		const a = e.target.closest?.('a[href]');
		if (!a || a.target || a.hasAttribute('download') || a.origin !== location.origin) return;
		if (a.pathname.startsWith('/api/') || a.pathname.startsWith('/fonts/')) return;
		e.preventDefault();
		if (a.href === location.href) return void toSection();
		if (a.pathname + a.search === shown && a.hash) {
			// Another section of the page on show: move there without drawing the page again.
			history.pushState(null, '', a.href);
			return void toSection();
		}
		navigate(a.href);
	});

	return render({ initial: true });
}
