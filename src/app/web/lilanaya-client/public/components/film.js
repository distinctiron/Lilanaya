import { h, prefersReducedMotion } from '../dom.js';
import { button } from './button.js';
import { clock } from './scene.js';
import { clipName, onSoundChange, setSound, sfx, soundOn, speak, wantMusic } from './sound.js';

const IN_VIEW = 0.6; // share of the screen that must be visible for the film to play…
const OUT_OF_VIEW = 0.35; // …and the share below which it stops
const READ_MS_PER_LETTER = 50;
const READ_MS_AT_LEAST = 1700;
const BETWEEN_CHAPTERS_MS = 900;
const TYPE_MS_PER_LETTER = 16; // captions are typed out
const AFTER_CAPTION_MS = 300;
const AFTER_VOICE_MS = 500; // the pause after a spoken caption, instead of reading time
const CARD_MS = 1100; // the title card that opens each chapter
const TICK_MS = 100; // how often the progress bar moves

/**
 * A short explaining film: chapters that play one after another on a framed screen, each
 * narrated by captions that appear under the picture.
 *
 * `chapters`: [{ title, text, play }]. The titles are listed next to the screen, with the `text`
 * of the chapter on show, and picking one there jumps to that chapter. `aside` is an optional
 * node shown under that list (something the film points at, such as a table).
 * `async play(stage, t)` draws the chapter into the
 * empty `stage` and tells it:
 *   - `await t.say(text)` shows a caption;
 *   - `await t.wait(ms)` lets time pass;
 *   - `await t.hold()` waits long enough to read the caption on show;
 *   - `t.sfx(name)` plays a sound effect (see sound.js).
 *
 * So that it reads as something that is running, a film always shows it: a blinking light and
 * PLAYING in its top bar, a progress bar that fills chapter by chapter, a title card before each
 * chapter, captions that are typed out behind a blinking caret, and avatars that hop on the spot
 * (the last three are CSS on `.film.is-playing`). All of it stops when the film is paused.
 *
 * Sound is off until the viewer turns it on with the film's Sound button (one setting for all
 * films). Then there is music while a film runs, effects, and a voice: a caption that has a
 * recorded clip is spoken, and the film waits for the clip to end instead of timing the read.
 * `filmCaptions` lists every caption with the name of its clip, for whoever records them.
 *
 * The film starts by itself the first time most of its screen is in view, stops where it is
 * when the screen is scrolled away (or Pause is pressed) and carries on from there when it is
 * back. It plays once and then offers Replay. Under `prefers-reduced-motion` nothing moves:
 * each chapter is shown as its finished picture with its last caption, one step at a time.
 *
 * Returns { node, destroy }.
 */
/** Every caption of every film made so far: clip name → text. Filled as films measure themselves. */
export const filmCaptions = new Map();

export function film({ label, chapters, aside = null }) {
	const still = prefersReducedMotion();
	const time = clock();
	let instant = false; // drawing a finished picture: no time passes
	let index = 0;
	let status = 'idle'; // idle (not started), playing, paused (by the viewer), ended
	let inView = false;
	let run = 0; // goes up whenever playing starts somewhere new; an older run then gives up

	const stage = h('div', { class: 'scene-stage', 'aria-hidden': 'true' });
	const words = document.createTextNode('');
	const caption = h('p', { class: 'film-caption' }, [words, h('span', { class: 'film-caret', 'aria-hidden': 'true' })]);
	const counter = h('span', { class: 'px' });
	const fills = chapters.map(() => h('span', { class: 'film-fill' }));
	const screen = h('div', { class: 'film-screen' }, [
		h('div', { class: 'film-bar', 'aria-hidden': 'true' }, [
			h('span', { class: 'film-state' }, [h('span', { class: 'film-light' }), counter]),
			h('span', { class: 'film-segments' }, fills.map((fill) => h('span', { class: 'film-seg' }, fill)))
		]),
		stage,
		caption
	]);
	const lengths = chapters.map(() => 1); // each chapter's running time in ms, measured below
	let elapsed = 0; // of the chapter on show
	const main = button('Play', { onclick: () => press() });
	const sound = button('', { onclick: () => setSound(!soundOn()) });
	const paintSound = () => {
		sound.textContent = soundOn() ? 'Sound on' : 'Sound off';
		sound.setAttribute('aria-pressed', String(soundOn()));
	};
	paintSound();
	const forgetSound = onSoundChange(() => {
		paintSound();
		if (!soundOn()) hush();
		paint();
	});
	let voice = null; // the clip being spoken
	let spoken = false; // whether the caption on show was spoken
	const hush = () => {
		voice?.stop();
		voice = null;
	};
	const picks = chapters.map((chapter, i) => h('button', { type: 'button', class: 'film-pick', onclick: () => jump(i) }, [h('span', { class: 'film-pick-no' }, String(i + 1)), chapter.title]));

	const t = {
		get still() {
			return still || instant;
		},
		wait: (ms) => (still || instant ? Promise.resolve() : time.wait(ms)),
		async say(text) {
			if (still || instant) {
				words.data = text;
				return;
			}
			const mine = run;
			hush();
			const clip = (voice = speak(text));
			spoken = Boolean(clip);
			for (let i = 1; i <= text.length; i++) {
				words.data = text.slice(0, i);
				await time.wait(TYPE_MS_PER_LETTER);
			}
			await time.wait(AFTER_CAPTION_MS);
			if (clip) {
				await clip.done;
				// The film went somewhere else while this was being said: this pass ends here, for good.
				if (mine !== run) await new Promise(() => {});
			}
		},
		hold: () => t.wait(spoken ? AFTER_VOICE_MS : readingTime(words.data)),
		sfx: (name) => void (still || instant || sfx(name))
	};
	const readingTime = (text) => Math.max(READ_MS_AT_LEAST, text.length * READ_MS_PER_LETTER);

	/** Runs every chapter once on a stage nobody sees, adding up its waits, to learn how long it takes. */
	async function measure() {
		for (const [i, chapter] of chapters.entries()) {
			let total = CARD_MS;
			let said = '';
			const stopwatch = {
				still: false,
				wait: async (ms) => void (total += ms),
				say: async (text) => void ((said = text), filmCaptions.set(clipName(text), text), (total += text.length * TYPE_MS_PER_LETTER + AFTER_CAPTION_MS)),
				hold: async () => void (total += readingTime(said)),
				sfx() {}
			};
			await chapter.play(h('div'), stopwatch).catch((error) => console.error(error));
			lengths[i] = total;
		}
	}

	const running = () => status === 'playing' && inView;
	const ticker = still
		? 0
		: setInterval(() => {
				if (!running() || instant) return;
				elapsed += TICK_MS;
				fills[index].style.width = `${Math.min(100, (elapsed / lengths[index]) * 100).toFixed(1)}%`;
			}, TICK_MS);

	function paint() {
		time.run(running());
		wantMusic(node, running());
		if (running()) voice?.resume();
		else voice?.pause();
		node.classList.toggle('is-playing', running());
		node.classList.toggle('is-paused', status === 'paused');
		counter.textContent = still ? `${index + 1} / ${chapters.length}` : `${{ idle: 'Ready', playing: 'Playing', paused: 'Paused', ended: 'The end' }[status]} ${index + 1}/${chapters.length}`;
		fills.forEach((fill, i) => {
			if (status === 'ended' || i < index) fill.style.width = '100%';
			else if (i > index || (i === index && elapsed === 0)) fill.style.width = '0';
		});
		picks.forEach((el, i) => (i === index ? el.setAttribute('aria-current', 'step') : el.removeAttribute('aria-current')));
		main.textContent = still ? (index < chapters.length - 1 ? 'Next step' : 'First step') : { idle: 'Play', playing: 'Pause', paused: 'Play', ended: 'Replay' }[status];
	}

	function open(i) {
		index = i;
		elapsed = 0;
		stage.replaceChildren();
		words.data = '';
		paint();
	}

	/** Draws chapter `i` as its finished picture, at once. */
	function poster(i) {
		hush();
		time.drop();
		run++;
		instant = true;
		open(i);
		return chapters[i]
			.play(stage, t)
			.catch((error) => console.error(error))
			.then(() => {
				instant = false;
			});
	}

	async function playFrom(first) {
		time.drop(); // whatever was playing stops at its current wait and is forgotten
		const mine = ++run;
		hush();
		status = 'playing';
		try {
			for (let i = first; i < chapters.length; i++) {
				open(i);
				stage.append(h('div', { class: 'film-card' }, [h('span', { class: 'px' }, `Chapter ${i + 1} of ${chapters.length}`), h('span', { class: 'film-card-title' }, chapters[i].title)]));
				t.sfx('card');
				await t.wait(CARD_MS);
				stage.replaceChildren();
				await chapters[i].play(stage, t);
				await t.wait(BETWEEN_CHAPTERS_MS);
				if (mine !== run) return;
			}
		} catch (error) {
			console.error(error);
		}
		status = 'ended';
		t.sfx('win');
		paint();
	}

	function jump(i) {
		if (still) return void poster(i);
		inView = true; // the viewer is looking at it; the observer corrects this if not
		playFrom(i);
	}

	function press() {
		if (still) return void poster((index + 1) % chapters.length);
		if (status === 'playing') status = 'paused';
		else if (status === 'paused') status = 'playing';
		else return jump(0);
		if (status === 'playing') inView = true;
		paint();
	}

	const observer =
		'IntersectionObserver' in window && !still
			? new IntersectionObserver(
					([entry]) => {
						// A screen taller than the window counts as in view once it fills most of the window.
						const share = Math.max(entry.intersectionRatio, entry.rootBounds ? entry.intersectionRect.height / entry.rootBounds.height : 0);
						if (share >= IN_VIEW) inView = true;
						else if (share < OUT_OF_VIEW) inView = false;
						if (inView && status === 'idle') playFrom(0);
						else paint();
					},
					{ threshold: [0, OUT_OF_VIEW, IN_VIEW, 1] }
				)
			: null;

	const node = h('div', { class: 'film', role: 'group', 'aria-label': label }, [
		h('div', { class: 'stack' }, [screen, h('div', { class: 'row' }, [main, sound])]),
		h('div', { class: 'stack-md' }, [h('ol', { class: 'film-chapters' }, chapters.map((chapter, i) => h('li', { class: 'stack-sm' }, [picks[i], h('p', null, chapter.text)]))), aside])
	]);

	// Measuring and the first poster finish before the browser handles anything else, so nothing can start in between.
	(still ? Promise.resolve() : measure()).then(() => poster(0));
	observer?.observe(screen);

	return {
		node,
		destroy() {
			observer?.disconnect();
			clearInterval(ticker);
			time.drop();
			run++;
			hush();
			forgetSound();
			wantMusic(node, false);
		}
	};
}
