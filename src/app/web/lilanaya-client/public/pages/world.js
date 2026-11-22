import { getJson } from '../api.js';
import { avatar, avatarName } from '../components/avatar.js';
import { linkButton } from '../components/button.js';
import { sectionHeading } from '../components/section-heading.js';
import { fmt, fmtValue, h } from '../dom.js';

const POLICY = {
	all_pairs: 'All pairs: every actor meets every other actor once, in actor order',
	round_robin: 'Round robin',
	random: 'All pairs, in an order drawn from the seed'
};
const AGGREGATE = { sum: 'Total payoff', avg: 'Average payoff per round' };

function rules(world) {
	const rows = [
		['Decisions', world.decisions.allowed.join(', ')],
		['Outcomes', (world.outcomes.allowed.length ? world.outcomes.allowed : world.decisions.allowed.flatMap((a) => world.decisions.allowed.map((b) => `${a}/${b}`))).join(', ')],
		['Pairing', POLICY[world.interaction.policy] ?? world.interaction.policy],
		['Rounds per match', String(world.time.iterations)],
		['Default seed', String(world.determinism.rng_seed)],
		['Ranked by', AGGREGATE[world.scoring.aggregate] ?? world.scoring.aggregate],
		['Version', world.metadata.version]
	];
	return h('dl', { class: 'rules' }, rows.map(([term, value]) => [h('dt', { class: 'meta' }, term), h('dd', null, value)]));
}

export function payoffTable(world) {
	const d = world.decisions.allowed;
	return h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Payoff matrix' }, [
		h('table', { class: 'grid-table' }, [
			h('caption', { class: 'meta' }, 'Each cell: you, opponent'),
			h('thead', null, h('tr', null, [h('td', null), d.map((b) => h('th', { scope: 'col' }, `Opponent plays ${b}`))])),
			h(
				'tbody',
				null,
				d.map((a) => h('tr', null, [h('th', { scope: 'row' }, `You play ${a}`), d.map((b) => h('td', null, world.payoff.matrix[a][b].join(', ')))]))
			)
		])
	]);
}

function leaderboard(board) {
	if (board.run_id == null) {
		return h('p', null, 'No runs yet. Run a simulation to fill the board.');
	}
	return [
		h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Leaderboard' }, [
			h('table', { class: 'grid-table' }, [
				h('thead', null, h('tr', null, ['Rank', 'Actor', 'Average', 'Variance', 'Matches'].map((t) => h('th', { scope: 'col' }, t)))),
				h(
					'tbody',
					null,
					board.entries.map((e) =>
						h('tr', null, [
							h('td', { class: 'rank' }, String(e.rank)),
							h('th', { scope: 'row' }, avatarName(e.actor_id, e.name)),
							h('td', { class: 'num' }, fmt(e.average)),
							h('td', { class: 'num' }, fmt(e.variance)),
							h('td', { class: 'num' }, String(e.entries))
						])
					)
				)
			])
		]),
		h('p', { class: 'meta' }, [`Seed ${board.seed} · ${board.iterations} rounds per match · `, h('a', { href: `/runs/${board.run_id}` }, `See run ${board.run_id}`)])
	];
}

function indicators(world, history) {
	const defs = world.kpis.definitions;
	if (!defs.length) return h('p', null, 'This world has no saved indicators.');
	const latest = history.snapshots.at(-1);
	const change = (delta) => (delta == null ? 'First run' : delta === 0 ? 'No change' : `${delta > 0 ? '+' : '-'}${fmtValue(Math.abs(delta))}`);
	return [
		h('div', { class: 'scroll-box', tabindex: '0', role: 'group', 'aria-label': 'Indicators' }, [
			h('table', { class: 'grid-table' }, [
				h('thead', null, h('tr', null, ['Indicator', 'Latest run', 'Change'].map((t) => h('th', { scope: 'col' }, t)))),
				h(
					'tbody',
					null,
					defs.map((def) => {
						const v = latest?.values[def.id];
						return h('tr', null, [
							h('th', { scope: 'row' }, def.label),
							h('td', { class: 'num' }, v ? fmtValue(v.value) : 'No runs yet'),
							h('td', { class: 'num' }, v ? change(v.delta) : '')
						]);
					})
				)
			])
		]),
		latest ? h('p', { class: 'meta' }, ['Change is against the run before · ', h('a', { href: `/runs/${latest.run_id}` }, `See run ${latest.run_id}`)]) : null
	];
}

export async function worldPage({ params }) {
	const id = encodeURIComponent(params.id);
	const [world, board, history, { strategies }] = await Promise.all([
		getJson(`/api/worlds/${id}`),
		getJson(`/api/worlds/${id}/leaderboard`),
		getJson(`/api/worlds/${id}/kpis?limit=1`),
		getJson('/api/strategies')
	]);
	return {
		title: world.metadata.name,
		node: h('div', { class: 'stack-lg' }, [
			h('section', { class: 'stack' }, [
				h('nav', { class: 'meta', 'aria-label': 'Breadcrumb' }, [h('a', { href: '/worlds' }, 'Worlds'), ` > ${world.metadata.name}`]),
				sectionHeading(world.metadata.name, { level: 1 }),
				h('p', { class: 'prose' }, world.metadata.description),
				h('p', null, linkButton('Run simulation', `/runs/new?world=${world.id}`, { primary: true }))
			]),
			h('div', { class: 'columns' }, [
				h('div', { class: 'stack-lg' }, [
					h('section', { class: 'stack' }, [sectionHeading('Rules'), rules(world)]),
					h('section', { class: 'stack' }, [sectionHeading('Payoff matrix'), payoffTable(world)])
				]),
				h('div', { class: 'stack-lg' }, [
					h('section', { class: 'stack' }, [sectionHeading('High scores'), leaderboard(board)]),
					h('section', { class: 'stack' }, [sectionHeading('Indicators'), indicators(world, history)]),
					h('section', { class: 'stack' }, [
						sectionHeading('Baseline actors'),
						h('ul', { class: 'plain-list' }, strategies.map((s) => h('li', { class: 'avatar-heading' }, [avatar(s.id, { size: 32 }), h('div', null, [h('strong', null, s.name), h('br'), s.description])]))),
						h('p', null, h('a', { href: '/strategies' }, 'How each strategy works'))
					])
				])
			])
		])
	};
}
