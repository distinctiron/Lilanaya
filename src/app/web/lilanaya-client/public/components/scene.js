/**
 * A clock that can be stopped. Animations only let time pass through `await clock.wait(ms)`,
 * so stopping the clock freezes them exactly where they are and starting it again carries on.
 * `run(on)` starts or stops it (it starts stopped); `drop()` forgets every pending wait, which
 * abandons whatever was waiting for good.
 */
export function clock() {
	const waits = new Set();
	let on = false;
	const arm = (w) => {
		w.since = performance.now();
		w.timer = setTimeout(() => {
			waits.delete(w);
			w.resolve();
		}, w.left);
	};
	return {
		wait(ms) {
			return new Promise((resolve) => {
				const w = { left: ms, timer: null, since: 0, resolve };
				waits.add(w);
				if (on) arm(w);
			});
		},
		run(value) {
			on = value;
			for (const w of waits) {
				if (on && w.timer == null) arm(w);
				else if (!on && w.timer != null) {
					clearTimeout(w.timer);
					w.timer = null;
					w.left = Math.max(0, w.left - (performance.now() - w.since));
				}
			}
		},
		drop() {
			for (const w of waits) clearTimeout(w.timer);
			waits.clear();
		}
	};
}
