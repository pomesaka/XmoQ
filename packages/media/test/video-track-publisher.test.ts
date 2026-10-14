import { describe, expect, spyOn, test } from "bun:test";
import {
	type MessageParameters,
	PublishEndedError,
	SessionClosedError,
	SubgroupClosedError,
} from "@xmoq/moqt";
import {
	createVideoTrackPublisher,
	type VideoChunk,
	type VideoStreamMapping,
	type VideoTrackPublisherOptions,
} from "../src/index.ts";
import { fakeFetch, fakeSubscribe } from "./fake-track.ts";

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
const NAMESPACE = ["xmoq", "demo"];
const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

/** "k" a key chunk, "d" a delta chunk, each 6 bytes at the encoder's time 0; "D" a delta chunk of 9 bytes, over the limit of these tests. */
const chunkOf = (token: string): VideoChunk => ({
	type: token === "k" ? "key" : "delta",
	timestamp: 0,
	data: new Uint8Array(
		token === "D"
			? [0, 0, 0, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]
			: [0, 0, 0, 1, 0xaa, 0xbb],
	),
});

const publisherOf = (
	mapping: VideoStreamMapping,
	more: Partial<VideoTrackPublisherOptions> = {},
) =>
	createVideoTrackPublisher({
		trackNamespace: NAMESPACE.map(utf8),
		trackName: utf8("video"),
		firstGroup: 5n,
		epoch: 1000000n,
		mapping,
		maxPayloadLength: 8,
		endTimeoutMs: 20,
		...more,
	});

/** What each chunk gives: "" for one that is sent, the reason for one that is not, and "+key" where a key chunk is asked for. */
const pushed = (
	publisher: ReturnType<typeof publisherOf>,
	tokens: string[],
): string[] =>
	tokens.map((token) => {
		const result = publisher.push(chunkOf(token));
		return `${result.skipped ?? ""}${result.askForKeyFrame ? "+key" : ""}`;
	});

/** Runs `act` and gives what was reported with `reportError` meanwhile. */
async function reported(act: () => Promise<void>): Promise<unknown[]> {
	const errors: unknown[] = [];
	const original = globalThis.reportError;
	globalThis.reportError = (error: unknown) => {
		errors.push(error);
	};
	try {
		await act();
	} finally {
		globalThis.reportError = original;
	}
	return errors;
}

/** A publisher with its subscription accepted. */
async function subscribed(
	mapping: VideoStreamMapping,
	more: Partial<VideoTrackPublisherOptions> = {},
	parameters: MessageParameters = {},
) {
	const publisher = publisherOf(mapping, more);
	const asked = fakeSubscribe(NAMESPACE, "video", { parameters });
	publisher.subscribe(asked.request);
	await settle();
	return { publisher, track: asked.accepted };
}

/**
 * Runs `act` and says whether the last timer it set was cleared. WHY the
 * last: one that fired before needs no clearing, and the one that is set
 * last is the one that can be left behind.
 */
async function lastTimerCleared(act: () => Promise<void>): Promise<boolean> {
	const set = spyOn(globalThis, "setTimeout");
	const clear = spyOn(globalThis, "clearTimeout");
	try {
		await act();
		const last: unknown = set.mock.results.at(-1)?.value;
		return clear.mock.calls.some(([timer]) => timer === last);
	} finally {
		set.mockRestore();
		clear.mockRestore();
	}
}

describe("createVideoTrackPublisher: the requests it takes", () => {
	test("a SUBSCRIBE for its track is accepted; before anything was sent, without a Largest Object", async () => {
		const publisher = publisherOf("stream-per-group");
		const asked = fakeSubscribe(NAMESPACE, "video");
		expect(publisher.subscribe(asked.request)).toBe(true);
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest none"]);
	});

	test("the Largest Object is the last Object the encoder's chunks made, also when nobody was subscribed", async () => {
		const publisher = publisherOf("stream-per-group");
		expect(pushed(publisher, ["k", "d"])).toEqual([
			"no-subscriber",
			"no-subscriber",
		]);
		const asked = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(asked.request);
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {5, 1}"]);
	});

	test.each<{ name: string; namespace: string[]; trackName: string }>([
		{ name: "another track", namespace: NAMESPACE, trackName: "audio" },
		{
			name: "a track whose name is the start of this one's",
			namespace: NAMESPACE,
			trackName: "vide",
		},
		{
			name: "the track of another namespace",
			namespace: ["xmoq", "other"],
			trackName: "video",
		},
		{
			name: "the track of a longer namespace",
			namespace: ["xmoq", "demo", "x"],
			trackName: "video",
		},
	])(
		"$name is not this publisher's: neither a SUBSCRIBE nor a FETCH is answered",
		({ namespace, trackName }) => {
			const publisher = publisherOf("stream-per-group");
			const subscribe = fakeSubscribe(namespace, trackName);
			const fetch = fakeFetch(namespace, trackName);
			expect(publisher.subscribe(subscribe.request)).toBe(false);
			expect(publisher.fetch(fetch.request)).toBe(false);
			expect([...subscribe.answers, ...fetch.answers]).toEqual([]);
		},
	);

	test("a second SUBSCRIBE while one is open, or is being answered, is refused with DUPLICATE_SUBSCRIPTION; after the first ended, one is accepted", async () => {
		const publisher = publisherOf("stream-per-group");
		const first = fakeSubscribe(NAMESPACE, "video");
		const atOnce = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(first.request);
		expect(publisher.subscribe(atOnce.request)).toBe(true);
		await settle();
		const later = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(later.request);
		await settle();
		expect([atOnce.answers, later.answers]).toEqual([
			["REQUEST_ERROR 25"],
			["REQUEST_ERROR 25"],
		]);
		first.accepted.end({ type: "peer-reset", code: 1 });
		await settle();
		const again = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(again.request);
		await settle();
		expect(again.answers).toEqual(["SUBSCRIBE_OK largest none"]);
	});

	test("a FETCH for the track is refused with NOT_SUPPORTED: no frame is kept", async () => {
		const publisher = publisherOf("stream-per-group");
		const asked = fakeFetch(NAMESPACE, "video");
		expect(publisher.fetch(asked.request)).toBe(true);
		await settle();
		expect(asked.answers).toEqual(["REQUEST_ERROR 3"]);
	});

	test("a SUBSCRIBE whose answer could not be sent is refused with INTERNAL_ERROR, the failure reported, and the place is free again", async () => {
		const publisher = publisherOf("stream-per-group");
		const failure = new RangeError("not an answer");
		const asked = fakeSubscribe(NAMESPACE, "video", { acceptFails: failure });
		const errors = await reported(async () => {
			publisher.subscribe(asked.request);
			await settle();
		});
		expect(asked.answers).toEqual(["REQUEST_ERROR 0"]);
		expect(errors).toEqual([failure]);
		const again = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(again.request);
		await settle();
		expect(again.answers).toEqual(["SUBSCRIBE_OK largest none"]);
	});
});

describe("createVideoTrackPublisher: what is written to the subscription", () => {
	test("a stream for every Group: Subgroup 0, the Objects one after the other with the Timestamp of LOC, finished when the next Group begins", async () => {
		const { publisher, track } = await subscribed("stream-per-group");
		expect(pushed(publisher, ["k", "d", "d", "k"])).toEqual(["", "", "", ""]);
		await settle();
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: object 0 6 bytes at 1000000",
			"group 5 Zero: object 1 6 bytes at 1000000",
			"group 5 Zero: object 2 6 bytes at 1000000",
			"group 5 Zero: closed",
			"group 6 Zero: opened",
			"group 6 Zero: object 0 6 bytes at 1000000",
		]);
		expect(publisher.stats()).toEqual({
			written: 4,
			skipped: {},
			writeFailures: 0,
			pendingObjects: 0,
			newGroupsAskedFor: 0,
			newGroupsBegun: 0,
		});
	});

	test("a stream for every Object: the Subgroup is the Object's own, and the stream is finished after it", async () => {
		const { publisher, track } = await subscribed("stream-per-object");
		pushed(publisher, ["k", "d"]);
		await settle();
		expect(track.log).toEqual([
			"group 5 FirstObjectId: opened",
			"group 5 FirstObjectId: object 0 6 bytes at 1000000",
			"group 5 FirstObjectId: closed",
			"group 5 FirstObjectId: opened",
			"group 5 FirstObjectId: object 1 6 bytes at 1000000",
			"group 5 FirstObjectId: closed",
		]);
	});

	test.each<{ mapping: VideoStreamMapping; log: string[] }>([
		{
			mapping: "stream-per-group",
			log: [
				"group 6 Zero: opened",
				"group 6 Zero: object 0 6 bytes at 1000000",
			],
		},
		{
			mapping: "stream-per-object",
			log: [
				"group 6 FirstObjectId: opened",
				"group 6 FirstObjectId: object 0 6 bytes at 1000000",
				"group 6 FirstObjectId: closed",
			],
		},
	])(
		"$mapping: a subscriber that comes in the middle of a Group gets nothing of it, has a key chunk asked for once, and gets the next key chunk first",
		async ({ mapping, log }) => {
			const publisher = publisherOf(mapping);
			pushed(publisher, ["k"]);
			const asked = fakeSubscribe(NAMESPACE, "video");
			publisher.subscribe(asked.request);
			await settle();
			// A key chunk is asked for with the first chunk after it came, and
			// not again.
			expect(pushed(publisher, ["d", "d", "k"])).toEqual([
				"no-subscriber+key",
				"no-subscriber",
				"",
			]);
			await settle();
			expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {5, 0}"]);
			expect(asked.accepted.log).toEqual(log);
		},
	);

	test("a subscriber whose first chunk is a key chunk has none asked for; the next one that comes has", async () => {
		const { publisher, track } = await subscribed("stream-per-group");
		expect(pushed(publisher, ["k", "d"])).toEqual(["", ""]);
		track.end({ type: "peer-reset", code: 1 });
		await settle();
		const again = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(again.request);
		await settle();
		expect(pushed(publisher, ["d", "d"])).toEqual([
			"no-subscriber+key",
			"no-subscriber",
		]);
	});

	test("a Group that is cut while nobody is subscribed has no key chunk asked for; the subscriber that comes has one, once", async () => {
		const publisher = publisherOf("stream-per-group");
		expect(pushed(publisher, ["k", "D", "d"])).toEqual([
			"no-subscriber",
			"too-large",
			"no-key-frame",
		]);
		const asked = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(asked.request);
		await settle();
		expect(pushed(publisher, ["d", "d", "k", "d"])).toEqual([
			"no-key-frame+key",
			"no-key-frame",
			"",
			"",
		]);
	});

	test("with FORWARD 0 nothing is written, and the end of the track is told", async () => {
		const { publisher, track } = await subscribed(
			"stream-per-group",
			{},
			{ forward: false },
		);
		expect(pushed(publisher, ["k", "d"])).toEqual([
			"no-subscriber",
			"no-subscriber",
		]);
		await publisher.end();
		expect(track.log).toEqual(["PUBLISH_DONE 2"]);
	});

	test("a chunk the sender does not send is not written, ends its Group, and a key chunk is asked for once", async () => {
		const { publisher, track } = await subscribed("stream-per-group");
		expect(pushed(publisher, ["k", "D", "d", "d", "k", "d"])).toEqual([
			"",
			"too-large+key",
			"no-key-frame",
			"no-key-frame",
			"",
			"",
		]);
		await settle();
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: object 0 6 bytes at 1000000",
			"group 5 Zero: closed",
			"group 6 Zero: opened",
			"group 6 Zero: object 0 6 bytes at 1000000",
			"group 6 Zero: object 1 6 bytes at 1000000",
		]);
	});

	test("a Group that is cut after a key chunk came has a key chunk asked for again", async () => {
		const { publisher } = await subscribed("stream-per-group");
		expect(pushed(publisher, ["k", "D", "k", "D", "d"])).toEqual([
			"",
			"too-large+key",
			"",
			"too-large+key",
			"no-key-frame",
		]);
	});
});

describe("createVideoTrackPublisher: a write that fails, and a subscriber that is behind", () => {
	test.each<{ name: string; fails: Error }>([
		{
			name: "the subscriber stopped the stream",
			fails: new SubgroupClosedError("stopped", { code: 1 }),
		},
		{ name: "the transport failed", fails: new Error("the transport failed") },
	])(
		"a stream for every Group, $name: the stream is reset, nothing more of the Group is written, a key chunk is asked for once, and the next Group goes on a new stream",
		async ({ fails }) => {
			const { publisher, track } = await subscribed("stream-per-group");
			pushed(publisher, ["k"]);
			await settle();
			track.failNextWrite(fails);
			// The chunk is taken before its write has failed.
			expect(pushed(publisher, ["d"])).toEqual([""]);
			await settle();
			expect(pushed(publisher, ["d", "d", "k", "d"])).toEqual([
				"no-key-frame+key",
				"no-key-frame",
				"",
				"",
			]);
			await settle();
			expect(track.log).toEqual([
				"group 5 Zero: opened",
				"group 5 Zero: object 0 6 bytes at 1000000",
				"group 5 Zero: reset",
				"group 6 Zero: opened",
				"group 6 Zero: object 0 6 bytes at 1000000",
				"group 6 Zero: object 1 6 bytes at 1000000",
			]);
			expect(publisher.stats().writeFailures).toBe(1);
		},
	);

	test("a stream for every Object: the stream of the Object that failed is reset, and what was queued of its Group is not written", async () => {
		const { publisher, track } = await subscribed("stream-per-object");
		track.failNextWrite(new Error("the transport failed"));
		// All three are taken before the first write has failed.
		expect(pushed(publisher, ["k", "d", "d"])).toEqual(["", "", ""]);
		await settle();
		expect(pushed(publisher, ["d", "k"])).toEqual(["no-key-frame+key", ""]);
		await settle();
		expect(track.log).toEqual([
			"group 5 FirstObjectId: opened",
			"group 5 FirstObjectId: reset",
			"group 6 FirstObjectId: opened",
			"group 6 FirstObjectId: object 0 6 bytes at 1000000",
			"group 6 FirstObjectId: closed",
		]);
	});

	test("a stream that could not be opened: its Group is cut short the same way", async () => {
		const { publisher, track } = await subscribed("stream-per-group");
		track.failNextOpen(new Error("no stream"));
		pushed(publisher, ["k", "d"]);
		await settle();
		expect(pushed(publisher, ["d", "k"])).toEqual(["no-key-frame+key", ""]);
		await settle();
		expect(track.log).toEqual([
			"group 6 Zero: opened",
			"group 6 Zero: object 0 6 bytes at 1000000",
		]);
	});

	test.each<{ mapping: VideoStreamMapping; log: string[] }>([
		{
			mapping: "stream-per-group",
			log: [
				"group 5 Zero: opened",
				"group 5 Zero: reset",
				"group 6 Zero: opened",
				"group 6 Zero: object 0 6 bytes at 1000000",
				"group 6 Zero: object 1 6 bytes at 1000000",
				"group 6 Zero: object 2 6 bytes at 1000000",
			],
		},
		{
			mapping: "stream-per-object",
			log: [
				"group 5 FirstObjectId: opened",
				"group 5 FirstObjectId: reset",
				"group 6 FirstObjectId: opened",
				"group 6 FirstObjectId: object 0 6 bytes at 1000000",
				"group 6 FirstObjectId: closed",
				"group 6 FirstObjectId: opened",
				"group 6 FirstObjectId: object 1 6 bytes at 1000000",
				"group 6 FirstObjectId: closed",
				"group 6 FirstObjectId: opened",
				"group 6 FirstObjectId: object 2 6 bytes at 1000000",
				"group 6 FirstObjectId: closed",
			],
		},
	])(
		"$mapping: a failure in a Group that is over leaves the rest of that Group unwritten, and does not cut the Group that came after it",
		async ({ mapping, log }) => {
			const { publisher, track } = await subscribed(mapping);
			track.failNextWrite(new Error("the transport failed"));
			// The key chunk of Group 6 is taken before the write of Group 5 failed.
			expect(pushed(publisher, ["k", "d", "k", "d"])).toEqual(["", "", "", ""]);
			await settle();
			expect(pushed(publisher, ["d"])).toEqual([""]);
			await settle();
			expect(track.log).toEqual(log);
		},
	);

	test("the subscriber ended the subscription: nothing is reset or reported, and the chunks after it have no subscriber", async () => {
		const { publisher, track } = await subscribed("stream-per-group");
		pushed(publisher, ["k"]);
		await settle();
		const errors = await reported(async () => {
			track.end({ type: "peer-reset", code: 1 });
			track.failNextWrite(
				new PublishEndedError({ type: "peer-reset", code: 1 }),
			);
			pushed(publisher, ["d"]);
			await settle();
		});
		expect(pushed(publisher, ["d", "k"])).toEqual([
			"no-subscriber",
			"no-subscriber",
		]);
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: object 0 6 bytes at 1000000",
		]);
		expect(errors).toEqual([]);
		expect(publisher.stats().writeFailures).toBe(0);
	});

	test.each<{ name: string; fails: Error }>([
		{
			name: "the subscription is over",
			fails: new PublishEndedError({ type: "peer-reset", code: 1 }),
		},
		{
			name: "the session is over",
			fails: new SessionClosedError({ type: "peer", code: 0, reason: "" }),
		},
	])(
		"a write that fails because $name is no failure: nothing is reset, nothing is cut, and no key chunk is asked for",
		async ({ fails }) => {
			const { publisher, track } = await subscribed("stream-per-group");
			pushed(publisher, ["k"]);
			await settle();
			// The write says so before `ended` of the subscription does.
			track.failNextWrite(fails);
			pushed(publisher, ["d"]);
			await settle();
			expect(pushed(publisher, ["d"])).toEqual([""]);
			expect(track.log).toEqual([
				"group 5 Zero: opened",
				"group 5 Zero: object 0 6 bytes at 1000000",
			]);
			expect(publisher.stats().writeFailures).toBe(0);
		},
	);

	test("a subscriber that takes nothing: when as many Objects wait as may, the next chunk is not sent and its Group ends; what waited of the Group is not written, the write that was under way is given up, and a key chunk is asked for when there is room again", async () => {
		const { publisher, track } = await subscribed("stream-per-group", {
			maxPendingObjects: 2,
		});
		track.holdWrites(new Promise(() => {}));
		expect(pushed(publisher, ["k", "d"])).toEqual(["", ""]);
		// The write of Object 0 is under way, and stands still.
		await settle();
		expect(pushed(publisher, ["d", "d", "k"])).toEqual([
			"behind",
			"behind",
			"behind",
		]);
		expect(publisher.stats().pendingObjects).toBe(2);
		await settle();
		expect(publisher.stats().pendingObjects).toBe(0);
		expect(pushed(publisher, ["d", "d", "k"])).toEqual([
			"no-key-frame+key",
			"no-key-frame",
			"",
		]);
		await settle();
		// Group 6 was the key chunk that was not sent: its ID was not used.
		// The reset is no failure of a write: it was this publisher's.
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: reset",
			"group 6 Zero: opened",
		]);
		expect(publisher.stats().writeFailures).toBe(0);
	});

	test("a Group that is cut while a write of the Group before it stands still: that write goes on, and nothing of the Group that was cut is written, not its stream either", async () => {
		const { publisher, track } = await subscribed("stream-per-group", {
			maxPendingObjects: 2,
		});
		let release: () => void = () => {};
		track.holdWrites(
			new Promise((resolve) => {
				release = resolve;
			}),
		);
		expect(pushed(publisher, ["k"])).toEqual([""]);
		await settle();
		// Object 0 of Group 5 stands still; Object 0 of Group 6 waits; the
		// next chunk of Group 6 is one too many.
		expect(pushed(publisher, ["k", "d"])).toEqual(["", "behind"]);
		release();
		await settle();
		expect(pushed(publisher, ["d", "k"])).toEqual(["no-key-frame+key", ""]);
		await settle();
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: object 0 6 bytes at 1000000",
			"group 5 Zero: closed",
			"group 7 Zero: opened",
			"group 7 Zero: object 0 6 bytes at 1000000",
		]);
	});

	test("a Group that is cut while its stream is being opened: the stream is reset when it has come, nothing is written to it, and the next Group goes on a new one", async () => {
		const { publisher, track } = await subscribed("stream-per-group", {
			maxPendingObjects: 2,
		});
		let release: () => void = () => {};
		track.holdOpens(
			new Promise((resolve) => {
				release = resolve;
			}),
		);
		expect(pushed(publisher, ["k", "d"])).toEqual(["", ""]);
		// The stream of Group 5 is being opened, and no write is under way.
		await settle();
		expect(pushed(publisher, ["d"])).toEqual(["behind"]);
		release();
		await settle();
		expect(pushed(publisher, ["d", "k"])).toEqual(["no-key-frame+key", ""]);
		await settle();
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: reset",
			"group 6 Zero: opened",
			"group 6 Zero: object 0 6 bytes at 1000000",
		]);
		expect(publisher.stats().writeFailures).toBe(0);
	});

	test("the bytes that wait have a limit as well, and are counted no more once they are written", async () => {
		const { publisher, track } = await subscribed("stream-per-group", {
			maxPendingBytes: 12,
		});
		// Two chunks of 6 bytes are what may wait.
		expect(pushed(publisher, ["k", "d"])).toEqual(["", ""]);
		await settle();
		expect(pushed(publisher, ["d", "d"])).toEqual(["", ""]);
		await settle();
		track.holdWrites(new Promise(() => {}));
		expect(pushed(publisher, ["d", "d", "d"])).toEqual(["", "", "behind"]);
	});
});

describe("createVideoTrackPublisher: the end of the track", () => {
	test("what is being written is finished, the stream of the Group is finished, and PUBLISH_DONE follows; nothing is sent after it", async () => {
		const publisher = publisherOf("stream-per-group");
		const asked = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(asked.request);
		await settle();
		pushed(publisher, ["k", "d"]);
		await publisher.end();
		expect(asked.accepted.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: object 0 6 bytes at 1000000",
			"group 5 Zero: object 1 6 bytes at 1000000",
			"group 5 Zero: closed",
			"PUBLISH_DONE 2",
		]);
		// Not looked at by the sender either: no key chunk is asked for.
		expect(pushed(publisher, ["k", "D"])).toEqual([
			"no-subscriber",
			"no-subscriber",
		]);
		const again = fakeSubscribe(NAMESPACE, "video");
		const fetch = fakeFetch(NAMESPACE, "video");
		publisher.subscribe(again.request);
		publisher.fetch(fetch.request);
		await settle();
		expect([again.answers, fetch.answers]).toEqual([
			["REQUEST_ERROR 16"],
			["REQUEST_ERROR 16"],
		]);
	});

	test.each<{ mapping: VideoStreamMapping; log: string[] }>([
		{
			mapping: "stream-per-group",
			log: ["group 5 Zero: opened", "group 5 Zero: reset", "PUBLISH_DONE 2"],
		},
		{
			mapping: "stream-per-object",
			log: [
				"group 5 FirstObjectId: opened",
				"group 5 FirstObjectId: reset",
				"PUBLISH_DONE 2",
			],
		},
	])(
		"$mapping: a subscriber that does not take what is written has its stream reset when the time is over, and gets PUBLISH_DONE",
		async ({ mapping, log }) => {
			const publisher = publisherOf(mapping);
			const asked = fakeSubscribe(NAMESPACE, "video");
			publisher.subscribe(asked.request);
			await settle();
			asked.accepted.holdWrites(new Promise(() => {}));
			pushed(publisher, ["k"]);
			const errors = await reported(() => publisher.end());
			expect(asked.accepted.log).toEqual(log);
			expect(errors).toEqual([]);
		},
	);

	// What stands in the lines when the time of `end` is over, whatever the
	// order in which the reset settles and the write under way fails.
	const LATE: Record<VideoStreamMapping, string[]> = {
		"stream-per-group": [
			"group 5 Zero: opened",
			"group 5 Zero: reset",
			"PUBLISH_DONE 2",
		],
		"stream-per-object": [
			"group 5 FirstObjectId: opened",
			"group 5 FirstObjectId: reset",
			"PUBLISH_DONE 2",
		],
	};
	const MAPPINGS: VideoStreamMapping[] = [
		"stream-per-group",
		"stream-per-object",
	];
	test.each(
		MAPPINGS.flatMap((mapping) =>
			[0, 1, 2, 3, 5, 8, 16, 24, 32, 64].flatMap((reset) =>
				[0, 1, 3, 8, 32].map((rejection) => ({ mapping, reset, rejection })),
			),
		),
	)(
		"$mapping, the time is over while a write stands still and more is queued behind it, a Group that opens a stream among it; the reset settles after $reset turns and the write fails after $rejection: nothing that was queued is begun, and PUBLISH_DONE is sent",
		async ({ mapping, reset, rejection }) => {
			const { publisher, track } = await subscribed(mapping, {
				endTimeoutMs: 5,
			});
			track.holdWrites(new Promise(() => {}));
			track.delay({ reset, rejection });
			expect(pushed(publisher, ["k", "d", "k", "d"])).toEqual(["", "", "", ""]);
			const errors = await reported(() => publisher.end());
			// What fails late has failed before the lines are read.
			await settle();
			expect(track.log).toEqual(LATE[mapping]);
			expect(errors).toEqual([]);
		},
	);

	test("the time is over while a stream is being opened: the stream is reset when it has come, and PUBLISH_DONE is sent after that", async () => {
		const { publisher, track } = await subscribed("stream-per-group", {
			endTimeoutMs: 20,
		});
		let release: () => void = () => {};
		track.holdOpens(
			new Promise((resolve) => {
				release = resolve;
			}),
		);
		pushed(publisher, ["k"]);
		// After the first time is over, and before the second.
		setTimeout(release, 30);
		const errors = await reported(() => publisher.end());
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"group 5 Zero: reset",
			"PUBLISH_DONE 2",
		]);
		expect(errors).toEqual([]);
	});

	test("a stream that is never opened: `end` resolves all the same, PUBLISH_DONE cannot be sent, and that is reported", async () => {
		const { publisher, track } = await subscribed("stream-per-group", {
			endTimeoutMs: 5,
		});
		track.holdOpens(new Promise(() => {}));
		pushed(publisher, ["k"]);
		const errors = await reported(() => publisher.end());
		expect(track.log).toEqual([
			"group 5 Zero: opened",
			"PUBLISH_DONE refused: a subgroup stream is open",
		]);
		expect(errors.map(String)).toEqual([
			"Error: a subgroup stream is neither closed nor reset",
		]);
	});

	test("a SUBSCRIBE that is being answered when the track ends gets PUBLISH_DONE once it is answered, and `end` waits for that", async () => {
		const publisher = publisherOf("stream-per-group");
		const asked = fakeSubscribe(NAMESPACE, "video", {
			acceptWaits: new Promise((resolve) => setTimeout(resolve, 5)),
		});
		publisher.subscribe(asked.request);
		await publisher.end();
		expect([asked.answers, asked.accepted.log]).toEqual([
			["SUBSCRIBE_OK largest none"],
			["PUBLISH_DONE 2"],
		]);
	});

	test.each<{ name: string; stuck: boolean }>([
		{ name: "everything was written", stuck: false },
		{ name: "the time was over", stuck: true },
	])("`end` leaves no timer behind where $name", async ({ stuck }) => {
		const { publisher, track } = await subscribed("stream-per-group", {
			endTimeoutMs: 5,
		});
		if (stuck) track.holdWrites(new Promise(() => {}));
		pushed(publisher, ["k"]);
		expect(await lastTimerCleared(() => publisher.end())).toBe(true);
	});

	test("without a subscription, `end` has nothing to tell; twice is once", async () => {
		const publisher = publisherOf("stream-per-group");
		await Promise.all([publisher.end(), publisher.end()]);
		expect(pushed(publisher, ["k"])).toEqual(["no-subscriber"]);
	});
});

describe("createVideoTrackPublisher: options", () => {
	test.each<{ name: string; more: Partial<VideoTrackPublisherOptions> }>([
		{ name: "no Object may wait", more: { maxPendingObjects: 0 } },
		{ name: "a fraction of an Object", more: { maxPendingObjects: 1.5 } },
		{ name: "no byte may wait", more: { maxPendingBytes: 0 } },
		{ name: "a time that is not a number", more: { endTimeoutMs: Number.NaN } },
		{ name: "an epoch the sender refuses", more: { epoch: -1n } },
		{
			name: "fewer bytes may wait than the longest payload has",
			more: { maxPayloadLength: 8, maxPendingBytes: 7 },
		},
	])("$name is a RangeError", ({ more }) => {
		expect(() => publisherOf("stream-per-group", more)).toThrow(RangeError);
	});
});

describe("createVideoTrackPublisher: a subscriber that asks for a new Group", () => {
	/** "k300" a key chunk and "d300" a delta chunk at the encoder's time 300 ms; "ask6" a NEW_GROUP_REQUEST of 6. */
	const step = (
		publisher: ReturnType<typeof publisherOf>,
		track: Awaited<ReturnType<typeof subscribed>>["track"],
		token: string,
	): string => {
		const value = Number(token.replace(/^[a-z]+/, ""));
		if (token.startsWith("ask")) {
			track.askForNewGroup(BigInt(value));
			return "";
		}
		const result = publisher.push({
			...chunkOf(token[0] ?? ""),
			timestamp: value * 1000,
		});
		return `${result.skipped ?? ""}${result.askForKeyFrame ? "+key" : ""}`;
	};

	test.each<{
		name: string;
		dynamicGroups: VideoTrackPublisherOptions["dynamicGroups"];
		steps: string[];
		expected: string[];
		asked: number;
		begun: number;
	}>([
		{
			name: "0 asks for a key chunk once the Group is 500 ms long",
			dynamicGroups: {},
			steps: ["k0", "d100", "ask0", "d200", "d400", "d500", "d600"],
			expected: ["", "", "", "", "", "+key", ""],
			asked: 1,
			begun: 1,
		},
		{
			name: "with a shortest Group of 0, at the next chunk",
			dynamicGroups: { shortestGroupMs: 0 },
			steps: ["k0", "d100", "ask0", "d200"],
			expected: ["", "", "", "+key"],
			asked: 1,
			begun: 1,
		},
		{
			name: "a value that is the Group being sent asks for nothing",
			dynamicGroups: {},
			steps: ["k0", "ask5", "d600"],
			expected: ["", "", ""],
			asked: 0,
			begun: 0,
		},
		{
			name: "a value larger than the Group being sent asks",
			dynamicGroups: {},
			steps: ["k0", "ask6", "d600"],
			expected: ["", "", "+key"],
			asked: 1,
			begun: 1,
		},
		{
			name: "a key chunk that comes first meets the request",
			dynamicGroups: {},
			steps: ["k0", "ask0", "d100", "k200", "d700"],
			expected: ["", "", "", "", ""],
			asked: 1,
			begun: 0,
		},
		{
			name: "a request before the first chunk is met by it",
			dynamicGroups: {},
			steps: ["ask0", "k0", "d600"],
			expected: ["", "", ""],
			asked: 1,
			begun: 0,
		},
		{
			name: "without dynamic Groups a request does nothing",
			dynamicGroups: undefined,
			steps: ["k0", "ask0", "d600"],
			expected: ["", "", ""],
			asked: 0,
			begun: 0,
		},
	])("$name", async ({ dynamicGroups, steps, expected, asked, begun }) => {
		const { publisher, track } = await subscribed(
			"stream-per-group",
			dynamicGroups === undefined ? {} : { dynamicGroups },
		);
		expect(steps.map((token) => step(publisher, track, token))).toEqual(
			expected,
		);
		const { newGroupsAskedFor, newGroupsBegun } = publisher.stats();
		expect([newGroupsAskedFor, newGroupsBegun]).toEqual([asked, begun]);
	});

	test.each<{
		name: string;
		dynamicGroups: VideoTrackPublisherOptions["dynamicGroups"];
		answer: string;
	}>([
		{
			name: "with dynamic Groups the SUBSCRIBE_OK has DYNAMIC_GROUPS = 1",
			dynamicGroups: {},
			answer: "SUBSCRIBE_OK largest none property 48=1",
		},
		{
			name: "without, no Track Property",
			dynamicGroups: undefined,
			answer: "SUBSCRIBE_OK largest none",
		},
	])("$name", async ({ dynamicGroups, answer }) => {
		const publisher = publisherOf(
			"stream-per-group",
			dynamicGroups === undefined ? {} : { dynamicGroups },
		);
		const subscribe = fakeSubscribe(NAMESPACE, "video");
		publisher.subscribe(subscribe.request);
		await settle();
		expect(subscribe.answers).toEqual([answer]);
	});
});
