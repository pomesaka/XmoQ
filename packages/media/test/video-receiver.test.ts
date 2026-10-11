import { describe, expect, test } from "bun:test";
import type { ReceivedObject } from "@xmoq/moqt";
import {
	createVideoReceiver,
	type VideoObject,
	type VideoReceiverEvent,
	type VideoReceiverOptions,
} from "../src/index.ts";
import { capturedObjects } from "./captured.ts";

/** A NAL unit of two bytes with its length before it. */
const PAYLOAD = new Uint8Array([0x00, 0x00, 0x00, 0x02, 0xaa, 0xbb]);
/** A length that goes beyond the payload. */
const BROKEN = new Uint8Array([0x00, 0x00, 0x00, 0x09, 0xaa, 0xbb]);

/**
 * An Object of a test, from "<group>,<object>" and a mark: "E" for End of
 * Group, "T" for End of Track, "!" for a payload that is not LOC's, "?" for
 * one without a Timestamp, "~" for one without a payload. Each of the
 * others has 6 bytes of payload.
 */
function objectOf(token: string): VideoObject {
	const [, group, object, mark] = /^(\d+),(\d+)([ET!?~]?)$/.exec(token) ?? [];
	if (group === undefined || object === undefined) {
		throw new Error(`not an Object of this test: ${token}`);
	}
	return {
		groupId: BigInt(group),
		object: {
			objectId: BigInt(object),
			properties: mark === "?" ? [] : [{ type: 0x10n, value: 1000n }],
			status:
				mark === "E" ? "EndOfGroup" : mark === "T" ? "EndOfTrack" : "Normal",
			payload:
				mark === "!" ? BROKEN : mark === "~" ? new Uint8Array() : PAYLOAD,
		},
	};
}

/** An event in a word: "f5,0k" a key frame, "f5,1" a frame, "d5,2:gap" a drop, "end5,3" the end of the track. */
function word(event: VideoReceiverEvent): string {
	const at = `${event.group},${event.object}`;
	switch (event.type) {
		case "frame":
			return `f${at}${event.key ? "k" : ""}`;
		case "dropped":
			return `d${at}:${event.because}`;
		case "end-of-track":
			return `end${at}`;
	}
}

/** What each Object that is pushed gives, the events of one push in one string. */
function after(objects: string[], options?: VideoReceiverOptions): string[] {
	const receiver = createVideoReceiver(options);
	return objects.map((token) =>
		receiver.push(objectOf(token)).map(word).join(" "),
	);
}

describe("createVideoReceiver: which frames a decoder gets, and in which order", () => {
	test.each<{ name: string; objects: string[]; events: string[] }>([
		{
			name: "a Group in order: Object 0 is the key frame",
			objects: ["5,0", "5,1", "5,2"],
			events: ["f5,0k", "f5,1", "f5,2"],
		},
		{
			name: "an Object ahead of its turn is held until the one before it has come",
			objects: ["5,0", "5,2", "5,1"],
			events: ["f5,0k", "", "f5,1 f5,2"],
		},
		{
			name: "a receiver that comes in the middle of a Group waits for Object 0 of a later one",
			objects: ["5,3", "5,4", "6,0", "6,1"],
			events: ["", "", "d5,3:no-key-frame d5,4:no-key-frame f6,0k", "f6,1"],
		},
		{
			name: "the next Group begins with its Object 0",
			objects: ["5,0", "5,1", "6,0"],
			events: ["f5,0k", "f5,1", "f6,0k"],
		},
		{
			name: "a frame that has not come when a later Group begins is not waited for, and is late when it comes",
			objects: ["5,0", "5,2", "6,0", "5,1"],
			events: ["f5,0k", "", "d5,2:gap f6,0k", "d5,1:late"],
		},
		{
			name: "a frame of a later Group that comes before that Group's Object 0 is held for it",
			objects: ["5,0", "6,1", "6,0"],
			events: ["f5,0k", "", "f6,0k f6,1"],
		},
		{
			name: "the frames of the Group that is played go on while Objects of a later one are held",
			objects: ["6,1", "5,0", "5,1", "6,0"],
			events: ["", "f5,0k", "f5,1", "f6,0k f6,1"],
		},
		{
			name: "what comes of a Group after a later one began is late",
			objects: ["5,0", "6,0", "5,1"],
			events: ["f5,0k", "f6,0k", "d5,1:late"],
		},
		{
			name: "Groups can be skipped, and do not come back",
			objects: ["5,0", "8,0", "6,0"],
			events: ["f5,0k", "f8,0k", "d6,0:late"],
		},
		{
			name: "a second copy of a frame that was handed on",
			objects: ["5,0", "5,0"],
			events: ["f5,0k", "d5,0:late"],
		},
		{
			name: "a second copy of an Object that is held",
			objects: ["5,0", "5,2", "5,2", "5,1"],
			events: ["f5,0k", "", "d5,2:late", "f5,1 f5,2"],
		},
		{
			name: "a payload that is not LOC's ends its Group: what was held after it goes, and what comes after it is late",
			objects: ["5,0", "5,2", "5,1!", "5,3", "6,0"],
			events: [
				"f5,0k",
				"",
				"d5,1:unusable d5,2:after-unusable",
				"d5,3:late",
				"f6,0k",
			],
		},
		{
			name: "an Object 0 without a Timestamp: its Group is not played",
			objects: ["5,0?", "5,1", "6,0"],
			events: ["d5,0:unusable", "d5,1:late", "f6,0k"],
		},
		{
			name: "End of Group: nothing after it in the Group is waited for",
			objects: ["5,0", "5,1E", "5,2", "6,0"],
			events: ["f5,0k", "", "d5,2:late", "f6,0k"],
		},
		{
			name: "End of Group that comes before the frame before it",
			objects: ["5,0", "5,2E", "5,1"],
			events: ["f5,0k", "", "f5,1"],
		},
		{
			name: "End of Group for a Group that has not begun: the Group ends there when it is played",
			objects: ["6,2E", "6,0", "6,1", "6,2", "7,0"],
			events: ["", "f6,0k", "f6,1", "d6,2:late", "f7,0k"],
		},
		{
			name: "End of Group: what was held at or after it does not exist",
			objects: ["5,0", "5,3", "5,2E"],
			events: ["f5,0k", "", "d5,3:after-end"],
		},
		{
			name: "End of Group: what was held after it goes in the order of the Object IDs",
			objects: ["5,0", "5,4", "5,3", "5,2E"],
			events: ["f5,0k", "", "", "d5,3:after-end d5,4:after-end"],
		},
		{
			name: "of two ends of one Group, the lower one holds",
			objects: ["5,0", "5,2E", "5,4E", "5,3"],
			events: ["f5,0k", "", "", "d5,3:after-end"],
		},
		{
			name: "End of Track: an Object at its Location does not exist",
			objects: ["5,0", "5,2T", "5,2", "5,1"],
			events: ["f5,0k", "end5,2", "d5,2:after-end", "f5,1"],
		},
		{
			name: "End of Group of a Group that was passed says nothing new",
			objects: ["5,0", "6,0", "5,1E"],
			events: ["f5,0k", "f6,0k", ""],
		},
		{
			name: "End of Track: nothing at or after it is a frame",
			objects: ["5,0", "5,1T", "6,0"],
			events: ["f5,0k", "end5,1", "d6,0:after-end"],
		},
		{
			name: "End of Track is told when it arrives, also before a frame that stands before it",
			objects: ["5,0", "5,2T", "5,1", "5,2"],
			events: ["f5,0k", "end5,2", "f5,1", "d5,2:late"],
		},
		{
			name: "End of Track: what was held at or after it goes, in this Group and in later ones",
			objects: ["5,0", "5,3", "6,1", "5,2T"],
			events: ["f5,0k", "", "", "end5,2 d5,3:after-end d6,1:after-end"],
		},
		{
			name: "End of Track a second time is not told again, and an earlier one is",
			objects: ["5,0", "6,0T", "6,0T", "5,3T"],
			events: ["f5,0k", "end6,0", "", "end5,3"],
		},
		{
			name: "an Object without a payload cannot be used",
			objects: ["5,0", "5,1~", "5,2"],
			events: ["f5,0k", "d5,1:unusable", "d5,2:late"],
		},
		{
			name: "Groups that come in no order: the lowest one with an Object 0 begins, and what is held of Groups before it goes",
			objects: ["9,1", "6,1", "8,0", "9,0"],
			events: ["", "", "d6,1:no-key-frame f8,0k", "f9,0k f9,1"],
		},
		{
			name: "what is dropped of a Group is told in the order of the Object IDs, however it came",
			objects: ["5,4", "5,3", "6,0"],
			events: ["", "", "d5,3:no-key-frame d5,4:no-key-frame f6,0k"],
		},
	])("$name", ({ objects, events }) => {
		expect(after(objects)).toEqual(events);
	});
});

describe("createVideoReceiver: what is held has limits", () => {
	test.each<{
		name: string;
		options: VideoReceiverOptions;
		objects: string[];
		events: string[];
	}>([
		{
			name: "more Objects held behind a missing frame than the limit: the Group is given up, and the frame is late when it comes",
			options: { maxHeldObjects: 2 },
			objects: ["5,0", "5,2", "5,3", "5,4", "5,1", "6,0"],
			events: [
				"f5,0k",
				"",
				"",
				"d5,2:gap d5,3:gap d5,4:gap",
				"d5,1:late",
				"f6,0k",
			],
		},
		{
			name: "as many as the limit are held",
			options: { maxHeldObjects: 2 },
			objects: ["5,0", "5,2", "5,3", "5,1"],
			events: ["f5,0k", "", "", "f5,1 f5,2 f5,3"],
		},
		{
			name: "more bytes held than the limit, 6 bytes an Object",
			options: { maxHeldBytes: 11 },
			objects: ["5,0", "5,2", "5,3"],
			events: ["f5,0k", "", "d5,2:gap d5,3:gap"],
		},
		{
			name: "as many bytes as the limit are held",
			options: { maxHeldBytes: 12 },
			objects: ["5,0", "5,2", "5,3", "5,1"],
			events: ["f5,0k", "", "", "f5,1 f5,2 f5,3"],
		},
		{
			name: "no key frame yet: the oldest Group that is held goes first",
			options: { maxHeldObjects: 2 },
			objects: ["5,3", "6,3", "7,3"],
			events: ["", "", "d5,3:overflow"],
		},
		{
			name: "a Group that lost Objects to the limit can still begin, and ends where the first of them is missing",
			options: { maxHeldObjects: 2 },
			objects: ["5,1", "5,2", "5,3", "5,0", "5,1", "6,0"],
			events: [
				"",
				"",
				"d5,1:overflow d5,2:overflow d5,3:overflow",
				"f5,0k",
				"f5,1",
				"f6,0k",
			],
		},
		{
			name: "as many ends of Groups are remembered as Objects may be held",
			options: { maxHeldObjects: 2 },
			objects: ["5,5E", "6,5E", "5,0", "5,5"],
			events: ["", "", "f5,0k", "d5,5:after-end"],
		},
		{
			name: "one more, and the end of the lowest Group is forgotten: an Object after it is held like any other",
			options: { maxHeldObjects: 2 },
			objects: ["5,5E", "6,5E", "7,5E", "5,0", "5,5"],
			events: ["", "", "", "f5,0k", ""],
		},
		{
			name: "the Group that is played is not given up for Objects of later Groups that are held",
			options: { maxHeldObjects: 2 },
			objects: ["5,0", "6,1", "7,1", "8,1", "5,1"],
			events: ["f5,0k", "", "", "d6,1:overflow", "f5,1"],
		},
	])("$name", ({ options, objects, events }) => {
		expect(after(objects, options)).toEqual(events);
	});

	test.each<{ options: VideoReceiverOptions }>([
		{ options: { maxHeldObjects: 0 } },
		{ options: { maxHeldObjects: 1.5 } },
		{ options: { maxHeldObjects: Number.NaN } },
		{ options: { maxHeldObjects: 4097 } },
		{ options: { maxHeldBytes: 0 } },
		{ options: { trackTimescale: 0n } },
		{ options: { trackTimescale: -1n } },
		{ options: { trackTimescale: 18446744073709551616n } },
		{ options: { maxHeldBytes: Number.POSITIVE_INFINITY } },
	])("$options is a RangeError", ({ options }) => {
		expect(() => createVideoReceiver(options)).toThrow(RangeError);
	});
});

describe("createVideoReceiver: the time and the data of a frame", () => {
	/** The one frame that Object 0 with these properties gives. */
	function frameOf(
		properties: VideoObject["object"]["properties"],
		options?: VideoReceiverOptions,
	): VideoReceiverEvent | undefined {
		const [event] = createVideoReceiver(options).push({
			groupId: 0n,
			object: { objectId: 0n, properties, status: "Normal", payload: PAYLOAD },
		});
		return event;
	}

	test.each<{
		name: string;
		properties: { type: bigint; value: bigint }[];
		trackTimescale?: bigint;
		timestamp: bigint;
	}>([
		{
			name: "a Timestamp alone is wall-clock time in microseconds",
			properties: [{ type: 0x10n, value: 1791690527000000n }],
			timestamp: 1791690527000000n,
		},
		{
			name: "with a Timescale of the Object",
			properties: [
				{ type: 0x08n, value: 90000n },
				{ type: 0x10n, value: 3000n },
			],
			timestamp: 33333n,
		},
		{
			name: "with the Timescale of the track",
			properties: [{ type: 0x10n, value: 3000n }],
			trackTimescale: 90000n,
			timestamp: 33333n,
		},
		{
			name: "the largest time that is handed on",
			properties: [{ type: 0x10n, value: 9007199254740991n }],
			timestamp: 9007199254740991n,
		},
		{
			name: "the largest Timescale of a track",
			properties: [{ type: 0x10n, value: 18446744073709551615n }],
			trackTimescale: 18446744073709551615n,
			timestamp: 1000000n,
		},
		{
			name: "the Timescale of the Object goes before the track's",
			properties: [
				{ type: 0x08n, value: 1000n },
				{ type: 0x10n, value: 3000n },
			],
			trackTimescale: 90000n,
			timestamp: 3000000n,
		},
	])("$name", ({ properties, trackTimescale, timestamp }) => {
		const options = trackTimescale === undefined ? {} : { trackTimescale };
		expect(frameOf(properties, options)).toMatchObject({
			type: "frame",
			timestamp,
		});
	});

	test("the data has a start code where the payload had a length", () => {
		expect(frameOf([{ type: 0x10n, value: 0n }])).toEqual({
			type: "frame",
			group: 0n,
			object: 0n,
			key: true,
			timestamp: 0n,
			data: new Uint8Array([0x00, 0x00, 0x00, 0x01, 0xaa, 0xbb]),
		});
	});

	test.each<{
		name: string;
		properties: { type: bigint; value: bigint }[];
		detail: string;
	}>([
		{ name: "no Timestamp", properties: [], detail: "no Timestamp" },
		{
			name: "a time that is no exact number for a decoder",
			properties: [{ type: 0x10n, value: 9007199254740992n }],
			detail: "a time beyond 2^53 - 1 microseconds",
		},
		{
			name: "a Timescale of 0",
			properties: [
				{ type: 0x08n, value: 0n },
				{ type: 0x10n, value: 1n },
			],
			detail: "the Timescale is 0",
		},
	])(
		"a frame that cannot be used says why: $name",
		({ properties, detail }) => {
			expect(frameOf(properties)).toEqual({
				type: "dropped",
				group: 0n,
				object: 0n,
				because: "unusable",
				detail,
			});
		},
	);
});

describe("createVideoReceiver: frames of moqlivemock's publisher (fixtures/moqt-18)", () => {
	test("the two Objects of the captured stream, as @xmoq/moqt decodes them, are a key frame and the frame after it, in the form a decoder takes without a description", () => {
		const receiver = createVideoReceiver();
		const objects: ReceivedObject[] = capturedObjects();
		expect(objects.length).toBe(2);
		const frames = objects.flatMap((object) => receiver.push(object));
		expect(
			frames.map((frame) =>
				frame.type === "frame"
					? {
							group: frame.group,
							object: frame.object,
							key: frame.key,
							timestamp: frame.timestamp,
							length: frame.data.length,
							start: Array.from(frame.data.subarray(0, 5)),
						}
					: frame,
			),
		).toEqual([
			{
				group: 1791690527n,
				object: 0n,
				key: true,
				timestamp: 1791690527000000n,
				length: 21533,
				start: [0x00, 0x00, 0x00, 0x01, 0x67],
			},
			{
				group: 1791690527n,
				object: 1n,
				key: false,
				timestamp: 1791690527040000n,
				length: 931,
				start: [0x00, 0x00, 0x00, 0x01, 0x41],
			},
		]);
	});
});

describe("createVideoReceiver: the caller gives a Group up", () => {
	/** As `after`, with "x" and "X" in place of an Object: the Group is given up, as the decoder failed or fell behind. */
	function afterGivingUp(objects: string[]): string[] {
		const receiver = createVideoReceiver();
		return objects.map((token) =>
			(token === "x"
				? receiver.abandonGroup("decoder-failed")
				: token === "X"
					? receiver.abandonGroup("decoder-behind")
					: receiver.push(objectOf(token))
			)
				.map(word)
				.join(" "),
		);
	}

	test.each<{ name: string; objects: string[]; events: string[] }>([
		{
			name: "nothing more of the Group is a frame, and the next frame is Object 0 of a later Group",
			objects: ["5,0", "x", "5,1", "6,0", "6,1"],
			events: ["f5,0k", "", "d5,1:late", "f6,0k", "f6,1"],
		},
		{
			name: "what was held of the Group goes, with the reason that was given",
			objects: ["5,0", "5,2", "5,3", "x"],
			events: ["f5,0k", "", "", "d5,2:decoder-failed d5,3:decoder-failed"],
		},
		{
			name: "the other reason",
			objects: ["5,0", "5,2", "X"],
			events: ["f5,0k", "", "d5,2:decoder-behind"],
		},
		{
			name: "what is held of a later Group stays, and is played when its Object 0 comes",
			objects: ["5,0", "6,1", "x", "6,0"],
			events: ["f5,0k", "", "", "f6,0k f6,1"],
		},
		{
			name: "while a key frame is waited for, there is nothing to give up",
			objects: ["5,3", "x", "6,0"],
			events: ["", "", "d5,3:no-key-frame f6,0k"],
		},
		{
			name: "twice is once",
			objects: ["5,0", "x", "x", "5,1"],
			events: ["f5,0k", "", "", "d5,1:late"],
		},
	])("$name", ({ objects, events }) => {
		expect(afterGivingUp(objects)).toEqual(events);
	});
});
