// Sound for the films: effects and music are made here with the Web Audio API (no files);
// the voice is one recorded clip per caption, served from /audio/voice/.
// Everything is off until the visitor turns sound on, and the films never depend on it.

const KEY = 'lilanaya.sound';
const VOICE_DIR = '/audio/voice/en/';

let on = false;
try {
	on = localStorage.getItem(KEY) === 'on';
} catch {
	// Storage blocked: sound starts off.
}
const listeners = new Set();
let ctx = null; // AudioContext, made on the first click that needs it
let master = null;

export const soundOn = () => on;

/** Calls `fn(on)` whenever sound is turned on or off; returns a function that stops the calls. */
export function onSoundChange(fn) {
	listeners.add(fn);
	return () => listeners.delete(fn);
}

/** Browsers only let sound start from a click or key press, so this must be called from one. */
function wake() {
	if (!ctx) {
		const Ctx = window.AudioContext ?? window.webkitAudioContext;
		if (!Ctx) return;
		ctx = new Ctx();
		master = ctx.createGain();
		master.gain.value = 0.9;
		master.connect(ctx.destination);
	}
	if (ctx.state === 'suspended') ctx.resume();
}

export function setSound(value) {
	on = value;
	try {
		localStorage.setItem(KEY, on ? 'on' : 'off');
	} catch {
		// Not remembered, that is all.
	}
	if (on) {
		wake();
		loadVoices();
	}
	tune();
	for (const fn of listeners) fn(on);
}

// Sound left on from an earlier visit can only start once the visitor touches the page.
if (on) {
	const first = () => {
		wake();
		tune();
	};
	addEventListener('pointerdown', first, { once: true });
	addEventListener('keydown', first, { once: true });
}

const ready = () => on && ctx?.state === 'running';

// --- effects: short square-wave blips ---

const hz = (midi) => 440 * 2 ** ((midi - 69) / 12);

function blip(midi, at, seconds, { type = 'square', level = 0.05, out = master } = {}) {
	const osc = ctx.createOscillator();
	const gain = ctx.createGain();
	osc.type = type;
	osc.frequency.value = hz(midi);
	gain.gain.setValueAtTime(level, at);
	gain.gain.setValueAtTime(level, at + seconds * 0.7);
	gain.gain.linearRampToValueAtTime(0, at + seconds);
	osc.connect(gain).connect(out);
	osc.start(at);
	osc.stop(at + seconds);
}

// name → [[midi note, start in seconds, length in seconds], …]
const EFFECTS = {
	move: [[69, 0, 0.05]],
	point: [[76, 0, 0.05], [81, 0.06, 0.07]],
	card: [[64, 0, 0.07], [69, 0.08, 0.07], [76, 0.16, 0.12]],
	step: [[72, 0, 0.04]],
	win: [[72, 0, 0.09], [76, 0.1, 0.09], [79, 0.2, 0.09], [84, 0.3, 0.22]]
};

/** Plays the effect `name` if sound is on. */
export function sfx(name) {
	if (!ready()) return;
	for (const [midi, start, length] of EFFECTS[name] ?? []) blip(midi, ctx.currentTime + start, length);
}

// --- music: a quiet four-bar loop, played for as long as some film is running ---

const STEP = 0.2; // seconds per eighth note
const BARS = [
	{ bass: 45, lead: [69, 72, 76, 72, 69, 72, 76, 72] }, // A minor
	{ bass: 41, lead: [69, 72, 77, 72, 69, 72, 77, 72] }, // F
	{ bass: 48, lead: [67, 72, 76, 72, 67, 72, 76, 72] }, // C
	{ bass: 43, lead: [67, 71, 74, 71, 67, 71, 74, 79] } // G
];
const MUSIC_LEVEL = 0.5;
const MUSIC_UNDER_VOICE = 0.18;

const wanting = new Set(); // films that are running
let musicOut = null;
let musicTimer = 0;
let nextStep = 0; // index of the next eighth note
let nextAt = 0; // when it is due, in context time
let speaking = 0;

function schedule() {
	// Keep a quarter of a second of notes queued; the timer only tops the queue up.
	while (nextAt < ctx.currentTime + 0.25) {
		const bar = BARS[Math.floor(nextStep / 8) % BARS.length];
		const beat = nextStep % 8;
		blip(bar.lead[beat], nextAt, STEP * 0.8, { level: 0.022, out: musicOut });
		if (beat % 4 === 0) blip(bar.bass, nextAt, STEP * 3.2, { type: 'triangle', level: 0.07, out: musicOut });
		nextStep++;
		nextAt += STEP;
	}
}

/** Starts or stops the music to match what is wanted right now. */
function tune() {
	const play = ready() && wanting.size > 0;
	if (play && !musicTimer) {
		if (!musicOut) {
			musicOut = ctx.createGain();
			musicOut.connect(master);
		}
		musicOut.gain.value = speaking ? MUSIC_UNDER_VOICE : MUSIC_LEVEL;
		nextAt = ctx.currentTime + 0.05;
		schedule();
		musicTimer = setInterval(schedule, 100);
	} else if (!play && musicTimer) {
		clearInterval(musicTimer);
		musicTimer = 0;
	}
}

/** A film says whether it is running; music plays while any film is. `who` is any value unique to the film. */
export function wantMusic(who, wanted) {
	if (wanted) wanting.add(who);
	else wanting.delete(who);
	tune();
}

function duck(by) {
	speaking += by;
	if (musicOut && ctx) musicOut.gain.setTargetAtTime(speaking ? MUSIC_UNDER_VOICE : MUSIC_LEVEL, ctx.currentTime, 0.08);
}

// --- voice: one clip per caption ---

/** FNV-1a, as eight hex digits: the name of a caption's clip. */
export function clipName(text) {
	let x = 0x811c9dc5;
	for (let i = 0; i < text.length; i++) {
		x ^= text.charCodeAt(i);
		x = Math.imul(x, 0x01000193);
	}
	return (x >>> 0).toString(16).padStart(8, '0');
}

let clips = null; // clip name → file name, from the manifest; null until loaded
let loading = null;

function loadVoices() {
	loading ??= fetch(`${VOICE_DIR}manifest.json`)
		.then((response) => (response.ok ? response.json() : {}))
		.catch(() => ({}))
		.then((manifest) => {
			clips = manifest.clips ?? {};
		});
	return loading;
}
if (on) loadVoices();

/**
 * Starts speaking the caption `text`, if sound is on and there is a clip for exactly this text.
 * Returns null when there is nothing to play, otherwise { done, pause(), resume(), stop() };
 * `done` resolves when the clip has finished (or failed, or was stopped).
 */
export function speak(text) {
	const file = ready() ? clips?.[clipName(text)] : null;
	if (!file) return null;
	const audio = new Audio(VOICE_DIR + file);
	let finish;
	let over = false;
	const done = new Promise((resolve) => (finish = resolve));
	const end = () => {
		if (over) return;
		over = true;
		duck(-1);
		finish();
	};
	duck(1);
	audio.addEventListener('ended', end);
	audio.addEventListener('error', end);
	audio.play().catch(end);
	return {
		done,
		pause: () => audio.pause(),
		resume: () => void (over || audio.play().catch(end)),
		stop() {
			audio.pause();
			end();
		}
	};
}
