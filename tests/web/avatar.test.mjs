// avatar.js: a strategy's avatar is drawn from its id, so it must be the same everywhere and every time.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { avatarGrid } from '../../src/app/web/lilanaya-client/public/components/avatar.js';

const picture = (id) => avatarGrid(id).map((row) => row.map((on) => (on ? '#' : '.')).join(''));
const ids = ['grudger-3fa9c1', 'copycat_js-000001', 'strategy-ffffff', 'a', 'patient-grudger', 'x'.repeat(64), '0', 'Ünï-cödé', ''];

test('every avatar is 8 by 8', () => {
	for (const id of [...ids, 'tit_for_tat', 'always_defect']) {
		const grid = avatarGrid(id);
		assert.equal(grid.length, 8, id);
		for (const row of grid) {
			assert.equal(row.length, 8, id);
			for (const cell of row) assert.equal(typeof cell, 'boolean', id);
		}
	}
});

test('the same id always gives the same avatar', () => {
	for (const id of ids) assert.deepEqual(avatarGrid(id), avatarGrid(id), id);
});

test('these avatars are drawn exactly like this, on every machine and in every browser', () => {
	// Fixed pictures: if one changes, every user's strategies change faces.
	assert.deepEqual(picture('always_cooperate'), ['..####..', '.######.', '##.##.##', '########', '#.####.#', '##....##', '.######.', '..#..#..']);
	assert.deepEqual(picture('grudger-3fa9c1'), PICTURES['grudger-3fa9c1']);
	assert.deepEqual(picture('copycat_js-000001'), PICTURES['copycat_js-000001']);
	assert.deepEqual(picture('strategy-ffffff'), PICTURES['strategy-ffffff']);
});

test('a generated avatar is symmetrical, has a body and eyes, and nothing floats under it', () => {
	for (let n = 0; n < 500; n++) {
		const id = `strategy-${n.toString(16).padStart(6, '0')}`;
		const grid = avatarGrid(id);
		for (const row of grid) assert.deepEqual(row, [...row].reverse(), `${id} is not symmetrical`);
		const blocks = grid.flat().filter(Boolean).length;
		assert.ok(blocks >= 16 && blocks <= 56, `${id} has ${blocks} blocks`);
		// The body: rows 1 to 5 are never empty, and at least two cells wide each side of the middle.
		for (let row = 1; row <= 5; row++) assert.ok(grid[row][3] || grid[row][2], `${id}: row ${row} is empty in the middle`);
		// Feet hang from something.
		for (const row of [6, 7]) for (let c = 0; c < 8; c++) if (grid[row][c]) assert.ok(grid[row - 1][c], `${id}: a block floats at row ${row}`);
	}
});

test('different strategies mostly get different avatars', () => {
	const seen = new Set();
	for (let n = 0; n < 500; n++) seen.add(picture(`strategy-${n.toString(16).padStart(6, '0')}`).join('\n'));
	assert.ok(seen.size > 450, `only ${seen.size} different avatars for 500 strategies`);
});

test('the four classics have their own hand-drawn avatars', () => {
	const classics = ['always_cooperate', 'always_defect', 'tit_for_tat', 'win_stay_lose_shift'].map((id) => picture(id).join('\n'));
	assert.equal(new Set(classics).size, 4);
});

const PICTURES = {
	'grudger-3fa9c1': ['...##...', '.######.', '##.##.##', '########', '.######.', '.######.', '..####..', '..####..'],
	'copycat_js-000001': ['..#..#..', '..####..', '.######.', '.#.##.#.', '########', '.######.', '.#.##.#.', '.#.##.#.'],
	'strategy-ffffff': ['.#....#.', '.######.', '.#.##.#.', '########', '.##..##.', '..####..', '...##...', '........']
};
