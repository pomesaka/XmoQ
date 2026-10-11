import { describe, expect, test } from "bun:test";
import { locProperties } from "@xmoq/loc";
import {
	createVideoReceiver,
	createVideoSender,
	type VideoChunk,
	type VideoObject,
	type VideoReceiverEvent,
	type VideoSenderStep,
	type VideoStreamMapping,
} from "../src/index.ts";

// Sequences at random, the same in every run: what the two state machines
// keep to whatever comes, which the tables of the other tests cannot say.

/** Numbers from 0 to 1 that depend on the seed alone (mulberry32). */
function random(seed: number): () => number {
	let state = seed;
	return () => {
		state = (state + 0x6d2b79f5) | 0;
		let t = Math.imul(state ^ (state >>> 15), 1 | state);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

const pick = <T>(next: () => number, from: readonly T[]): T => {
	const one = from[Math.floor(next() * from.length)];
	if (one === undefined) throw new Error("nothing to pick from");
	return one;
};

const SEQUENCES = 2000;

describe("createVideoSender, sequences at random", () => {
	const GOOD = [0, 0, 0, 1, 0x65];
	const DATA: readonly number[][] = [
		GOOD,
		GOOD,
		GOOD,
		GOOD,
		GOOD,
		GOOD,
		[0, 0, 1, 0x65, 0x01],
		[0, 0, 0, 1],
		[],
		[0, 0, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8],
	];
	const TIMES: readonly number[] = [
		0,
		40000,
		80000,
		0,
		40000,
		Number.NaN,
		0.5,
		-1e18,
		Number.POSITIVE_INFINITY,
	];

	test.each<{ mapping: VideoStreamMapping; firstGroup: bigint }>([
		{ mapping: "stream-per-object", firstGroup: 5n },
		{ mapping: "stream-per-group", firstGroup: 5n },
		{ mapping: "stream-per-group", firstGroup: 18446744073709551613n },
		{ mapping: "stream-per-object", firstGroup: 18446744073709551615n },
	])(
		"$mapping from Group $firstGroup: Groups without a gap, Objects from 0, one stream open at most, and what is sent the receiver plays",
		({ mapping, firstGroup }) => {
			const next = random(Number(firstGroup % 1000n) + mapping.length);
			for (let sequence = 0; sequence < SEQUENCES; sequence++) {
				const sender = createVideoSender({
					firstGroup,
					epoch: 1000000n,
					mapping,
					maxPayloadLength: 8,
				});
				const receiver = createVideoReceiver();
				let group: bigint | undefined;
				let object = 0n;
				let sending = false;
				let open: bigint | undefined;
				const check = (steps: VideoSenderStep[], chunk?: VideoChunk): void => {
					const sent = steps.filter((step) => step.type === "object");
					const skipped = steps.filter((step) => step.type === "skipped");
					// (g) a chunk is one Object or one skip.
					if (chunk !== undefined) {
						expect(sent.length + skipped.length).toBe(1);
					} else {
						expect(sent.length + skipped.length).toBe(0);
					}
					for (const step of steps) {
						if (step.type === "skipped") {
							sending = false;
							// (d) a chunk that is not sent leaves no stream open.
							expect(open).toBeUndefined();
							continue;
						}
						if (step.type === "close" || step.type === "reset") {
							// (d) only a stream that is open is ended.
							expect(open).toBe(step.groupId);
							open = undefined;
							continue;
						}
						if (step.objectId === 0n) {
							// (a) Groups from the first, one after the other; (b)
							// Object 0 is a key chunk.
							expect(step.groupId).toBe(
								group === undefined ? firstGroup : group + 1n,
							);
							expect(chunk?.type).toBe("key");
							group = step.groupId;
							object = 0n;
							sending = true;
						}
						// (c) nothing after a chunk that was not sent, before a key
						// chunk; (b) Objects from 0, one after the other.
						expect(sending).toBe(true);
						expect(step.groupId).toBe(group ?? -1n);
						expect(step.objectId).toBe(object);
						object += 1n;
						if (mapping === "stream-per-object") {
							// (e) every Object opens and finishes its stream.
							expect([
								step.opensStream,
								step.closesStream,
								step.subgroupId,
							]).toEqual([true, true, step.objectId]);
						} else {
							// (d) no second stream for a Group, and no Object on a
							// stream that is not open.
							expect(step.closesStream).toBe(false);
							expect(step.opensStream).toBe(open === undefined);
							if (step.opensStream) open = step.groupId;
							expect(open).toBe(step.groupId);
						}
						// What is sent, the receiver hands on as a frame.
						const got = receiver.push({
							groupId: step.groupId,
							object: {
								objectId: step.objectId,
								properties: step.properties,
								status: "Normal",
								payload: step.payload,
							},
						});
						expect(got.map((event) => event.type)).toEqual(["frame"]);
					}
				};
				const length = 1 + Math.floor(next() * 24);
				for (let turn = 0; turn < length; turn++) {
					const roll = next();
					// (f) nothing throws.
					if (roll < 0.08) {
						sending = false;
						check(sender.endGroup("track-ended"));
					} else if (roll < 0.16) {
						sending = false;
						check(sender.endGroup("write-failed"));
					} else {
						const chunk: VideoChunk = {
							type: next() < 0.3 ? "key" : "delta",
							timestamp: pick(next, TIMES),
							data: new Uint8Array(pick(next, DATA)),
						};
						check(sender.push(chunk), chunk);
					}
				}
			}
		},
	);
});

describe("createVideoReceiver, sequences at random", () => {
	const PAYLOAD = new Uint8Array([0, 0, 0, 2, 0xaa, 0xbb]);
	const BROKEN = new Uint8Array([0, 0, 0, 9, 0xaa, 0xbb]);

	test.each<{ maxHeldObjects: number; maxHeldBytes: number }>([
		{ maxHeldObjects: 1, maxHeldBytes: 6 },
		{ maxHeldObjects: 3, maxHeldBytes: 40 },
		{ maxHeldObjects: 6, maxHeldBytes: 13 },
		{ maxHeldObjects: 64, maxHeldBytes: 4194304 },
	])(
		"holding $maxHeldObjects Objects and $maxHeldBytes bytes: frames in order and from Object 0 without a gap, nothing twice, nothing after an end",
		({ maxHeldObjects, maxHeldBytes }) => {
			const next = random(maxHeldObjects * 1000 + maxHeldBytes);
			for (let sequence = 0; sequence < SEQUENCES; sequence++) {
				const receiver = createVideoReceiver({ maxHeldObjects, maxHeldBytes });
				/** The frames so far, as "group,object". */
				const frames: string[] = [];
				let last: { group: bigint; object: bigint } | undefined;
				let trackEnd: { group: bigint; object: bigint } | undefined;
				/** Every Group up to this one was given up with `abandonGroup`, or was over then. */
				let givenUp: bigint | undefined;
				const pushed = new Map<string, number>();
				const accounted = new Map<string, number>();
				const check = (events: VideoReceiverEvent[]): void => {
					for (const event of events) {
						const at = `${event.group},${event.object}`;
						if (event.type === "end-of-track") {
							// An end of the track only ever moves to the front.
							const earlier =
								trackEnd === undefined ||
								event.group < trackEnd.group ||
								(event.group === trackEnd.group &&
									event.object < trackEnd.object);
							expect(earlier).toBe(true);
							trackEnd = { group: event.group, object: event.object };
							continue;
						}
						accounted.set(at, (accounted.get(at) ?? 0) + 1);
						if (event.type === "dropped") continue;
						// Nothing of a Group that was given up, or of one before it.
						if (givenUp !== undefined) {
							expect(event.group > givenUp).toBe(true);
						}
						// Ascending, and within a Group from 0 without a gap.
						if (last === undefined || event.group > last.group) {
							expect(event.object).toBe(0n);
						} else {
							expect(event.group).toBe(last.group);
							expect(event.object).toBe(last.object + 1n);
						}
						expect(event.key).toBe(event.object === 0n);
						// Nothing at or after where the track was said to end.
						if (trackEnd !== undefined) {
							const before =
								event.group < trackEnd.group ||
								(event.group === trackEnd.group &&
									event.object < trackEnd.object);
							expect(before).toBe(true);
						}
						last = { group: event.group, object: event.object };
						frames.push(at);
					}
				};
				const length = 1 + Math.floor(next() * 24);
				for (let turn = 0; turn < length; turn++) {
					const group = BigInt(Math.floor(next() * 4));
					const objectId = BigInt(Math.floor(next() * 5));
					const roll = next();
					const status =
						roll < 0.06 ? "EndOfGroup" : roll < 0.09 ? "EndOfTrack" : "Normal";
					const kind = next();
					const object: VideoObject = {
						groupId: group,
						object: {
							objectId,
							status,
							properties:
								kind < 0.05
									? []
									: kind < 0.1
										? [
												...locProperties({ timestamp: 1n }),
												...locProperties({ timestamp: 2n }),
											]
										: locProperties({ timestamp: 1000n }),
							payload:
								kind > 0.95 ? BROKEN : kind > 0.9 ? new Uint8Array() : PAYLOAD,
						},
					};
					if (status === "Normal") {
						const at = `${group},${objectId}`;
						pushed.set(at, (pushed.get(at) ?? 0) + 1);
					}
					if (next() < 0.05) {
						check(receiver.abandonGroup("decoder-failed"));
						// The Group that is handed on is the one of the last frame,
						// or a later one that has given no frame yet.
						if (last !== undefined) givenUp = last.group;
					}
					check(receiver.push(object));
				}
				// Everything that is held comes out, or is dropped, for a Group far
				// ahead.
				check(
					receiver.push({
						groupId: 1000n,
						object: {
							objectId: 0n,
							status: "Normal",
							properties: locProperties({ timestamp: 1000n }),
							payload: PAYLOAD,
						},
					}),
				);
				pushed.set("1000,0", 1);
				// No frame twice.
				expect(new Set(frames).size).toBe(frames.length);
				// An Object that only says an end is neither a frame nor dropped.
				// Of what was pushed, nothing became a frame or was dropped
				// more often than it came.
				for (const [at, times] of accounted) {
					expect(times).toBeLessThanOrEqual(pushed.get(at) ?? 0);
				}
				// Where the track was not said to end, the Object of the Group
				// far ahead began that Group, and nothing is held any more:
				// everything that was pushed is accounted for, once for each
				// time it came. (After an End of Track that Object is dropped,
				// and what was held before the end stays held.)
				if (trackEnd === undefined) {
					expect([...accounted].sort()).toEqual([...pushed].sort());
				}
			}
		},
	);
});
