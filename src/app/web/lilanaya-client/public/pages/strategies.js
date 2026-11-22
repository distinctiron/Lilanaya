import { ApiError, deleteJson, getJson } from '../api.js';
import { avatar, revealAvatar } from '../components/avatar.js';
import { button, linkButton } from '../components/button.js';
import { fold } from '../components/fold.js';
import { codeBlock, machineTable } from '../components/machine.js';
import { sectionHeading } from '../components/section-heading.js';
import { h, prefersReducedMotion } from '../dom.js';
import { getUser, signInUrl } from '../session.js';

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
	const group = (title, list) =>
		list.length ? h('div', { class: 'stack' }, [sectionHeading(title), h('ul', { class: 'cards folds' }, list.map((s) => (s === fresh ? freshCard : card(s, ctx))))]) : null;

	return {
		title: 'Strategies',
		node: h('section', { class: 'stack-lg' }, [
			h('div', { class: 'stack' }, [
				sectionHeading('Strategies', { level: 1 }),
				h('p', { class: 'prose' }, 'A strategy decides what to play each round from what has happened so far. Most are rules: a few states, one decision per state, and conditions for moving between them. A strategy can also be written as code. Open a strategy to see how it works.'),
				h('p', null, user ? linkButton('New strategy', '/strategies/new', { primary: true }) : linkButton('Sign in to make your own', signInUrl('/strategies/new')))
			]),
			user && !mine.length ? h('p', { class: 'meta' }, 'You have no strategies yet') : null,
			group('Your strategies', mine),
			group('Built in', builtin),
			group('From other players', others)
		])
	};
}
