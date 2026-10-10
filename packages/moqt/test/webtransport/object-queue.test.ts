import { describe, expect, test } from "bun:test";
import { createObjectQueue } from "../../src/webtransport/object-queue.ts";

/** Lets promise reactions that are already due run. */
const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

/** Whether each promise has resolved by now. */
async function resolvedFlags(promises: Promise<void>[]): Promise<boolean[]> {
	const flags = promises.map(() => false);
	promises.forEach((promise, index) => {
		void promise.then(() => {
			flags[index] = true;
		});
	});
	await settle();
	return flags;
}

describe("createObjectQueue", () => {
	test("releases every waiting push once the consumer has caught up", async () => {
		const queue = createObjectQueue<number>(1, () => Promise.resolve());
		// Three producers wait at the same time, as three subgroup streams of one
		// subscription do.
		const pushes = [queue.push(1), queue.push(2), queue.push(3)];
		expect(await resolvedFlags(pushes)).toEqual([false, false, false]);

		const reader = queue.readable.getReader();
		const read = [
			(await reader.read()).value,
			(await reader.read()).value,
			(await reader.read()).value,
		];
		expect(read).toEqual([1, 2, 3]);
		expect(await resolvedFlags(pushes)).toEqual([true, true, true]);
	});

	test("resolves a push at once while the queue has room", async () => {
		const queue = createObjectQueue<number>(2, () => Promise.resolve());
		expect(await resolvedFlags([queue.push(1)])).toEqual([true]);
	});

	test("end() closes the stream after the queued items and releases waiting pushes", async () => {
		const queue = createObjectQueue<number>(1, () => Promise.resolve());
		const pushes = [queue.push(1), queue.push(2)];
		queue.end();
		expect(await resolvedFlags(pushes)).toEqual([true, true]);

		const reader = queue.readable.getReader();
		expect(await reader.read()).toEqual({ done: false, value: 1 });
		expect(await reader.read()).toEqual({ done: false, value: 2 });
		expect(await reader.read()).toEqual({ done: true, value: undefined });
		expect(await resolvedFlags([queue.push(3)])).toEqual([true]);
	});

	test("end(error) lets the consumer read what is queued, then fails the stream", async () => {
		const queue = createObjectQueue<number>(4, () => Promise.resolve());
		void queue.push(1);
		void queue.push(2);
		queue.end(new Error("cut off"));

		const reader = queue.readable.getReader();
		expect(await reader.read()).toEqual({ done: false, value: 1 });
		expect(await reader.read()).toEqual({ done: false, value: 2 });
		await expect(reader.read()).rejects.toThrow("cut off");
	});

	test("end(error) fails a read that was already waiting", async () => {
		const queue = createObjectQueue<number>(4, () => Promise.resolve());
		const reading = queue.readable.getReader().read();
		queue.end(new Error("cut off"));
		await expect(reading).rejects.toThrow("cut off");
	});

	test("a read that waits gets the item pushed after it", async () => {
		const queue = createObjectQueue<number>(4, () => Promise.resolve());
		const reading = queue.readable.getReader().read();
		void queue.push(7);
		expect(await reading).toEqual({ done: false, value: 7 });
	});

	test("discard() closes the stream at once and drops what was not read", async () => {
		const queue = createObjectQueue<number>(1, () => Promise.resolve());
		const pushes = [queue.push(1), queue.push(2)];
		queue.discard();
		expect(await resolvedFlags(pushes)).toEqual([true, true]);
		expect(await queue.readable.getReader().read()).toEqual({
			done: true,
			value: undefined,
		});
	});

	test("cancelling the stream runs onCancel and releases waiting pushes", async () => {
		let cancelled = 0;
		const queue = createObjectQueue<number>(1, () => {
			cancelled += 1;
			return Promise.resolve();
		});
		const pushes = [queue.push(1), queue.push(2)];
		await queue.readable.cancel();
		expect(cancelled).toBe(1);
		expect(await resolvedFlags(pushes)).toEqual([true, true]);
	});
});
