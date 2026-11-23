// What a plan, a pack and a price are called on the pages. Pure functions: no page, no network.

const count = (n, one, many) => `${n.toLocaleString('en')} ${n === 1 ? one : many}`;

export const strategies = (n) => count(n, 'strategy', 'strategies');
export const runs = (n) => count(n, 'run', 'runs');

/** An amount in a currency's smallest unit as people write it: (899, 'eur') is "€8.99". */
export function money(amount, currency) {
	try {
		return new Intl.NumberFormat('en', { style: 'currency', currency: currency.toUpperCase() }).format(amount / 100);
	} catch {
		return `${(amount / 100).toFixed(2)} ${String(currency).toUpperCase()}`;
	}
}

/** The day of an ISO timestamp (UTC) as people write it: "23 December 2026". */
export function day(iso) {
	const date = new Date(iso);
	if (!iso || Number.isNaN(date.getTime())) return '';
	return new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' }).format(date);
}

/** What a plan lets the AI write, in a sentence. `ai` is a plan's `ai_requests`: `{ limit, per }`. */
export function aiSentence(ai) {
	return ai.per === 'world' ? `The AI writes ${strategies(ai.limit)} for you in each world. They never come back.` : `The AI writes ${strategies(ai.limit)} a month for you, across all worlds.`;
}

/** A pack in a few words: "10 strategies + 100 runs". */
export const packWords = (pack) => `${strategies(pack.strategies)} + ${runs(pack.runs)}`;

/**
 * How many strategies the AI may still write for this user in `worldId`: `{ plan, pack, total }`.
 * `usage` is `/api/usage`. The plan's own are used first, then what is left of the packs.
 */
export function strategiesLeft(usage, worldId) {
	const ai = usage?.ai_requests;
	if (!ai) return { plan: 0, pack: 0, total: 0 };
	const used = ai.per === 'world' ? (ai.used_by_world[worldId] ?? 0) : ai.used_this_month;
	const plan = Math.max(0, ai.limit - used);
	const pack = Math.max(0, usage.packs?.strategies_left ?? 0);
	return { plan, pack, total: plan + pack };
}

/** The line under the description box in the editor: what is left before one is used. */
export function allowanceLine(usage, world) {
	const ai = usage.ai_requests;
	const perWorld = ai.per === 'world';
	const { plan, pack, total } = strategiesLeft(usage, world.id);
	const scope = perWorld ? `in ${world.metadata.name}` : `until ${day(usage.period_end)}`;
	const fromPacks = pack > 0 ? ` · ${strategies(pack)} left in your packs` : '';
	if (total === 0) {
		return `The AI has written all ${strategies(ai.limit)} your plan gives you ${perWorld ? `in ${world.metadata.name}. They do not come back` : `this month. They come back on ${day(usage.period_end)}`} · You can still build the rules by hand, or write the strategy in code`;
	}
	if (plan === 0) return `Your plan’s own are used up ${perWorld ? `in ${world.metadata.name}` : `until ${day(usage.period_end)}`}${fromPacks} · Each press of “Turn into rules” uses one`;
	return `The AI can write ${plan} of ${ai.limit} more ${plan === 1 ? 'strategy' : 'strategies'} for you ${scope}${fromPacks} · Each press of “Turn into rules” uses one${perWorld ? ' · They never come back' : ''}`;
}
