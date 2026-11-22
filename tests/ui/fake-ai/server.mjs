// Stands in for Vercel AI Gateway in the UI tests: the language model (Chat Completions) and
// Jev, the screen in front of it (Decisions). No dependencies; run with `node server.mjs`.
//
// What it answers depends only on words in the description, so a test picks its answer by what it types:
//
//   the screen (POST /v1/decisions)
//     "ignore your instructions"   -> instructions to the AI: turned down
//     "pancake"                    -> unrelated: turned down
//     anything else                -> a strategy description: let through
//
//   the model (POST /v1/chat/completions)
//     "flip a coin"                -> rules, with a flag: breaks the rules of the game
//     "most of the time"           -> rules, with a flag: an assumption that code could do exactly
//     "read my opponent"           -> no rules: the description rests on breaking the rules
//     "shopping list"              -> no rules: not a strategy
//     "impossible move"            -> rules the validator rejects, both times it is asked
//     anything else                -> Grim Trigger: cooperate until betrayed, then defect for ever
//
// GET /__calls lists what both were sent, so a test can prove that a text never reached the model.
import { createServer } from 'node:http';

const PORT = 8090;
const LABELS = ['strategy_description', 'instructions_to_the_ai', 'abusive_or_harmful', 'unrelated'];
const calls = { screened: [], asked: [] };

const grim = (extra = {}) => ({
	name: 'Grim Trigger',
	description: 'Cooperates until the opponent defects once, then defects for ever.',
	states: ['NICE', 'GRIM'],
	initial_state: 'NICE',
	action_map: { NICE: 'C', GRIM: 'D' },
	transitions: [{ from: 'NICE', to: 'GRIM', guard: { op: 'last_opponent_action', is: 'D' } }],
	rationale: 'Two states: NICE cooperates and GRIM defects. One defection by the opponent moves it to GRIM, which it never leaves.',
	flags: [],
	...extra
});

function verdict(text) {
	const lower = text.toLowerCase();
	const [label, strategy] = lower.includes('ignore your instructions') ? ['instructions_to_the_ai', 0.01] : lower.includes('pancake') ? ['unrelated', 0.02] : ['strategy_description', 0.97];
	const rest = (1 - strategy) / 3;
	return {
		answers: [{ type: 'choice', name: 'kind', choice: label, confidence: 0.9, probabilities: LABELS.map((value) => ({ value, probability: value === 'strategy_description' ? strategy : rest })) }],
		model: 'typesafe-ai/jev',
		usage: { input_tokens: 300, output_tokens: 20, total_tokens: 320 }
	};
}

function rules(description) {
	const lower = description.toLowerCase();
	if (lower.includes('read my opponent')) return { error: 'A strategy cannot know who its opponent is or what it will play: both players decide at the same time.', breaks_rules: true };
	if (lower.includes('shopping list')) return { error: 'This is a shopping list, not a description of how to play.' };
	if (lower.includes('impossible move')) return grim({ action_map: { NICE: 'C', GRIM: 'WIN' } });
	if (lower.includes('flip a coin'))
		return grim({ flags: [{ kind: 'violation', asked: 'flip a coin', why: 'A strategy may not use chance: the same history must always lead to the same decision', instead: 'Left out.', code: false }] });
	if (lower.includes('most of the time'))
		return grim({
			flags: [{ kind: 'assumption', asked: 'defects most of the time', why: 'A state machine cannot count how often the opponent has defected over the whole match', instead: 'It reacts to the first defection.', code: true }]
		});
	return grim();
}

function completion(messages) {
	const shown = messages.filter((m) => m.role === 'user')[0]?.content ?? '';
	const description = shown.includes('<<<DESCRIPTION') ? shown.slice(shown.indexOf('<<<DESCRIPTION')) : shown;
	return {
		choices: [{ index: 0, message: { role: 'assistant', content: JSON.stringify(rules(description)) }, finish_reason: 'stop' }],
		usage: { prompt_tokens: 900, completion_tokens: 100, total_tokens: 1000 }
	};
}

const send = (res, status, body) => {
	res.writeHead(status, { 'content-type': 'application/json' });
	res.end(JSON.stringify(body));
};

createServer((req, res) => {
	let raw = '';
	req.on('data', (chunk) => (raw += chunk));
	req.on('end', () => {
		if (req.method === 'GET' && req.url === '/__calls') return send(res, 200, calls);
		if (req.method !== 'POST') return send(res, 404, { error: 'not found' });
		// The real gateway refuses a request without the key.
		if (!/^Bearer \S+/.test(req.headers.authorization ?? '')) return send(res, 401, { error: 'missing key' });
		let body;
		try {
			body = JSON.parse(raw);
		} catch {
			return send(res, 400, { error: 'not JSON' });
		}
		if (req.url === '/v1/decisions') {
			calls.screened.push(body.input);
			return send(res, 200, verdict(String(body.input ?? '')));
		}
		if (req.url === '/v1/chat/completions') {
			calls.asked.push(body.messages.map((m) => m.content).join('\n'));
			return send(res, 200, completion(body.messages ?? []));
		}
		send(res, 404, { error: 'not found' });
	});
}).listen(PORT, () => console.log(`fake AI gateway listening on ${PORT}`));
