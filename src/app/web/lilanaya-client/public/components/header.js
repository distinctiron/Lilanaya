import { h } from '../dom.js';
import { signInUrl } from '../session.js';
import { linkButton } from './button.js';

const NAV = [
	{ label: 'Worlds', href: '/worlds', active: (p) => p === '/' || p.startsWith('/worlds') },
	{ label: 'Strategies', href: '/strategies', active: (p) => p.startsWith('/strategies') },
	{ label: 'Runs', href: '/runs', active: (p) => p.startsWith('/runs') && p !== '/runs/new' }
];

/**
 * Draws the site header into `container`, marking the nav link for `path` as current.
 * `user` is the signed-in user, null when signed out, undefined while not yet known.
 */
export function renderHeader(container, path, user) {
	const onSignIn = path.startsWith('/sign-in');
	const account =
		user === undefined
			? null
			: user
				? linkButton(user.display_name, '/profile', { 'aria-label': `Profile of ${user.display_name}`, 'aria-current': path === '/profile' ? 'page' : null, class: 'btn btn-name' })
				: linkButton('Sign in', onSignIn ? '/sign-in' : signInUrl(path === '/' ? '/worlds' : undefined));
	container.replaceChildren(
		h('header', { class: 'site-header' }, [
			h('a', { class: 'logo', href: '/' }, 'Lilanaya'),
			h(
				'nav',
				{ class: 'site-nav', 'aria-label': 'Main' },
				NAV.map((item) => h('a', { href: item.href, 'aria-current': item.active(path) ? 'page' : null }, item.label))
			),
			h('div', { class: 'row' }, [account, linkButton('New run', '/runs/new', { primary: true })])
		])
	);
}
