// Strategy avatars: a small pixel figure for every strategy, white blocks on the black field.
// The built-in classics have hand-drawn figures; every other strategy gets one drawn from its
// id, so the same strategy looks the same on every screen and nothing has to be stored.

const SIZE = 8; // cells per side
const SVG = 'http://www.w3.org/2000/svg';

// '#' is a block. Each row is 8 cells.
const CLASSICS = {
	// Round and smiling: it always cooperates.
	always_cooperate: ['..####..', '.######.', '##.##.##', '########', '#.####.#', '##....##', '.######.', '..#..#..'],
	// Horns and a scowl: it always defects.
	always_defect: ['#......#', '##....##', '.######.', '#..##..#', '##.##.##', '########', '.#.##.#.', '#.#..#.#'],
	// One eye open, one eye shut: it does to you what you just did to it.
	tit_for_tat: ['.##..##.', '########', '#..##.##', '#..##..#', '########', '##....##', '.######.', '.##..##.'],
	// A dial on its head that flips when a round goes badly.
	win_stay_lose_shift: ['...##...', '..####..', '.######.', '##.##.##', '########', '.#....#.', '.######.', '##....##']
};

/** FNV-1a: a stable 32-bit number for a string. */
function hash(text) {
	let x = 0x811c9dc5;
	for (let i = 0; i < text.length; i++) {
		x ^= text.charCodeAt(i);
		x = Math.imul(x, 0x01000193);
	}
	return x >>> 0;
}

/** mulberry32: a small seeded generator, so a figure never depends on Math.random. */
function rng(seed) {
	let a = seed;
	return () => {
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

// Half-rows, written from the centre outwards (index 0 is next to the centre line).
const TOPS = ['....', '#...', '.#..', '..#.', '...#', '#..#', '.##.', '##..'];
const FEET = ['#...', '.#..', '..#.', '#.#.', '.#.#', '##..', '.##.', '#..#'];

/** Draws a figure from a seed: a body two to four cells wide each side, eyes, sometimes a mouth, a top and feet. */
function generate(seed) {
	const next = rng(seed);
	const pick = (list) => list[Math.floor(next() * list.length)];
	const half = Array.from({ length: SIZE }, () => [false, false, false, false]);
	const put = (row, pattern) => {
		for (let c = 0; c < 4; c++) half[row][c] = pattern[c] === '#';
	};

	// Body: rows 1 to 5, the width drifting by at most one cell from row to row.
	const eyeRow = 2 + Math.floor(next() * 2);
	let width = 2 + Math.floor(next() * 3);
	for (let row = 1; row <= 5; row++) {
		width = Math.min(4, Math.max(2, width + Math.floor(next() * 3) - 1));
		if (row === eyeRow || row === eyeRow - 1) width = Math.max(3, width);
		for (let c = 0; c < width; c++) half[row][c] = true;
	}
	half[eyeRow][1] = false; // eyes
	if (next() < 0.5) half[eyeRow + 2][0] = false; // mouth
	if (next() < 0.4) half[eyeRow + 1][3] = true; // arms

	put(0, pick(TOPS));
	const feet = pick(FEET);
	put(6, feet);
	put(7, next() < 0.5 ? feet : pick(FEET));
	// Feet must hang from something: drop any cell with nothing above it.
	for (const row of [6, 7]) for (let c = 0; c < 4; c++) if (!half[row - 1][c]) half[row][c] = false;

	return half.map((cells) => [...cells].reverse().concat(cells));
}

/** The 8 × 8 grid of a strategy: rows of booleans, true where there is a block. */
export function avatarGrid(id) {
	const classic = CLASSICS[id];
	if (classic) return classic.map((row) => [...row].map((ch) => ch === '#'));
	return generate(hash(String(id)));
}

/**
 * The avatar of the strategy `id`, as an inline SVG `size` pixels square (use multiples of 8
 * so the blocks stay sharp). Decorative by default: the strategy's name is always next to it.
 * Pass `label` where the avatar stands alone. `build: true` leaves every block hidden for
 * `revealAvatar` to bring in.
 */
export function avatar(id, { size = 32, label = null, build = false } = {}) {
	const svg = document.createElementNS(SVG, 'svg');
	svg.setAttribute('class', 'avatar');
	svg.setAttribute('viewBox', `0 0 ${SIZE} ${SIZE}`);
	svg.setAttribute('width', String(size));
	svg.setAttribute('height', String(size));
	svg.setAttribute('shape-rendering', 'crispEdges');
	if (label) {
		svg.setAttribute('role', 'img');
		svg.setAttribute('aria-label', label);
	} else svg.setAttribute('aria-hidden', 'true');
	avatarGrid(id).forEach((row, y) =>
		row.forEach((on, x) => {
			if (!on) return;
			const rect = document.createElementNS(SVG, 'rect');
			rect.setAttribute('x', String(x));
			rect.setAttribute('y', String(y));
			rect.setAttribute('width', '1');
			rect.setAttribute('height', '1');
			if (build) rect.setAttribute('visibility', 'hidden');
			svg.append(rect);
		})
	);
	return svg;
}

/** Shows the first `count` blocks of an avatar made with `build: true` (all of them by default). */
export function revealAvatar(svg, count = Infinity) {
	[...svg.children].forEach((rect, i) => {
		if (i < count) rect.removeAttribute('visibility');
		else rect.setAttribute('visibility', 'hidden');
	});
	return svg.children.length;
}

/** Avatar and name side by side, for lists, tables and headings. */
export function avatarName(id, name, { size = 24 } = {}) {
	const span = document.createElement('span');
	span.className = 'avatar-name';
	const text = document.createElement('span');
	text.textContent = name;
	span.append(avatar(id, { size }), text);
	return span;
}
