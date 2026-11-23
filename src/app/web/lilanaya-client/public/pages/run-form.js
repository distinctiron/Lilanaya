import { ApiError, getJson, postJson } from '../api.js';
import { avatar } from '../components/avatar.js';
import { button, linkButton } from '../components/button.js';
import { screen, screenGroup } from '../components/screen.js';
import { sectionHeading } from '../components/section-heading.js';
import { h, prefersReducedMotion } from '../dom.js';
import { getUser } from '../session.js';
import { logInPrompt } from './sign-in.js';

// Mirrors the API's limits (src/app/api/src/routes.rs); the server still has the final say.
const MAX_ITERATIONS = 10000;
const MAX_SEED = Number.MAX_SAFE_INTEGER;

export async function runFormPage({ query, navigate }) {
	if (!(await getUser())) return { title: 'New run', node: logInPrompt('start a run') };
	const [{ worlds }, { strategies }] = await Promise.all([getJson('/api/worlds'), getJson('/api/strategies')]);
	const initial = worlds.find((w) => w.id === query.get('world')) ?? worlds[0];

	const worldSelect = h(
		'select',
		{ id: 'run-world', name: 'world' },
		worlds.map((w) => h('option', { value: w.id, selected: w.id === initial.id }, w.metadata.name))
	);
	const iterations = h('input', {
		id: 'run-iterations',
		name: 'iterations',
		type: 'number',
		inputmode: 'numeric',
		min: '1',
		max: String(MAX_ITERATIONS),
		step: '1',
		required: true,
		'aria-describedby': 'run-iterations-hint'
	});
	const seed = h('input', {
		id: 'run-seed',
		name: 'seed',
		type: 'number',
		inputmode: 'numeric',
		min: '0',
		max: String(MAX_SEED),
		step: '1',
		required: true,
		'aria-describedby': 'run-seed-hint'
	});
	const order = h(
		'select',
		{ id: 'run-order', name: 'order', 'aria-describedby': 'run-order-hint' },
		[
			['all_pairs', 'In actor order'],
			['shuffle_all', 'Random: shuffle the pairs once'],
			['sample_per_round', 'Random: reshuffle every round']
		].map(([value, label]) => h('option', { value }, label))
	);
	const useDefaults = (world) => {
		iterations.value = String(Math.min(world.time.iterations, MAX_ITERATIONS));
		seed.value = String(world.determinism.rng_seed);
		order.value = world.interaction.policy === 'random' ? world.randomness.schedule_mode : 'all_pairs';
	};
	useDefaults(initial);
	worldSelect.addEventListener('change', () => useDefaults(worlds.find((w) => w.id === worldSelect.value)));

	// Built-ins are entered unless unticked; the user's own and other players' are opt-in.
	const boxes = strategies.map((s) => h('input', { type: 'checkbox', name: 'actors', value: s.id, id: `actor-${s.id}`, checked: s.builtin }));
	const option = (s) => {
		const note = [s.mine && s.visibility === 'private' ? 'Private: the run will be private too' : null, !s.mine && !s.builtin ? `By ${s.owner_name}` : null, s.description].filter(Boolean).join(' · ');
		// The note is cut to two lines so the whole field fits a screen; the full text is one hover away.
		return h('label', { class: 'check', for: `actor-${s.id}` }, [
			boxes[strategies.indexOf(s)],
			avatar(s.id, { size: 24 }),
			h('span', { class: 'check-text' }, [h('strong', null, s.name), note ? h('span', { class: 'check-note muted', title: note }, note) : null])
		]);
	};
	const group = (title, list) => (list.length ? [h('p', { class: 'meta' }, title), h('div', { class: 'check-grid' }, list.map(option))] : null);
	const actorsError = h('p', { class: 'error-box', role: 'alert', hidden: true }, 'Pick at least two actors.');
	const actorsField = h('fieldset', { class: 'stack-sm', 'aria-describedby': 'run-actors-hint' }, [
		h('legend', { class: 'field-label' }, 'Actors'),
		h('p', { class: 'meta', id: 'run-actors-hint' }, 'Pick at least two. Every actor plays every other actor.'),
		group('Built in', strategies.filter((s) => s.builtin)),
		group('Yours', strategies.filter((s) => s.mine)),
		group('From other players', strategies.filter((s) => !s.mine && !s.builtin)),
		actorsError
	]);

	const errors = h('div', { class: 'error-box', role: 'alert', hidden: true });
	const showError = (message, details = []) => {
		errors.replaceChildren(h('p', null, message), details.length ? h('ul', null, details.map((d) => h('li', null, `${d.path}: ${d.message}`))) : '');
		errors.hidden = false;
		for (const [field, el] of [['iterations', iterations], ['seed', seed]]) {
			if (details.some((d) => d.path === field)) el.setAttribute('aria-invalid', 'true');
			else el.removeAttribute('aria-invalid');
		}
	};

	const submit = button('Run simulation', { primary: true, type: 'submit' });
	// The second screen: how long and in what order, and the button that starts the run.
	const settings = screen(
		[
			sectionHeading('Settings'),
			h('div', { class: 'stack-lg form' }, [
				h('div', { class: 'field-row' }, [
					h('div', { class: 'field' }, [
						h('label', { class: 'field-label', for: 'run-iterations' }, 'Rounds per match'),
						iterations,
						h('p', { class: 'meta', id: 'run-iterations-hint' }, `1 to ${MAX_ITERATIONS}`)
					]),
					h('div', { class: 'field' }, [
						h('label', { class: 'field-label', for: 'run-seed' }, 'Seed'),
						seed,
						h('p', { class: 'meta', id: 'run-seed-hint' }, 'Same seed, same result')
					])
				]),
				h('div', { class: 'field' }, [
					h('label', { class: 'field-label', for: 'run-order' }, 'Match order'),
					order,
					h('p', { class: 'meta', id: 'run-order-hint' }, 'Every pair still plays every round · A random order is drawn from the seed')
				]),
				errors,
				h('div', null, submit)
			])
		],
		{ class: 'stack-md' }
	);
	const form = h(
		'form',
		{
			class: 'screens',
			onsubmit: async (e) => {
				e.preventDefault();
				const actors = boxes.filter((b) => b.checked).map((b) => b.value);
				actorsError.hidden = actors.length >= 2;
				if (actors.length < 2) {
					// Said where the actors are picked, a screen up from the button that was pressed.
					boxes[0].focus();
					return;
				}
				errors.hidden = true;
				submit.disabled = true;
				submit.textContent = 'Running…';
				try {
					const run = await postJson('/api/runs', {
						world_id: worldSelect.value,
						actors,
						iterations: Number(iterations.value),
						seed: Number(seed.value),
						...(order.value === 'all_pairs' ? { policy: 'all_pairs' } : { policy: 'random', schedule_mode: order.value })
					});
					await navigate(`/runs/${run.id}`);
				} catch (error) {
					submit.disabled = false;
					submit.textContent = 'Run simulation';
					if (!(error instanceof ApiError)) console.error(error);
					showError(error instanceof ApiError ? error.message : 'The run could not be started.', error.details ?? []);
					// Out of runs: say where more can be had.
					if (error instanceof ApiError && error.code === 'run_limit_reached') errors.append(h('p', null, linkButton('See plans and packs', '/plans')));
				}
			}
		},
		// Who plays, and how long and in what order: side by side on a laptop, two screens on a phone.
		screenGroup([
			screen(
				[
					sectionHeading('New run', { level: 1 }),
					h('div', { class: 'field form' }, [h('label', { class: 'field-label', for: 'run-world' }, 'World'), worldSelect]),
					actorsField,
					h('div', { class: 'next-step' }, button('Next: settings', { onclick: () => settings.scrollIntoView({ block: 'start', behavior: prefersReducedMotion() ? 'auto' : 'smooth' }) }))
				],
				{ class: 'stack' }
			),
			settings
		], { class: 'wide-left' })
	);

	return { title: 'New run', node: form };
}
