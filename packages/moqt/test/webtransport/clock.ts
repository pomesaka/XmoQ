// For the tests whose clock stands still and is moved by hand
// (`jest.useFakeTimers` of bun:test drives `setTimeout`), so that "not yet"
// and "now" are a millisecond apart and no test waits.

import { jest } from "bun:test";

/** Lets what is queued run: the streams of the fake transport move with promises alone. */
export async function flush(): Promise<void> {
	for (let turn = 0; turn < 200; turn++) await Promise.resolve();
}

/** Moves the clock, and lets what that set off run. */
export async function pass(milliseconds: number): Promise<void> {
	await flush();
	jest.advanceTimersByTime(milliseconds);
	await flush();
}

/** How a promise stands: "pending", "resolved", or what it rejected with. */
export async function state(promise: Promise<unknown>): Promise<unknown> {
	let seen: unknown = "pending";
	promise.then(
		() => {
			seen = "resolved";
		},
		(error: unknown) => {
			seen = error;
		},
	);
	await flush();
	return seen;
}
