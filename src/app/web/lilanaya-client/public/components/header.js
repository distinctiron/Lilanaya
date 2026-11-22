import { h } from '../dom.js';
import { logInUrl, signUpUrl } from '../session.js';
import { button, linkButton } from './button.js';

const NAV = [
	{ label: 'How it works', href: '/how-it-works', active: (p) => p === '/how-it-works' },
	{ label: 'Worlds', href: '/worlds', active: (p) => p.startsWith('/worlds') },
	{ label: 'Strategies', href: '/strategies', active: (p) => p.startsWith('/strategies') },
	{ label: 'Runs', href: '/runs', active: (p) => p.startsWith('/runs') && p !== '/runs/new' }
];

/**
 * Draws the site header into `container`, marking the nav link for `path` as current.
 * `user` is the logged-in user, null when logged out, undefined while not yet known.
 */
export function renderHeader(container, path, user) {
	// Pages that fill the window (the front page's first screen) need to know how tall the header is.
	if (!container.dataset.measured && 'ResizeObserver' in window) {
		container.dataset.measured = 'yes';
		new ResizeObserver(() => document.documentElement.style.setProperty('--header-height', `${container.offsetHeight}px`)).observe(container);
	}
	// On the account pages themselves there is nowhere to come back to.
	const onAccountPage = path.startsWith('/log-in') || path.startsWith('/sign-up');
	const back = path === '/' ? '/worlds' : undefined;
	const account =
		user === undefined
			? []
			: user
				? [linkButton(user.display_name, '/profile', { 'aria-label': `Profile of ${user.display_name}`, 'aria-current': path === '/profile' ? 'page' : null, class: 'btn btn-name' })]
				: [linkButton('Log in', onAccountPage ? '/log-in' : logInUrl(back)), linkButton('Sign up', onAccountPage ? '/sign-up' : signUpUrl(back))];
	// Phone width only (style.css hides the button on wider windows): the links are folded behind it, so the header is one line.
	const toggle = button('Menu', {
		class: 'btn menu-toggle',
		'aria-expanded': 'false',
		'aria-controls': 'site-menu',
		onclick: () => {
			const open = toggle.getAttribute('aria-expanded') !== 'true';
			toggle.setAttribute('aria-expanded', String(open));
			toggle.textContent = open ? 'Close' : 'Menu';
			bar.classList.toggle('is-open', open);
		}
	});
	const bar = h('header', { class: 'site-header' }, [
		h('a', { class: 'logo', href: '/' }, 'Lilanaya'),
		toggle,
		h('div', { class: 'site-menu', id: 'site-menu' }, [
			h(
				'nav',
				{ class: 'site-nav', 'aria-label': 'Main' },
				NAV.map((item) => h('a', { href: item.href, 'aria-current': item.active(path) ? 'page' : null }, item.label))
			),
			h('div', { class: 'row site-account' }, [...account, linkButton('New run', '/runs/new', { primary: true })])
		])
	]);
	container.replaceChildren(bar);
}
