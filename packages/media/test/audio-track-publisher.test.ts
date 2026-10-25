import { describe, expect, test } from "bun:test";
import { SubgroupClosedError } from "@xmoq/moqt";
import { createAudioTrackPublisher } from "../src/index.ts";
import { fakeFetch, fakeSubscribe } from "./fake-track.ts";

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
const NAMESPACE = ["xmoq", "demo"];
const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

const chunk = { timestamp: 0, data: new Uint8Array([1, 2, 3]) };

/** A row: what fails, the publisher's counts after one chunk, and what was done to the subscription. */
interface Case {
	name: string;
	fail: "none" | "write" | "close";
	stats: { written: number; skipped: Record<string, number> };
	log: string[];
}

describe("createAudioTrackPublisher: what it counts", () => {
	test.each<Case>([
		{
			name: "a chunk whose write and close go through",
			fail: "none",
			stats: { written: 1, skipped: {} },
			log: [
				"group 5 Zero: opened",
				"group 5 Zero: object 0 3 bytes at 1000000",
				"group 5 Zero: closed",
			],
		},
		{
			name: "a chunk whose write fails is left out, and not written",
			fail: "write",
			stats: { written: 0, skipped: { "write-failed": 1 } },
			log: ["group 5 Zero: opened", "group 5 Zero: reset"],
		},
		{
			name: "a chunk whose stream's close fails was written all the same, and its stream is reset",
			fail: "close",
			stats: { written: 1, skipped: {} },
			log: [
				"group 5 Zero: opened",
				"group 5 Zero: object 0 3 bytes at 1000000",
				"group 5 Zero: close failed",
				"group 5 Zero: reset",
			],
		},
	])("$name", async ({ fail, stats, log }) => {
		const publisher = createAudioTrackPublisher({
			trackNamespace: NAMESPACE.map(utf8),
			trackName: utf8("audio"),
			firstGroup: 5n,
			epoch: 1000000n,
		});
		const { request, accepted } = fakeSubscribe(NAMESPACE, "audio");
		expect(publisher.subscribe(request)).toBe(true);
		await settle();
		if (fail === "write")
			accepted.failNextWrite(new SubgroupClosedError("stopped"));
		if (fail === "close")
			accepted.failNextClose(new SubgroupClosedError("stopped"));
		publisher.push(chunk);
		await settle();
		expect(publisher.stats()).toEqual(stats);
		expect(accepted.log).toEqual(log);
	});
});

/** What is done to the publisher, in order, with a turn of the event loop after each. */
type Step =
	| "push"
	| "subscribe"
	/** A SUBSCRIBE with FORWARD 0. */
	| "subscribe-forward-0"
	/** A SUBSCRIBE whose SUBSCRIBE_OK is out 5 ms after it was sent. */
	| "subscribe-answered-later"
	/** Every write from here on waits for ever. */
	| "hold-writes"
	| "fetch"
	| "end";

describe("createAudioTrackPublisher: how it answers, as the video publisher does", () => {
	test.each<{
		name: string;
		steps: Step[];
		answers: string[];
		log: string[];
	}>([
		{
			name: "a subscription with FORWARD 0 is sent nothing, and gets PUBLISH_DONE",
			steps: ["subscribe-forward-0", "push", "end"],
			answers: ["SUBSCRIBE_OK largest none"],
			log: ["PUBLISH_DONE 2"],
		},
		{
			name: "the SUBSCRIBE_OK says where the last chunk pushed is",
			steps: ["push", "push", "subscribe"],
			answers: ["SUBSCRIBE_OK largest {6, 0}"],
			log: [],
		},
		{
			name: "a FETCH is refused with NOT_SUPPORTED, and with DOES_NOT_EXIST after the end",
			steps: ["fetch", "end", "fetch"],
			answers: ["REQUEST_ERROR 3", "REQUEST_ERROR 16"],
			log: [],
		},
		{
			name: "a write that does not end in time is given up, and PUBLISH_DONE is sent",
			steps: ["subscribe", "hold-writes", "push", "push", "end"],
			answers: ["SUBSCRIBE_OK largest none"],
			log: ["group 5 Zero: opened", "group 5 Zero: reset", "PUBLISH_DONE 2"],
		},
		{
			name: "a SUBSCRIBE being answered at the end gets PUBLISH_DONE once it is answered",
			steps: ["subscribe-answered-later", "end"],
			answers: ["SUBSCRIBE_OK largest none"],
			log: ["PUBLISH_DONE 2"],
		},
	])("$name", async ({ steps, answers, log }) => {
		const publisher = createAudioTrackPublisher({
			trackNamespace: NAMESPACE.map(utf8),
			trackName: utf8("audio"),
			firstGroup: 5n,
			epoch: 1000000n,
			endTimeoutMs: 10,
		});
		const seen: string[] = [];
		const subscribed = (
			parameters: { forward: boolean } | undefined,
			acceptWaits: Promise<void> | undefined,
		) =>
			fakeSubscribe(NAMESPACE, "audio", {
				...(parameters === undefined ? {} : { parameters }),
				...(acceptWaits === undefined ? {} : { acceptWaits }),
			});
		let subscription: ReturnType<typeof fakeSubscribe> | undefined;
		for (const step of steps) {
			switch (step) {
				case "push":
					publisher.push(chunk);
					break;
				case "subscribe":
				case "subscribe-forward-0":
				case "subscribe-answered-later":
					subscription = subscribed(
						step === "subscribe-forward-0" ? { forward: false } : undefined,
						step === "subscribe-answered-later"
							? new Promise((resolve) => setTimeout(resolve, 5))
							: undefined,
					);
					publisher.subscribe(subscription.request);
					break;
				case "hold-writes":
					subscription?.accepted.holdWrites(new Promise(() => {}));
					break;
				case "fetch": {
					const fetch = fakeFetch(NAMESPACE, "audio");
					publisher.fetch(fetch.request);
					await settle();
					seen.push(...fetch.answers);
					break;
				}
				case "end":
					await publisher.end();
					break;
			}
			await settle();
		}
		expect({
			answers: [...(subscription?.answers ?? []), ...seen],
			log: subscription?.accepted.log ?? [],
		}).toEqual({ answers, log });
	});
});
