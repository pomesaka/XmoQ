import { describe, expect, test } from "bun:test";
import { SubgroupClosedError } from "@xmoq/moqt";
import { createAudioTrackPublisher } from "../src/index.ts";
import { fakeSubscribe } from "./fake-track.ts";

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
