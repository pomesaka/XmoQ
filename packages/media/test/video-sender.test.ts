import { describe, expect, test } from "bun:test";
import { readLocTiming, toAnnexB } from "@xmoq/loc";
import {
	createVideoReceiver,
	createVideoSender,
	DEFAULT_MAPPING,
	DEFAULT_MAX_PAYLOAD_LENGTH,
	type VideoChunk,
	type VideoObject,
	type VideoReceiverEvent,
	type VideoSenderOptions,
	type VideoSenderStep,
	type VideoStreamMapping,
} from "../src/index.ts";
import { capturedChunk, capturedObjects } from "./captured.ts";

/** A start code and a NAL unit of two bytes. */
const DATA = [0x00, 0x00, 0x00, 0x01, 0xaa, 0xbb];
const OPTIONS: VideoSenderOptions = {
	firstGroup: 5n,
	epoch: 1791690527000000n,
	maxPayloadLength: 8,
};

/**
 * A chunk of a test: "k" a key chunk, "d" a delta chunk, each with 6 bytes
 * of data; "K" and "D" the same with 9 bytes, one over the limit of these
 * tests; "k3" and "d3" with a start code of 3 bytes; "kL" with a length of
 * 4 bytes in place of the start code; "dT" a delta chunk whose time is not
 * a number.
 */
function chunkOf(token: string): VideoChunk {
	const data = token.endsWith("L")
		? [0x00, 0x00, 0x00, 0x02, 0xaa, 0xbb]
		: token.endsWith("3")
			? [0x00, 0x00, 0x01, 0xaa, 0xbb, 0xcc]
			: token === "K" || token === "D"
				? [...DATA, 0xcc, 0xdd, 0xee]
				: DATA;
	return {
		type: token.toLowerCase().startsWith("k") ? "key" : "delta",
		timestamp: token.endsWith("T") ? Number.NaN : 0,
		data: new Uint8Array(data),
	};
}

/** A step in a word: "5,1 s1 open close" an Object of Group 5 with Object ID 1 in Subgroup 1, "close5" the stream of Group 5 finished, "reset5" that stream reset, "skip:<reason>". */
function word(step: VideoSenderStep): string {
	switch (step.type) {
		case "object":
			return [
				`${step.groupId},${step.objectId}`,
				`s${step.subgroupId}`,
				...(step.opensStream ? ["open"] : []),
				...(step.closesStream ? ["close"] : []),
			].join(" ");
		case "close":
			return `close${step.groupId}`;
		case "reset":
			return `reset${step.groupId}`;
		case "skipped":
			return `skip:${step.because}`;
	}
}

/** What each chunk gives, the steps of one chunk joined by "; ". "end" in place of a chunk ends the Group as the track ends, "fail" as an Object could not be written. */
function after(chunks: string[], mapping?: VideoStreamMapping): string[] {
	const sender = createVideoSender(
		mapping === undefined ? OPTIONS : { ...OPTIONS, mapping },
	);
	return chunks.map((token) =>
		(token === "end"
			? sender.endGroup("track-ended")
			: token === "fail"
				? sender.endGroup("write-failed")
				: sender.push(chunkOf(token))
		)
			.map(word)
			.join("; "),
	);
}

describe("createVideoSender: which Object a chunk is, and on which stream", () => {
	test.each<{
		name: string;
		mapping?: VideoStreamMapping;
		chunks: string[];
		steps: string[];
	}>([
		{
			name: "without a mapping, a Group has one stream, Subgroup 0, finished when a key chunk begins the next Group",
			chunks: ["k", "d", "d", "k", "d"],
			steps: [
				"5,0 s0 open",
				"5,1 s0",
				"5,2 s0",
				"close5; 6,0 s0 open",
				"6,1 s0",
			],
		},
		{
			name: "a stream for every Object: each has a stream and a Subgroup of its own; a key chunk begins the next Group",
			mapping: "stream-per-object",
			chunks: ["k", "d", "d", "k", "d"],
			steps: [
				"5,0 s0 open close",
				"5,1 s1 open close",
				"5,2 s2 open close",
				"6,0 s0 open close",
				"6,1 s1 open close",
			],
		},
		{
			name: "a stream for every Group, said: the same as without a mapping",
			mapping: "stream-per-group",
			chunks: ["k", "d", "d", "k", "d"],
			steps: [
				"5,0 s0 open",
				"5,1 s0",
				"5,2 s0",
				"close5; 6,0 s0 open",
				"6,1 s0",
			],
		},
		{
			name: "a delta chunk before the first key chunk is not sent, and takes no Group ID",
			mapping: "stream-per-object",
			chunks: ["d", "d", "k"],
			steps: ["skip:no-key-frame", "skip:no-key-frame", "5,0 s0 open close"],
		},
		{
			name: "a chunk over the length ends its Group: nothing after it is sent before the next key chunk",
			mapping: "stream-per-object",
			chunks: ["k", "D", "d", "k"],
			steps: [
				"5,0 s0 open close",
				"skip:too-large",
				"skip:no-key-frame",
				"6,0 s0 open close",
			],
		},
		{
			name: "the same with a stream for every Group: the stream is finished where the Group is cut",
			mapping: "stream-per-group",
			chunks: ["k", "D", "d", "k"],
			steps: [
				"5,0 s0 open",
				"close5; skip:too-large",
				"skip:no-key-frame",
				"6,0 s0 open",
			],
		},
		{
			name: "a chunk without the start code, in a Group whose stream is open: the stream is finished",
			mapping: "stream-per-group",
			chunks: ["k", "d3", "d", "k"],
			steps: [
				"5,0 s0 open",
				"close5; skip:not-annex-b",
				"skip:no-key-frame",
				"6,0 s0 open",
			],
		},
		{
			name: "a chunk without a time, in a Group whose stream is open: the stream is finished",
			mapping: "stream-per-group",
			chunks: ["k", "dT", "d", "k"],
			steps: [
				"5,0 s0 open",
				"close5; skip:no-time",
				"skip:no-key-frame",
				"6,0 s0 open",
			],
		},
		{
			name: "a key chunk that is not sent takes no Group ID, and ends the Group before it",
			mapping: "stream-per-group",
			chunks: ["k", "K", "d", "k"],
			steps: [
				"5,0 s0 open",
				"close5; skip:too-large",
				"skip:no-key-frame",
				"6,0 s0 open",
			],
		},
		{
			name: "a chunk with a start code of 3 bytes is not sent",
			mapping: "stream-per-object",
			chunks: ["k3", "d", "k", "d3", "d"],
			steps: [
				"skip:not-annex-b",
				"skip:no-key-frame",
				"5,0 s0 open close",
				"skip:not-annex-b",
				"skip:no-key-frame",
			],
		},
		{
			name: "a chunk with a length before its NAL unit is not sent",
			mapping: "stream-per-object",
			chunks: ["kL", "k"],
			steps: ["skip:not-annex-b", "5,0 s0 open close"],
		},
		{
			name: "after the Group was ended, the next Object is the next key chunk",
			mapping: "stream-per-group",
			chunks: ["k", "d", "end", "d", "k", "end", "end"],
			steps: [
				"5,0 s0 open",
				"5,1 s0",
				"close5",
				"skip:no-key-frame",
				"6,0 s0 open",
				"close6",
				"",
			],
		},
		{
			name: "an Object could not be written: the stream of the Group is reset, not finished, and the next Object is the next key chunk",
			mapping: "stream-per-group",
			chunks: ["k", "d", "fail", "d", "k", "fail", "fail"],
			steps: [
				"5,0 s0 open",
				"5,1 s0",
				"reset5",
				"skip:no-key-frame",
				"6,0 s0 open",
				"reset6",
				"",
			],
		},
		{
			name: "an Object could not be written, with a stream for every Object: no stream is open here",
			mapping: "stream-per-object",
			chunks: ["k", "fail", "d", "k"],
			steps: [
				"5,0 s0 open close",
				"",
				"skip:no-key-frame",
				"6,0 s0 open close",
			],
		},
		{
			name: "with a stream for every Object, ending the Group leaves no stream to finish",
			mapping: "stream-per-object",
			chunks: ["k", "end", "d", "k"],
			steps: [
				"5,0 s0 open close",
				"",
				"skip:no-key-frame",
				"6,0 s0 open close",
			],
		},
	])("$name", ({ mapping, chunks, steps }) => {
		expect(after(chunks, mapping)).toEqual(steps);
	});

	test("as long as the limit is sent, and one byte more is not", () => {
		const sender = createVideoSender({ ...OPTIONS, maxPayloadLength: 6 });
		expect(sender.push(chunkOf("k")).map(word)).toEqual(["5,0 s0 open"]);
		const short = createVideoSender({ ...OPTIONS, maxPayloadLength: 5 });
		expect(short.push(chunkOf("k")).map(word)).toEqual(["skip:too-large"]);
	});
});

describe("createVideoSender: the time and the payload of an Object", () => {
	const first = (
		timestamp: number,
		epoch: bigint,
	): VideoSenderStep | undefined =>
		createVideoSender({ firstGroup: 0n, epoch }).push({
			type: "key",
			timestamp,
			data: new Uint8Array(DATA),
		})[0];

	test.each<{ name: string; timestamp: number; epoch: bigint; value: bigint }>([
		{
			name: "the encoder's time is counted from the epoch that was given",
			timestamp: 40000,
			epoch: 1791690527000000n,
			value: 1791690527040000n,
		},
		{
			name: "the first frame",
			timestamp: 0,
			epoch: 1791690527000000n,
			value: 1791690527000000n,
		},
		{
			name: "the largest time that is sent",
			timestamp: 0,
			epoch: 9007199254740991n,
			value: 9007199254740991n,
		},
		{
			name: "a time before the encoder's zero",
			timestamp: -10,
			epoch: 10n,
			value: 0n,
		},
	])(
		"the properties say the time in wall-clock microseconds, and no unit: $name",
		({ timestamp, epoch, value }) => {
			const step = first(timestamp, epoch);
			if (step?.type !== "object") throw new Error(`no Object: ${step?.type}`);
			// Read as LOC is read here; the number of the property is the
			// matter of @xmoq/loc.
			expect(readLocTiming(step.properties)).toEqual({
				type: "ok",
				timing: { timestamp: value },
			});
			expect(step.properties.length).toBe(1);
		},
	);

	test.each<{ name: string; timestamp: number; epoch: bigint }>([
		{ name: "before the Unix epoch", timestamp: -11, epoch: 10n },
		{ name: "a fraction", timestamp: 0.5, epoch: 10n },
		{ name: "without end", timestamp: Number.POSITIVE_INFINITY, epoch: 10n },
		{ name: "a number that is no exact one", timestamp: 1e21, epoch: 10n },
		{
			name: "one microsecond over the largest time",
			timestamp: 1,
			epoch: 9007199254740991n,
		},
		{ name: "not a number", timestamp: Number.NaN, epoch: 10n },
	])(
		"a chunk whose time is none is not sent: $name",
		({ timestamp, epoch }) => {
			expect(first(timestamp, epoch)).toEqual({
				type: "skipped",
				because: "no-time",
			});
		},
	);

	test("the payload is the data of the chunk, the same array", () => {
		const chunk = chunkOf("k");
		const [step] = createVideoSender(OPTIONS).push(chunk);
		expect(step?.type === "object" && step.payload).toBe(chunk.data);
	});

	test.each<{ name: string; options: VideoSenderOptions }>([
		{ name: "a Group below 0", options: { firstGroup: -1n, epoch: 0n } },
		{
			name: "a Group over 2^64 - 1",
			options: { firstGroup: 18446744073709551616n, epoch: 0n },
		},
		{ name: "an epoch below 0", options: { firstGroup: 0n, epoch: -1n } },
		{
			name: "an epoch over 2^53 - 1",
			options: { firstGroup: 0n, epoch: 9007199254740992n },
		},
		{
			name: "a length of 0",
			options: { firstGroup: 0n, epoch: 0n, maxPayloadLength: 0 },
		},
		{
			name: "a length that is a fraction",
			options: { firstGroup: 0n, epoch: 0n, maxPayloadLength: 1.5 },
		},
	])("$name is a RangeError", ({ options }) => {
		expect(() => createVideoSender(options)).toThrow(RangeError);
	});

	test("the last Group there is can be sent; a key chunk after it is not, takes nothing, and ends that Group like any chunk that is not sent", () => {
		const sender = createVideoSender({
			firstGroup: 18446744073709551615n,
			epoch: 0n,
			mapping: "stream-per-group",
		});
		expect(
			["k", "d", "k", "d", "k"].map((token) =>
				sender.push(chunkOf(token)).map(word).join("; "),
			),
		).toEqual([
			"18446744073709551615,0 s0 open",
			"18446744073709551615,1 s0",
			"close18446744073709551615; skip:no-group-id",
			"skip:no-key-frame",
			"skip:no-group-id",
		]);
	});

	test.each<{ name: string; data: number[]; sent: boolean }>([
		{
			name: "a start code of 4 bytes and a NAL unit",
			data: [0, 0, 0, 1, 0x65],
			sent: true,
		},
		{
			name: "a first byte that is not 0",
			data: [1, 0, 0, 1, 0x65],
			sent: false,
		},
		{
			name: "a second byte that is not 0",
			data: [0, 1, 0, 1, 0x65],
			sent: false,
		},
		{
			name: "a third byte that is not 0",
			data: [0, 0, 1, 1, 0x65],
			sent: false,
		},
		{
			name: "a fourth byte that is not 1",
			data: [0, 0, 0, 2, 0x65],
			sent: false,
		},
		{
			name: "a start code with nothing after it",
			data: [0, 0, 0, 1],
			sent: false,
		},
		{ name: "three bytes", data: [0, 0, 0], sent: false },
		{ name: "no bytes", data: [], sent: false },
	])("the data of a chunk: $name", ({ data, sent }) => {
		const [step] = createVideoSender({ firstGroup: 0n, epoch: 0n }).push({
			type: "key",
			timestamp: 0,
			data: new Uint8Array(data),
		});
		expect(step?.type === "object" ? "sent" : step).toEqual(
			sent ? "sent" : { type: "skipped", because: "not-annex-b" },
		);
	});

	test("without a length given, a payload of 1,000,000 bytes is sent and one byte more is not", () => {
		expect(DEFAULT_MAX_PAYLOAD_LENGTH).toBe(1000000);
		const data = new Uint8Array(1000001);
		data.set([0, 0, 0, 1, 0x65]);
		const sender = createVideoSender({ firstGroup: 0n, epoch: 0n });
		expect(
			sender.push({
				type: "key",
				timestamp: 0,
				data: data.subarray(0, 1000000),
			})[0]?.type,
		).toBe("object");
		// The stream of the Group before is finished first.
		expect(sender.push({ type: "key", timestamp: 0, data })).toEqual([
			{ type: "close", groupId: 0n, subgroupId: 0n },
			{ type: "skipped", because: "too-large" },
		]);
	});

	test.each<{ name: string; json: string }>([
		{ name: "a first Group that is a number", json: '{"firstGroup":5}' },
		{ name: "an epoch that is a number", json: '{"epoch":5}' },
		{ name: "an epoch that is no number", json: '{"epoch":null}' },
	])("from a caller without types, $name is a RangeError", ({ json }) => {
		const options: VideoSenderOptions = Object.assign(
			{ firstGroup: 0n, epoch: 0n },
			JSON.parse(json),
		);
		expect(() => createVideoSender(options)).toThrow(RangeError);
	});

	test("from a caller without types, `endGroup` with no reason resets the stream: a FIN would say that every Object was written", () => {
		const sender = createVideoSender({
			...OPTIONS,
			mapping: "stream-per-group",
		});
		sender.push(chunkOf("k"));
		// What `endGroup()` without an argument hands in.
		const none: "track-ended" = JSON.parse("null");
		expect(sender.endGroup(none).map(word)).toEqual(["reset5"]);
	});

	test("the mapping without a setting is a stream for every Group", () => {
		expect(DEFAULT_MAPPING).toBe("stream-per-group");
	});

	test("a mapping that is none is a RangeError", () => {
		// What a caller without types can hand in.
		const options: VideoSenderOptions = Object.assign(
			{ firstGroup: 0n, epoch: 0n },
			JSON.parse('{"mapping":"stream-per-frame"}'),
		);
		expect(() => createVideoSender(options)).toThrow(RangeError);
	});
});

describe("what the sender writes, the receiver of this package reads", () => {
	/** The Objects a step list has, as a subscription would deliver them. */
	const objectsOf = (steps: VideoSenderStep[]): VideoObject[] =>
		steps.flatMap((step) =>
			step.type === "object"
				? [
						{
							groupId: step.groupId,
							object: {
								objectId: step.objectId,
								properties: step.properties,
								status: "Normal",
								payload: step.payload,
							},
						},
					]
				: [],
		);

	const frameWord = (event: VideoReceiverEvent): string =>
		event.type === "frame"
			? `${event.group},${event.object}${event.key ? "k" : ""}@${event.timestamp} ${Array.from(event.data).join(".")}`
			: `${event.type} ${event.group},${event.object}`;

	/** Two Groups of three frames, 40 ms apart, the data of each frame its own. */
	const chunks: VideoChunk[] = [
		{ type: "key", timestamp: 0, data: new Uint8Array([0, 0, 0, 1, 0x65, 1]) },
		{
			type: "delta",
			timestamp: 40000,
			data: new Uint8Array([0, 0, 0, 1, 0x41, 2]),
		},
		{
			type: "delta",
			timestamp: 80000,
			data: new Uint8Array([0, 0, 0, 1, 0x41, 3]),
		},
		{
			type: "key",
			timestamp: 120000,
			data: new Uint8Array([0, 0, 0, 1, 0x65, 4]),
		},
		{
			type: "delta",
			timestamp: 160000,
			data: new Uint8Array([0, 0, 0, 1, 0x41, 5]),
		},
		{
			type: "delta",
			timestamp: 200000,
			data: new Uint8Array([0, 0, 0, 1, 0x41, 6]),
		},
	];
	const FRAMES = [
		"5,0k@1000000 0.0.0.1.101.1",
		"5,1@1040000 0.0.0.1.65.2",
		"5,2@1080000 0.0.0.1.65.3",
		"6,0k@1120000 0.0.0.1.101.4",
		"6,1@1160000 0.0.0.1.65.5",
		"6,2@1200000 0.0.0.1.65.6",
	];

	test.each<{ name: string; mapping: VideoStreamMapping; order: number[] }>([
		{
			name: "a stream for every Group, in order",
			mapping: "stream-per-group",
			order: [0, 1, 2, 3, 4, 5],
		},
		{
			name: "a stream for every Object, in order",
			mapping: "stream-per-object",
			order: [0, 1, 2, 3, 4, 5],
		},
		{
			name: "a stream for every Object, each Group's Objects in another order",
			mapping: "stream-per-object",
			order: [0, 2, 1, 4, 5, 3],
		},
	])(
		"$name: every frame comes out, in order, with its time and its data",
		({ mapping, order }) => {
			const sender = createVideoSender({
				firstGroup: 5n,
				epoch: 1000000n,
				mapping,
			});
			const sent = objectsOf(chunks.flatMap((chunk) => sender.push(chunk)));
			const receiver = createVideoReceiver();
			const got = order.flatMap((index) => {
				const object = sent[index];
				if (object === undefined) throw new Error(`no Object ${index}`);
				return receiver.push(object);
			});
			expect(got.map(frameWord)).toEqual(FRAMES);
		},
	);

	test("two chunks of Chromium's encoder: each is sent as it is, the key chunk as Object 0 of a Group and the other after it, and the receiver hands both on unchanged", () => {
		const key = capturedChunk("key");
		const delta = capturedChunk("delta");
		const sender = createVideoSender({ firstGroup: 7n, epoch: 1000000n });
		// The times the encoder gave them, of a source that began at 5 seconds.
		const steps = [
			...sender.push({ type: "key", timestamp: 5000000, data: key }),
			...sender.push({ type: "delta", timestamp: 5040000, data: delta }),
		];
		expect(steps.map((step) => step.type)).toEqual(["object", "object"]);
		const sent = objectsOf(steps);
		expect(sent.map((object) => object.object.payload)).toEqual([
			capturedChunk("key"),
			capturedChunk("delta"),
		]);
		const receiver = createVideoReceiver();
		const frames = sent.flatMap((object) => receiver.push(object));
		expect(
			frames.map((event) =>
				event.type === "frame"
					? `${event.group},${event.object}${event.key ? "k" : ""}@${event.timestamp} ${event.data.length} bytes`
					: event.type,
			),
		).toEqual(["7,0k@6000000 229 bytes", "7,1@6040000 92 bytes"]);
		expect(
			frames.map((event) => (event.type === "frame" ? event.data : undefined)),
		).toEqual([capturedChunk("key"), capturedChunk("delta")]);
	});

	test("the frames of moqlivemock's publisher, re-sent: the receiver gives the same for the Objects of the sender as for the ones that were captured", () => {
		// The captured Objects, as @xmoq/moqt decodes their stream: Group
		// 1791690527, at 1791690527000000 and 40 ms later.
		const captured = capturedObjects();
		const direct = createVideoReceiver();
		const fromCapture = captured.flatMap((object) => direct.push(object));
		// The same frames as an encoder would give them, through the sender.
		const sender = createVideoSender({
			firstGroup: 1791690527n,
			epoch: 1791690527000000n,
		});
		const resent = createVideoReceiver();
		const times = [0, 40000];
		const fromSender = captured.flatMap(({ object }, index) => {
			const annexB = toAnnexB(object.payload);
			if (annexB.type !== "ok") throw new Error(annexB.reason);
			return objectsOf(
				sender.push({
					type: index === 0 ? "key" : "delta",
					timestamp: times[index] ?? Number.NaN,
					data: annexB.payload,
				}),
			).flatMap((sent) => resent.push(sent));
		});
		expect(fromSender.map((event) => event.type)).toEqual(["frame", "frame"]);
		expect(fromSender).toEqual(fromCapture);
	});
});
