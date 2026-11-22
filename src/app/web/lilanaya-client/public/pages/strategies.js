import { ApiError, deleteJson, getJson } from '../api.js';
import { avatar, revealAvatar } from '../components/avatar.js';
import { button, linkButton } from '../components/button.js';
import { fold } from '../components/fold.js';
import { codeBlock, machineTable } from '../components/machine.js';
import { pager } from '../components/pager.js';
import { screen, screenGroup, screens } from '../components/screen.js';
import { sectionHeading } from '../components/section-heading.js';
import { h, prefersReducedMotion } from '../dom.js';
import { getUser, signUpUrl } from '../session.js';

const BLOCK_MS = 30; // one block of a new strategy's avatar

function card(s, { refresh }, face, open = false) {
	const tags = [s.mode === 'code' ? 'Code' : 'Rules'];
	if (s.mine) tags.push(s.visibility === 'public' ? 'Public' : 'Private');
	else if (!s.builtin && s.owner_name) tags.push(`By ${s.owner_name}`);
	const errors = h('p', { role: 'alert', hidden: true });
	const actions = s.mine
		? h('p', { class: 'row' }, [
				linkButton('Edit', `/strategies/${s.id}/edit`, { 'aria-label': `Edit ${s.name}` }),
				button('Delete', {
					'aria-label': `Delete ${s.name}`,
					onclick: async () => {
						if (!window.confirm(`Delete "${s.name}"? Runs it already played in stay as they are.`)) return;
						try {
							await deleteJson(`/api/strategies/${s.id}`);
							refresh();
						} catch (error) {
							if (!(error instanceof ApiError)) console.error(error);
							errors.textContent = error instanceof ApiError ? error.message : 'The strategy could not be deleted.';
							errors.hidden = false;
						}
					}
				})
			])
		: null;
	// Folded: avatar, name and tags, so many strategies fit on a screen. Folded out: what it does.
	return fold({
		open,
		group: 'strategy', // one open at a time, so a screen of strategies stays a screen
		summary: [h('h3', { class: 'section-heading avatar-heading' }, [face ?? avatar(s.id, { size: 32 }), s.name]), h('span', { class: 'meta fold-tags' }, tags.join(' · '))],
		body: [s.description ? h('p', null, s.description) : null, s.mode === 'code' ? codeBlock(s) : machineTable(s), actions, errors]
	});
}

export async function strategiesPage(ctx) {
	const [user, { strategies }] = await Promise.all([getUser(), getJson('/api/strategies')]);
	const mine = strategies.filter((s) => s.mine);
	const builtin = strategies.filter((s) => s.builtin);
	const others = strategies.filter((s) => !s.mine && !s.builtin);

	// Straight after a strategy is made (the editor sends ?new=<id>), its avatar is built block by block.
	const fresh = mine.find((s) => s.id === ctx.query.get('new'));
	let freshCard = null;
	if (fresh) {
		ctx.replaceUrl('/strategies');
		const still = prefersReducedMotion();
		const face = avatar(fresh.id, { size: 32, build: !still });
		freshCard = card(fresh, ctx, face, true);
		let shown = 0;
		const timer = still ? 0 : setInterval(() => (revealAvatar(face, ++shown) <= shown ? clearInterval(timer) : null), BLOCK_MS);
		ctx.onLeave(() => clearInterval(timer));
		requestAnimationFrame(() => freshCard.scrollIntoView({ block: 'nearest' }));
	}
	// Each group is a screen of its own; a group too long for the window is shown a page at a time.
	const group = (title, list, reserve) => [
		sectionHeading(title),
		pager({
			rowHeight: 60,
			reserve,
			start: Math.max(0, list.indexOf(fresh)),
			rows: list.map((s) => (s === fresh ? freshCard : card(s, ctx))),
			wrap: (rows) => h('ul', { class: 'cards folds' }, rows)
		})
	];
	const intro = h('div', { class: 'stack' }, [
		sectionHeading('Strategies', { level: 1 }),
		h('p', { class: 'prose' }, 'A strategy decides what to play each round from what has happened so far. Most are rules: a few states, one decision per state, and conditions for moving between them. A strategy can also be written as code. Open a strategy to see how it works.'),
		h('p', null, user ? linkButton('New strategy', '/strategies/new', { primary: true }) : linkButton('Sign up to make your own', signUpUrl('/strategies/new'))),
		user && !mine.length ? h('p', { class: 'meta' }, 'You have no strategies yet') : null
	]);
	const groups = [
		['Your strategies', mine],
		['Built in', builtin],
		['From other players', others]
	].filter(([, list]) => list.length);

	return {
		title: 'Strategies',
		// The first screen opens with what a strategy is, then the first group; the other groups follow.
		node: screens(
			groups.length
				? [
						screen([intro, h('div', { class: 'stack' }, group(...groups[0], 520))], { class: 'stack-lg' }),
						// Two groups left share a screen on a laptop, side by side.
						groups.length === 3
							? screenGroup(groups.slice(1).map(([title, list]) => screen(group(title, list, 240), { class: 'stack' })))
							: groups.slice(1).map(([title, list]) => screen(group(title, list, 240), { class: 'stack' }))
					]
				: [screen([intro])]
		)
	};
}
