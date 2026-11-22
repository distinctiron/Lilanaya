// A pocket Prisoner's Dilemma, for the landing page and the films only.
// Same payoffs and the same four classics as the built-in world; real runs happen on the server.

export const PAYOFF = { CC: [3, 3], CD: [0, 5], DC: [5, 0], DD: [1, 1] };
const flip = (move) => (move === 'C' ? 'D' : 'C');
export const copyThem = (mine, theirs) => theirs;

export const CLASSICS = {
	tit_for_tat: { name: 'Tit for Tat', text: 'Cooperates first, then copies the other player’s last move.', first: 'C', next: copyThem },
	always_cooperate: { name: 'Always Cooperate', text: 'Cooperates every round, whatever happens.', first: 'C', next: () => 'C' },
	always_defect: { name: 'Always Defect', text: 'Defects every round, whatever happens.', first: 'D', next: () => 'D' },
	win_stay_lose_shift: {
		name: 'Win-Stay Lose-Shift',
		text: 'Cooperates first. Keeps its move after a good round and switches after a bad one.',
		first: 'C',
		next: (mine, theirs) => (theirs === 'C' ? mine : flip(mine))
	}
};
/** Plays `rounds` rounds; returns both players' moves and their running scores. */
export function playMatch(a, b, rounds) {
	const out = { a: [], b: [], scoreA: [], scoreB: [] };
	let [ma, mb, sa, sb] = [a.first, b.first, 0, 0];
	for (let i = 0; i < rounds; i++) {
		if (i) [ma, mb] = [a.next(ma, mb), b.next(mb, ma)];
		const [pa, pb] = PAYOFF[ma + mb];
		out.a.push(ma);
		out.b.push(mb);
		out.scoreA.push((sa += pa));
		out.scoreB.push((sb += pb));
	}
	return out;
}

/** Everyone against everyone: total score per player id. */
export function playAll(players, rounds) {
	const totals = Object.fromEntries(players.map((p) => [p.id, 0]));
	for (let i = 0; i < players.length; i++) {
		for (let j = i + 1; j < players.length; j++) {
			const m = playMatch(players[i].rule, players[j].rule, rounds);
			totals[players[i].id] += m.scoreA.at(-1);
			totals[players[j].id] += m.scoreB.at(-1);
		}
	}
	return totals;
}
