// What the pages of the demo do alike: the line that says how a page goes,
// a clock that runs in a background tab, and a count a second.

import { DEFAULT_RECONNECT_DELAY_MS } from "@xmoq/moqt";

/** The pause between sessions, as a page says it: what `keepPublishing` and `keepPlaying` wait where they are given no `againMs`. */
export const AGAIN = `${DEFAULT_RECONNECT_DELAY_MS / 1000} s`;

/**
 * Calls `tick` every `ms` milliseconds, also while the page is a tab in the
 * background.
 *
 * WHY not `setInterval` of the page: a browser slows the timers of a page
 * nobody looks at (Chromium: to one a second, and less after a while), and
 * stops `requestAnimationFrame` there. A publisher in a background tab
 * then sends a frame a second. The timers of a worker are not slowed with
 * the page, and a message from a worker is handled at once: so the clock
 * runs in a worker, and the work stays here. `?timer=page` is the page's
 * own timer, and `?timer=slow` that timer as a background tab has it, to
 * see the difference.
 */
export function every(ms: number, tick: () => void): void {
	const timer = new URLSearchParams(location.search).get("timer");
	if (timer === "page") {
		setInterval(tick, ms);
		return;
	}
	if (timer === "slow") {
		// What a browser makes of a page's timer in a background tab: once
		// a second at most. For seeing what that does, without a tab.
		setInterval(tick, Math.max(ms, 1000));
		return;
	}
	const clock = new Worker(
		URL.createObjectURL(
			new Blob([`setInterval(() => postMessage(0), ${ms});`], {
				type: "text/javascript",
			}),
		),
	);
	clock.addEventListener("message", tick);
}

/** How many times something happened in the last second: `count()` notes one, `rate()` says how many. */
export function perSecond(): { count(): void; rate(): number } {
	let times: number[] = [];
	return {
		count() {
			times.push(performance.now());
		},
		rate() {
			const since = performance.now() - 1000;
			times = times.filter((time) => time > since);
			return times.length;
		},
	};
}

export const say = (text: string): void => {
	const status = document.getElementById("status");
	if (status !== null) status.textContent = text;
};
