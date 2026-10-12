import { describe, expect, jest, test } from "bun:test";
import type {
	FetchWriter,
	GroupOrder,
	Location,
	MessageParameters,
	PeerFetch,
	PeerSubscribe,
	PublishEnd,
	PublishedTrack,
	SubgroupWriter,
} from "@xmoq/moqt";
import {
	PublishEndedError,
	ServedFetchEndedError,
	SessionClosedError,
	SubgroupClosedError,
} from "@xmoq/moqt";
import { type Catalog, readCatalog } from "../src/core/catalog.ts";
import { createCatalogPublisher } from "../src/moqt/publish.ts";

// The requests and the tracks are made by hand and note what is done to
// them; the run in a browser, through a relay, is in interop/.

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
const NAMESPACE = [utf8("xmoq"), utf8("demo")];
const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

const track = (name: string) => ({
	name,
	packaging: "loc",
	isLive: true,
	codec: "avc3.42E01F",
	bitrate: 1000000,
});
const catalogOf = (...names: string[]): Catalog => ({
	version: "draft-01",
	tracks: names.map(track),
});
/** The names of the tracks of the catalog a payload is. */
function names(payload: Uint8Array): string {
	const read = readCatalog(payload);
	if (read.type !== "catalog") throw new Error(read.type);
	return read.catalog.tracks.map((each) => each.name).join(",");
}

/** A subscription of the peer that was accepted, and what was done on it, in lines. */
interface FakeTrack {
	track: PublishedTrack;
	log: string[];
	/** Ends the subscription as its subscriber would. */
	end(how: PublishEnd): void;
	/** With a promise: every write waits for it. */
	holdWrites(until: Promise<void>): void;
	/** The next write fails with this. */
	failNextWrite(error: Error): void;
	/** With a promise: every subgroup stream opens when it resolves. */
	holdOpens(until: Promise<void>): void;
	/** With a promise: `done` resolves when it does, after PUBLISH_DONE was sent. */
	holdDone(until: Promise<void>): void;
	/** Called each time a subgroup stream was closed, within `close`. */
	onClose(callback: () => void): void;
}

/**
 * Refuses what `PublishedTrack` of @xmoq/moqt refuses: a subgroup on a
 * subscription that is over (`PublishEndedError`), a write to a stream that
 * was reset (`SubgroupClosedError`), and PUBLISH_DONE while a subgroup
 * stream is neither closed nor reset (an `Error`, and nothing is sent) or
 * on a subscription that is over, and a write on a subscription that is
 * over. `done` can be made to take its time, as the real one does while it
 * waits for the subscriber to end the request stream.
 */
function fakeTrack(): FakeTrack {
	const log: string[] = [];
	let end: (how: PublishEnd) => void = () => {};
	const ended = new Promise<PublishEnd>((resolve) => {
		end = resolve;
	});
	let held: Promise<void> = Promise.resolve();
	let failure: Error | undefined;
	let over: PublishEnd | undefined;
	let open = 0;
	let heldOpen: Promise<void> = Promise.resolve();
	let heldDone: Promise<void> = Promise.resolve();
	let closed: () => void = () => {};
	const track: PublishedTrack = {
		requestId: 1n,
		trackAlias: 0n,
		parameters: {},
		ended,
		// No test asks this fake for a new Group.
		onNewGroupRequest: () => {},
		openSubgroup: async (options) => {
			if (over !== undefined) throw new PublishEndedError(over);
			await heldOpen;
			if (over !== undefined) throw new PublishEndedError(over);
			const subgroup =
				options.subgroupId.mode === "Zero" ? "0" : options.subgroupId.mode;
			const stream = `group ${options.groupId} subgroup ${subgroup}${options.endOfGroup ? " end-of-group" : ""}${options.properties ? " with-properties" : ""}`;
			open += 1;
			let state: "open" | "closed" | "reset" = "open";
			let stopped: () => void = () => {};
			const reset = new Promise<void>((resolve) => {
				stopped = resolve;
			});
			const isOpen = (): boolean => state === "open";
			const writer: SubgroupWriter = {
				write: async (object) => {
					if (!isOpen()) throw new SubgroupClosedError("not open");
					if (over !== undefined) throw new PublishEndedError(over);
					if (failure !== undefined) {
						const error = failure;
						failure = undefined;
						throw error;
					}
					await Promise.race([held, reset]);
					if (!isOpen()) throw new SubgroupClosedError("reset");
					log.push(
						`${stream}: object ${object.objectId} ${names(object.payload)}`,
					);
				},
				close: async () => {
					if (!isOpen()) return;
					state = "closed";
					open -= 1;
					log.push(`${stream}: closed`);
					closed();
				},
				reset: async () => {
					if (!isOpen()) return;
					state = "reset";
					open -= 1;
					stopped();
					log.push(`${stream}: reset`);
				},
			};
			return writer;
		},
		done: async (statusCode) => {
			if (over !== undefined) {
				log.push("PUBLISH_DONE refused: the subscription is over");
				throw new PublishEndedError(over);
			}
			if (open > 0) {
				log.push("PUBLISH_DONE refused: a subgroup stream is open");
				throw new Error("a subgroup stream is neither closed nor reset");
			}
			log.push(`PUBLISH_DONE ${statusCode}`);
			over = { type: "done" };
			end(over);
			await heldDone;
			return { type: "fin" };
		},
	};
	return {
		track,
		log,
		end: (how) => {
			over = how;
			end(how);
		},
		holdWrites: (until) => {
			held = until;
		},
		failNextWrite: (error) => {
			failure = error;
		},
		holdOpens: (until) => {
			heldOpen = until;
		},
		holdDone: (until) => {
			heldDone = until;
		},
		onClose: (callback) => {
			closed = callback;
		},
	};
}

/** A SUBSCRIBE of the peer, and how it was answered. */
function fakeSubscribe(
	trackName: string,
	options: {
		namespace?: Uint8Array[];
		parameters?: MessageParameters;
		/** `accept` rejects with this. */
		acceptFails?: Error;
		/** The request went: its signal has aborted. */
		gone?: boolean;
		/** `accept` resolves when this does. */
		acceptAfter?: Promise<void>;
	} = {},
): { request: PeerSubscribe; answers: string[]; accepted: FakeTrack } {
	const answers: string[] = [];
	const accepted = fakeTrack();
	const request: PeerSubscribe = {
		requestId: 1n,
		trackNamespace: options.namespace ?? NAMESPACE,
		trackName: utf8(trackName),
		parameters: options.parameters ?? {},
		signal:
			options.gone === true
				? AbortSignal.abort()
				: new AbortController().signal,
		accept: async (answer) => {
			if (options.acceptFails !== undefined) throw options.acceptFails;
			await options.acceptAfter;
			const largest = answer?.parameters?.largestObject;
			answers.push(
				`SUBSCRIBE_OK largest ${largest === undefined ? "none" : `{${largest.group}, ${largest.object}}`}`,
			);
			return accepted.track;
		},
		reject: async (code) => {
			answers.push(`REQUEST_ERROR ${code}`);
		},
	};
	return { request, answers, accepted };
}

/** A FETCH of the peer, how it was answered, and what was written to its stream. */
function fakeFetch(
	start: [bigint, bigint],
	end: [bigint, bigint],
	options: {
		trackName?: string;
		groupOrder?: GroupOrder;
		/** `write` rejects with this. */
		writeFails?: Error;
		/** `accept` rejects with this. */
		acceptFails?: Error;
		/** The request went: its signal has aborted. */
		gone?: boolean;
	} = {},
): { request: PeerFetch; log: string[] } {
	const log: string[] = [];
	const at = ([group, object]: [bigint, bigint]): Location => ({
		group,
		object,
	});
	const writer: FetchWriter = {
		requestId: 3n,
		ended: new Promise(() => {}),
		write: async (object) => {
			if (options.writeFails !== undefined) throw options.writeFails;
			log.push(
				`object {${object.groupId}, ${object.objectId}} subgroup ${object.subgroupId} ${names(object.payload)}`,
			);
		},
		endOfRange: async (kind, location) => {
			log.push(`${kind} through {${location.group}, ${location.object}}`);
		},
		close: async () => {
			log.push("closed");
		},
		reset: async () => {
			log.push("reset");
		},
	};
	const request: PeerFetch = {
		requestId: 3n,
		trackNamespace: NAMESPACE,
		trackName: utf8(options.trackName ?? "catalog"),
		startLocation: at(start),
		endLocation: at(end),
		groupOrder: options.groupOrder ?? "Ascending",
		fillTimeout: undefined,
		parameters: {},
		signal:
			options.gone === true
				? AbortSignal.abort()
				: new AbortController().signal,
		accept: async (answer) => {
			if (options.acceptFails !== undefined) throw options.acceptFails;
			log.push(
				`FETCH_OK end {${answer.endLocation.group}, ${answer.endLocation.object}}`,
			);
			return writer;
		},
		reject: async (code) => {
			log.push(`REQUEST_ERROR ${code}`);
		},
	};
	return { request, log };
}

const publisherOf = (...first: string[]) =>
	createCatalogPublisher({
		namespace: NAMESPACE,
		catalog: catalogOf(...first),
		firstGroup: 100n,
		endTimeoutMs: 20,
	});

/**
 * `end()` of the publisher, with a time: a fault that keeps it from
 * resolving fails the test where it would stand still. For the real clock.
 */
async function ended(publisher: { end(): Promise<void> }): Promise<void> {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const late = new Promise<never>((_, reject) => {
		timer = setTimeout(
			() => reject(new Error("`end` has not resolved after 2 seconds")),
			2000,
		);
	});
	try {
		await Promise.race([publisher.end(), late]);
	} finally {
		clearTimeout(timer);
	}
}

/** Lets what is ready run, without the clock. */
async function turns(): Promise<void> {
	for (let turn = 0; turn < 50; turn++) await Promise.resolve();
}

/**
 * Under a clock that is moved by hand: calls `end()`, moves the clock by
 * each of `steps` milliseconds in turn, and says whether `end()` has
 * resolved by then. Never waits for it.
 */
async function endedWithin(
	publisher: { end(): Promise<void> },
	steps: number[],
): Promise<boolean> {
	let resolved = false;
	void publisher.end().then(() => {
		resolved = true;
	});
	await turns();
	for (const step of steps) {
		jest.advanceTimersByTime(step);
		await turns();
	}
	return resolved;
}

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

describe("createCatalogPublisher", () => {
	test("has its first catalog at Object 0 of the first Group", () => {
		const publisher = publisherOf("a");
		expect(publisher.current.at).toEqual({ group: 100n, object: 0n });
		expect(publisher.current.catalog).toEqual(catalogOf("a"));
	});

	test("without a first Group, the Group is the time in milliseconds", () => {
		const before = BigInt(Date.now());
		const publisher = createCatalogPublisher({
			namespace: NAMESPACE,
			catalog: catalogOf("a"),
		});
		const { group } = publisher.current.at;
		expect(group >= before && group <= BigInt(Date.now())).toBe(true);
	});

	test.each<{ name: string; trackName: string; namespace: Uint8Array[] }>([
		{
			name: "another track of the namespace",
			trackName: "video",
			namespace: NAMESPACE,
		},
		{
			name: "the catalog of another namespace",
			trackName: "catalog",
			namespace: [utf8("xmoq"), utf8("other")],
		},
		{
			name: "the catalog of a namespace that is the start of this one",
			trackName: "catalog",
			namespace: [utf8("xmoq")],
		},
		{
			name: "the catalog of a longer namespace",
			trackName: "catalog",
			namespace: [...NAMESPACE, utf8("more")],
		},
		{
			name: "a track whose name is the start of the catalog track's",
			trackName: "cata",
			namespace: NAMESPACE,
		},
		{
			name: "the catalog of a namespace whose field is the start of this one's",
			trackName: "catalog",
			namespace: [utf8("xmoq"), utf8("dem")],
		},
		{
			name: "a track whose name begins with the catalog track's",
			trackName: "catalogs",
			namespace: NAMESPACE,
		},
		{
			name: "the catalog of a namespace whose field begins with this one's",
			trackName: "catalog",
			namespace: [utf8("xmoq"), utf8("demos")],
		},
		{
			name: "a track whose name is spelled another way",
			trackName: "Catalog",
			namespace: NAMESPACE,
		},
	])(
		"a SUBSCRIBE for $name is not taken, and not answered",
		({ trackName, namespace }) => {
			const asked = fakeSubscribe(trackName, { namespace });
			expect(publisherOf("a").subscribe(asked.request)).toBe(false);
			expect(asked.answers).toEqual([]);
		},
	);

	test("a FETCH for another track is not taken, and not answered", () => {
		const asked = fakeFetch([100n, 0n], [100n, 1n], { trackName: "video" });
		expect(publisherOf("a").fetch(asked.request)).toBe(false);
		expect(asked.log).toEqual([]);
	});

	test("a SUBSCRIBE for the catalog track is accepted with the Location of the catalog, and the catalog is not sent on it", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		expect(publisher.subscribe(asked.request)).toBe(true);
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {100, 0}"]);
		expect(asked.accepted.log).toEqual([]);
	});

	test("an update goes to every subscription: Object 0 of the next Group, on a stream of its own that is closed", async () => {
		const publisher = publisherOf("a");
		const first = fakeSubscribe("catalog");
		const second = fakeSubscribe("catalog");
		publisher.subscribe(first.request);
		publisher.subscribe(second.request);
		await settle();
		publisher.update(catalogOf("a", "b"));
		await settle();
		const wanted = [
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
		];
		expect(first.accepted.log).toEqual(wanted);
		expect(second.accepted.log).toEqual(wanted);
		expect(publisher.current.at).toEqual({ group: 101n, object: 0n });
	});

	test("a subscriber that is slow to take a catalog gets the newest next, and not the ones between", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		let release: () => void = () => {};
		asked.accepted.holdWrites(
			new Promise((resolve) => {
				release = resolve;
			}),
		);
		publisher.update(catalogOf("a", "b"));
		await settle();
		publisher.update(catalogOf("a", "b", "c"));
		publisher.update(catalogOf("a", "b", "c", "d"));
		await settle();
		expect(asked.accepted.log).toEqual([]);
		release();
		await settle();
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
			"group 103 subgroup 0 end-of-group: object 0 a,b,c,d",
			"group 103 subgroup 0 end-of-group: closed",
		]);
	});

	test("a catalog that was published while a SUBSCRIBE was being answered is sent on that subscription", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		// Before the answer has resolved.
		publisher.update(catalogOf("a", "b"));
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {100, 0}"]);
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
		]);
	});

	test("a subscription with FORWARD 0 is accepted and sent no catalog, and is told of the end", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog", { parameters: { forward: false } });
		expect(publisher.subscribe(asked.request)).toBe(true);
		await settle();
		publisher.update(catalogOf("a", "b"));
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {100, 0}"]);
		expect(asked.accepted.log).toEqual([]);
		await ended(publisher);
		expect(asked.accepted.log).toEqual(["PUBLISH_DONE 2"]);
	});

	test("a subscription that ended gets nothing more", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		asked.accepted.end({ type: "peer-reset", code: 1 });
		await settle();
		publisher.update(catalogOf("a", "b"));
		await settle();
		expect(asked.accepted.log).toEqual([]);
		// It is not kept: the end of the track is not told to it either.
		await ended(publisher);
		expect(asked.accepted.log).toEqual([]);
	});

	test("an update that may not follow the catalog before is a RangeError, and nothing is published", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		const other: Catalog = {
			version: "draft-01",
			tracks: [{ ...track("a"), codec: "avc3.640028" }],
		};
		expect(() => publisher.update(other)).toThrow(RangeError);
		await settle();
		expect(publisher.current.at).toEqual({ group: 100n, object: 0n });
		expect(asked.accepted.log).toEqual([]);
	});

	test.each<{
		name: string;
		updates: number;
		start: [bigint, bigint];
		end: [bigint, bigint];
		log: string[];
	}>([
		{
			name: "the range a Joining FETCH with offset 0 makes",
			updates: 0,
			start: [100n, 0n],
			end: [100n, 1n],
			log: ["FETCH_OK end {100, 1}", "object {100, 0} subgroup 0 a", "closed"],
		},
		{
			name: "a range that starts after the catalog",
			updates: 0,
			start: [100n, 1n],
			end: [101n, 0n],
			// INVALID_RANGE.
			log: ["REQUEST_ERROR 17"],
		},
		{
			name: "a range over catalogs that were replaced",
			updates: 2,
			start: [100n, 0n],
			end: [102n, 1n],
			log: [
				"FETCH_OK end {102, 1}",
				"Unknown through {101, 0}",
				"object {102, 0} subgroup 0 a",
				"closed",
			],
		},
	])(
		"a FETCH for the catalog track is answered: $name",
		async ({ updates, start, end, log }) => {
			const publisher = publisherOf("a");
			for (let update = 0; update < updates; update++) {
				publisher.update(catalogOf("a"));
			}
			const asked = fakeFetch(start, end);
			expect(publisher.fetch(asked.request)).toBe(true);
			await settle();
			expect(asked.log).toEqual(log);
		},
	);

	test("a FETCH is answered with the catalog that was published when it came, also when that is replaced meanwhile", async () => {
		const publisher = publisherOf("a");
		const asked = fakeFetch([100n, 0n], [100n, 1n]);
		publisher.fetch(asked.request);
		publisher.update(catalogOf("a", "b"));
		await settle();
		expect(asked.log).toEqual([
			"FETCH_OK end {100, 1}",
			"object {100, 0} subgroup 0 a",
			"closed",
		]);
	});

	test("`end` tells every subscription PUBLISH_DONE with TRACK_ENDED, after the catalog that is being written to it", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		let release: () => void = () => {};
		asked.accepted.holdWrites(
			new Promise((resolve) => {
				release = resolve;
			}),
		);
		publisher.update(catalogOf("a", "b"));
		await settle();
		const ending = publisher.end();
		await settle();
		expect(asked.accepted.log).toEqual([]);
		release();
		await ending;
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
			"PUBLISH_DONE 2",
		]);
	});

	test("after `end`, a request for the track is refused with DOES_NOT_EXIST, and an update is a RangeError", async () => {
		const publisher = publisherOf("a");
		await ended(publisher);
		const subscribe = fakeSubscribe("catalog");
		const fetch = fakeFetch([100n, 0n], [100n, 1n]);
		expect(publisher.subscribe(subscribe.request)).toBe(true);
		expect(publisher.fetch(fetch.request)).toBe(true);
		await settle();
		expect(subscribe.answers).toEqual(["REQUEST_ERROR 16"]);
		expect(fetch.log).toEqual(["REQUEST_ERROR 16"]);
		expect(() => publisher.update(catalogOf("a"))).toThrow(RangeError);
	});

	test("a catalog that is published right before `end` is sent to every subscription, and then PUBLISH_DONE", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		// The last catalog of a broadcast, and the end, with nothing between.
		publisher.update({ version: "draft-01", isComplete: true, tracks: [] });
		await ended(publisher);
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 ",
			"group 101 subgroup 0 end-of-group: closed",
			"PUBLISH_DONE 2",
		]);
	});

	test("`end` while a catalog is being written and a newer one waits: the one under way and the newest are sent first", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		let release: () => void = () => {};
		asked.accepted.holdWrites(
			new Promise((resolve) => {
				release = resolve;
			}),
		);
		publisher.update(catalogOf("a", "b"));
		await settle();
		publisher.update(catalogOf("a", "b", "c"));
		const ending = publisher.end();
		release();
		await ending;
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
			"group 102 subgroup 0 end-of-group: object 0 a,b,c",
			"group 102 subgroup 0 end-of-group: closed",
			"PUBLISH_DONE 2",
		]);
	});

	test("a subscriber that does not take the last catalog in time has its stream reset, and gets PUBLISH_DONE all the same", async () => {
		const publisher = publisherOf("a");
		const stuck = fakeSubscribe("catalog");
		const other = fakeSubscribe("catalog");
		publisher.subscribe(stuck.request);
		publisher.subscribe(other.request);
		await settle();
		stuck.accepted.holdWrites(new Promise(() => {}));
		publisher.update(catalogOf("a", "b"));
		const errors = await reported(() => ended(publisher));
		expect(stuck.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: reset",
			"PUBLISH_DONE 2",
		]);
		expect(other.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
			"PUBLISH_DONE 2",
		]);
		// A subscriber that is slow is no fault.
		expect(errors).toEqual([]);
	});

	test("`end` a second time waits for the first, and nothing is sent twice", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		await Promise.all([publisher.end(), publisher.end()]);
		await ended(publisher);
		expect(asked.accepted.log).toEqual(["PUBLISH_DONE 2"]);
	});

	test("a SUBSCRIBE whose answer was on its way when the track ended is told of the end", async () => {
		const publisher = publisherOf("a");
		let answer: () => void = () => {};
		const asked = fakeSubscribe("catalog", {
			acceptAfter: new Promise((resolve) => {
				answer = resolve;
			}),
		});
		publisher.subscribe(asked.request);
		await ended(publisher);
		answer();
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {100, 0}"]);
		expect(asked.accepted.log).toEqual(["PUBLISH_DONE 2"]);
	});

	test("a subscription with FORWARD 0 that is over is not told of the end of the track", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog", { parameters: { forward: false } });
		publisher.subscribe(asked.request);
		await settle();
		asked.accepted.end({ type: "peer-reset", code: 1 });
		await settle();
		await ended(publisher);
		expect(asked.accepted.log).toEqual([]);
	});

	test.each<{ name: string; kind: "subscribe" | "fetch" }>([
		{ name: "a SUBSCRIBE", kind: "subscribe" },
		{ name: "a FETCH", kind: "fetch" },
	])(
		"$name whose answer could not be sent is refused with INTERNAL_ERROR, and the failure reported",
		async ({ kind }) => {
			const failure = new RangeError("not an answer");
			const publisher = publisherOf("a");
			const subscribe = fakeSubscribe("catalog", { acceptFails: failure });
			const fetch = fakeFetch([100n, 0n], [100n, 1n], { acceptFails: failure });
			const errors = await reported(async () => {
				if (kind === "subscribe") publisher.subscribe(subscribe.request);
				else publisher.fetch(fetch.request);
				await settle();
			});
			expect(kind === "subscribe" ? subscribe.answers : fetch.log).toEqual([
				"REQUEST_ERROR 0",
			]);
			expect(errors).toEqual([failure]);
		},
	);

	test.each<{ name: string; kind: "subscribe" | "fetch" }>([
		{ name: "a SUBSCRIBE", kind: "subscribe" },
		{ name: "a FETCH", kind: "fetch" },
	])(
		"$name that went before it was answered gets no answer, and nothing is reported",
		async ({ kind }) => {
			const gone = { acceptFails: new Error("gone"), gone: true };
			const publisher = publisherOf("a");
			const subscribe = fakeSubscribe("catalog", gone);
			const fetch = fakeFetch([100n, 0n], [100n, 1n], gone);
			const errors = await reported(async () => {
				if (kind === "subscribe") publisher.subscribe(subscribe.request);
				else publisher.fetch(fetch.request);
				await settle();
			});
			expect(kind === "subscribe" ? subscribe.answers : fetch.log).toEqual([]);
			expect(errors).toEqual([]);
		},
	);

	test.each<{ name: string; fails: Error; log: string[]; reports: number }>([
		{
			name: "a failure that is not the subscriber's doing: the stream is reset, and it is reported",
			fails: new RangeError("not written"),
			log: ["FETCH_OK end {100, 1}", "reset"],
			reports: 1,
		},
		{
			name: "the subscriber gave the FETCH up: nothing more is done, and nothing reported",
			fails: new ServedFetchEndedError({ type: "peer-cancelled", code: 1 }),
			log: ["FETCH_OK end {100, 1}"],
			reports: 0,
		},
	])(
		"a FETCH whose stream could not be written: $name",
		async ({ fails, log, reports }) => {
			const asked = fakeFetch([100n, 0n], [100n, 1n], { writeFails: fails });
			const errors = await reported(async () => {
				publisherOf("a").fetch(asked.request);
				await settle();
			});
			expect(asked.log).toEqual(log);
			expect(errors.length).toBe(reports);
		},
	);

	test.each<{ name: string; fails: Error; reports: number }>([
		{
			name: "a failure that is nobody's stop: the stream is reset and the failure reported",
			fails: new Error("the transport failed"),
			reports: 1,
		},
		{
			name: "the subscriber stopped that stream: it is reset, and nothing is reported",
			fails: new SubgroupClosedError("stopped", { code: 1 }),
			reports: 0,
		},
	])(
		"a catalog that could not be written, $name; the next catalog goes on a new stream, and the track can be ended",
		async ({ fails, reports }) => {
			const publisher = publisherOf("a");
			const asked = fakeSubscribe("catalog");
			publisher.subscribe(asked.request);
			await settle();
			asked.accepted.failNextWrite(fails);
			const errors = await reported(async () => {
				publisher.update(catalogOf("a", "b"));
				await settle();
				publisher.update(catalogOf("a", "b", "c"));
				await ended(publisher);
			});
			expect(asked.accepted.log).toEqual([
				"group 101 subgroup 0 end-of-group: reset",
				"group 102 subgroup 0 end-of-group: object 0 a,b,c",
				"group 102 subgroup 0 end-of-group: closed",
				"PUBLISH_DONE 2",
			]);
			expect(errors.length).toBe(reports);
		},
	);

	test.each<{ turns: number }>([
		{ turns: 0 },
		{ turns: 1 },
		{ turns: 2 },
		{ turns: 3 },
		{ turns: 4 },
		{ turns: 5 },
		{ turns: 6 },
	])(
		"a catalog that is published $turns microtasks after the stream of the one before was closed is written as well",
		async ({ turns }) => {
			const publisher = publisherOf("a");
			const asked = fakeSubscribe("catalog");
			publisher.subscribe(asked.request);
			await settle();
			let left = turns;
			let published = false;
			const step = (): void => {
				if (left > 0) {
					left -= 1;
					queueMicrotask(step);
					return;
				}
				publisher.update(catalogOf("a", "b", "c"));
			};
			asked.accepted.onClose(() => {
				if (published) return;
				published = true;
				step();
			});
			publisher.update(catalogOf("a", "b"));
			await settle();
			expect(asked.accepted.log).toEqual([
				"group 101 subgroup 0 end-of-group: object 0 a,b",
				"group 101 subgroup 0 end-of-group: closed",
				"group 102 subgroup 0 end-of-group: object 0 a,b,c",
				"group 102 subgroup 0 end-of-group: closed",
			]);
		},
	);

	test("a subscriber whose stream does not open: `end` resolves after twice the time and reports it; when the stream opens after all it is reset, and PUBLISH_DONE follows", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		let open: () => void = () => {};
		asked.accepted.holdOpens(
			new Promise((resolve) => {
				open = resolve;
			}),
		);
		publisher.update(catalogOf("a", "b"));
		const errors = await reported(() => ended(publisher));
		expect(errors).toEqual([
			new Error(
				"a subscription of the catalog track could not be told of its end in time: its stream does not open",
			),
		]);
		expect(asked.accepted.log).toEqual([]);
		open();
		await settle();
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: reset",
			"PUBLISH_DONE 2",
		]);
	});

	test("a subscriber that is slow to end its request stream after PUBLISH_DONE is no fault, however long that takes", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		// Three times the time `end` gives a subscriber for a catalog.
		asked.accepted.holdDone(new Promise((resolve) => setTimeout(resolve, 60)));
		publisher.update(catalogOf("a", "b"));
		const errors = await reported(() => ended(publisher));
		expect(errors).toEqual([]);
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
			"PUBLISH_DONE 2",
		]);
	});

	test("a SUBSCRIBE whose answer was on its way when the track ended gets the catalog that was published meanwhile, and then PUBLISH_DONE", async () => {
		const publisher = publisherOf("a");
		let answer: () => void = () => {};
		const asked = fakeSubscribe("catalog", {
			acceptAfter: new Promise((resolve) => {
				answer = resolve;
			}),
		});
		publisher.subscribe(asked.request);
		publisher.update(catalogOf("a", "b"));
		await ended(publisher);
		answer();
		await settle();
		expect(asked.answers).toEqual(["SUBSCRIBE_OK largest {100, 0}"]);
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: object 0 a,b",
			"group 101 subgroup 0 end-of-group: closed",
			"PUBLISH_DONE 2",
		]);
	});

	test("a SUBSCRIBE with FORWARD 0 whose answer was on its way when the track ended gets PUBLISH_DONE alone", async () => {
		const publisher = publisherOf("a");
		let answer: () => void = () => {};
		const asked = fakeSubscribe("catalog", {
			parameters: { forward: false },
			acceptAfter: new Promise((resolve) => {
				answer = resolve;
			}),
		});
		publisher.subscribe(asked.request);
		publisher.update(catalogOf("a", "b"));
		await ended(publisher);
		answer();
		await settle();
		expect(asked.accepted.log).toEqual(["PUBLISH_DONE 2"]);
	});

	test("`current` is a copy: changing it changes nothing of what is published", () => {
		const publisher = publisherOf("a");
		const { catalog, at } = publisher.current;
		Object.assign(catalog, { tracks: [] });
		Object.assign(at, { group: 7n });
		expect(publisher.current.catalog.tracks.length).toBe(1);
		expect(publisher.current.at).toEqual({ group: 100n, object: 0n });
	});

	test.each<{ endTimeoutMs: number }>([
		{ endTimeoutMs: Number.NaN },
		{ endTimeoutMs: -1 },
		{ endTimeoutMs: 1.5 },
		{ endTimeoutMs: 2147483648 },
		{ endTimeoutMs: Number.POSITIVE_INFINITY },
	])("endTimeoutMs of $endTimeoutMs is a RangeError", ({ endTimeoutMs }) => {
		expect(() =>
			createCatalogPublisher({
				namespace: NAMESPACE,
				catalog: catalogOf("a"),
				endTimeoutMs,
			}),
		).toThrow(RangeError);
	});

	test.each<{ name: string; fails: Error }>([
		{
			name: "the subscription was ended by its subscriber",
			fails: new PublishEndedError({ type: "peer-reset", code: 1 }),
		},
		{
			name: "the session is over",
			fails: new SessionClosedError({ type: "peer", code: 0, reason: "" }),
		},
	])(
		"a catalog that could not be written because $name: nothing is reset, and nothing reported",
		async ({ fails }) => {
			const publisher = publisherOf("a");
			const asked = fakeSubscribe("catalog");
			publisher.subscribe(asked.request);
			await settle();
			asked.accepted.failNextWrite(fails);
			const errors = await reported(async () => {
				publisher.update(catalogOf("a", "b"));
				await settle();
			});
			expect(asked.accepted.log).toEqual([]);
			expect(errors).toEqual([]);
		},
	);

	test("after `end`, no timer is left", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		publisher.update(catalogOf("a", "b"));
		jest.useFakeTimers();
		try {
			await publisher.end();
			expect(jest.getTimerCount()).toBe(0);
		} finally {
			jest.useRealTimers();
		}
		expect(asked.accepted.log.at(-1)).toBe("PUBLISH_DONE 2");
	});

	test("after `end` gave a subscriber up for the catalog it does not take, no timer is left", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		asked.accepted.holdWrites(new Promise(() => {}));
		publisher.update(catalogOf("a", "b"));
		await settle();
		jest.useFakeTimers();
		try {
			expect(await endedWithin(publisher, [20])).toBe(true);
			expect(jest.getTimerCount()).toBe(0);
		} finally {
			jest.useRealTimers();
		}
		expect(asked.accepted.log).toEqual([
			"group 101 subgroup 0 end-of-group: reset",
			"PUBLISH_DONE 2",
		]);
	});

	test("after `end` left a subscriber whose stream does not open, no timer is left", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		asked.accepted.holdOpens(new Promise(() => {}));
		publisher.update(catalogOf("a", "b"));
		await settle();
		jest.useFakeTimers();
		try {
			const errors = await reported(async () => {
				// The first time, and not yet the second.
				expect(await endedWithin(publisher, [20, 19])).toBe(false);
				expect(await endedWithin(publisher, [1])).toBe(true);
			});
			expect(errors.length).toBe(1);
			expect(jest.getTimerCount()).toBe(0);
		} finally {
			jest.useRealTimers();
		}
		expect(asked.accepted.log).toEqual([]);
	});

	test("a SUBSCRIBE that is answered after `end` leaves no timer when it was told", async () => {
		const publisher = publisherOf("a");
		let answer: () => void = () => {};
		const asked = fakeSubscribe("catalog", {
			acceptAfter: new Promise((resolve) => {
				answer = resolve;
			}),
		});
		publisher.subscribe(asked.request);
		publisher.update(catalogOf("a", "b"));
		jest.useFakeTimers();
		try {
			expect(await endedWithin(publisher, [])).toBe(true);
			answer();
			await turns();
			expect(asked.accepted.log).toEqual([
				"group 101 subgroup 0 end-of-group: object 0 a,b",
				"group 101 subgroup 0 end-of-group: closed",
				"PUBLISH_DONE 2",
			]);
			expect(jest.getTimerCount()).toBe(0);
		} finally {
			jest.useRealTimers();
		}
	});

	test("a subscription that ended is written to no more", async () => {
		const publisher = publisherOf("a");
		const asked = fakeSubscribe("catalog");
		publisher.subscribe(asked.request);
		await settle();
		asked.accepted.end({ type: "peer-reset", code: 1 });
		await settle();
		const errors = await reported(async () => {
			publisher.update(catalogOf("a", "b"));
			await settle();
			await ended(publisher);
		});
		expect(asked.accepted.log).toEqual([]);
		expect(errors).toEqual([]);
	});
});
