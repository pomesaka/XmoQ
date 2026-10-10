import { expect, test } from "bun:test";
import { createStreamSlots } from "../../src/webtransport/stream-slots.ts";

const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

/** Whether the promise has settled by the next task. */
async function settled(promise: Promise<void>): Promise<boolean> {
	let done = false;
	void promise.then(() => {
		done = true;
	});
	await settle();
	return done;
}

test("a place is free until as many streams as the limit hold one", async () => {
	const slots = createStreamSlots(2);
	expect(await settled(slots.free())).toBe(true);
	slots.take();
	expect(await settled(slots.free())).toBe(true);
	slots.take();
	expect(await settled(slots.free())).toBe(false);
});

test("the wait for a place ends when a stream has ended", async () => {
	const slots = createStreamSlots(1);
	slots.take();
	const waiting = slots.free();
	expect(await settled(waiting)).toBe(false);
	slots.release();
	expect(await settled(waiting)).toBe(true);
	// The place is the next stream's, and the one after it waits again.
	slots.take();
	expect(await settled(slots.free())).toBe(false);
});

// The loop that takes streams waits here when the session ends, and no
// stream has to end for that: nothing then lets it go but `close()`.
test("the wait for a place ends when the slots are closed, with every stream still holding its place", async () => {
	const slots = createStreamSlots(1);
	slots.take();
	const waiting = slots.free();
	expect(await settled(waiting)).toBe(false);
	slots.close();
	expect(await settled(waiting)).toBe(true);
});

test("after the slots are closed no place is waited for", async () => {
	const slots = createStreamSlots(1);
	slots.take();
	slots.close();
	expect(await settled(slots.free())).toBe(true);
});

test("two that wait for a place are both let go", async () => {
	const slots = createStreamSlots(1);
	slots.take();
	const first = slots.free();
	const second = slots.free();
	slots.close();
	expect([await settled(first), await settled(second)]).toEqual([true, true]);
});
