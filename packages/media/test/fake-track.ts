import { microsecondsOf, readLocTiming } from "@xmoq/loc";
import {
	type KeyValuePair,
	type MessageParameters,
	type PeerFetch,
	type PeerSubscribe,
	type PublishEnd,
	PublishEndedError,
	type PublishedTrack,
	SubgroupClosedError,
	type SubgroupWriter,
} from "@xmoq/moqt";

// A subscription and its requests made by hand, which note what is done to
// them. The run through a relay, in a browser, is in interop/.

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);

/** A subscription of the peer that was accepted, and what was done on it, in lines. */
export interface FakeTrack {
	track: PublishedTrack;
	log: string[];
	/** Ends the subscription as its subscriber would. */
	end(how: PublishEnd): void;
	/** With a promise: every write waits for it. */
	holdWrites(until: Promise<void>): void;
	/** With a promise: every `openSubgroup` waits for it, its stream open from the call on. */
	holdOpens(until: Promise<void>): void;
	/**
	 * How many turns of the microtask queue a `reset()` takes to resolve,
	 * and how many a write that was under way takes to fail after it. The
	 * stream is reset from the call on, whatever these are.
	 */
	delay(turns: { reset: number; rejection: number }): void;
	/** The next write fails with this. */
	failNextWrite(error: Error): void;
	/** The next `close` of a stream fails with this, and leaves the stream open. */
	failNextClose(error: Error): void;
	/** The next `openSubgroup` fails with this. */
	failNextOpen(error: Error): void;
	/**
	 * The subscriber sends a NEW_GROUP_REQUEST. As the real one, a request
	 * that comes before a handler is named is kept for it, the last one.
	 */
	askForNewGroup(group: bigint): void;
}

const turns = async (count: number): Promise<void> => {
	for (let turn = 0; turn < count; turn++) await Promise.resolve();
};

/**
 * Refuses what `PublishedTrack` of @xmoq/moqt refuses (read in
 * `packages/moqt/src/webtransport/connection.ts`, `onPublish` and
 * `subgroupWriter`, and `packages/moqt/src/core/requests.ts`,
 * `publishDone`):
 *
 * - a subgroup or a write on a subscription that is over
 *   (`PublishEndedError`), a write that is under way when it ends among
 *   them; `close` and `reset` do nothing then, and its streams are gone;
 * - a write to a stream that was closed or reset (`SubgroupClosedError`);
 * - an Object whose ID is not above the one before it on its stream (the
 *   session's encoder throws for it);
 * - PUBLISH_DONE while a subgroup stream is neither closed nor reset (an
 *   `Error`, and nothing is sent). **A stream counts as open from the call
 *   of `openSubgroup` on**, before its promise resolves, and as reset from
 *   the call of `reset()` on: the session is called at once, and the
 *   promise waits for the transport.
 *
 * The real one lets 16 writes of a stream be under way; one at a time is
 * what the publisher is to do, so a second one is noted in the lines.
 * **More lenient than the real one**: `done` resolves at once, where the
 * real one waits for the subscriber to end its stream.
 */
export function fakeTrack(): FakeTrack {
	const log: string[] = [];
	let end: (how: PublishEnd) => void = () => {};
	const ended = new Promise<PublishEnd>((resolve) => {
		end = resolve;
	});
	let held: Promise<void> = Promise.resolve();
	let heldOpens: Promise<void> = Promise.resolve();
	let delays = { reset: 0, rejection: 0 };
	let writeFailure: Error | undefined;
	let closeFailure: Error | undefined;
	let openFailure: Error | undefined;
	let over: PublishEnd | undefined;
	let open = 0;
	let newGroupHandler: ((group: bigint) => void) | undefined;
	let keptRequest: bigint | undefined;
	const finish = (how: PublishEnd): void => {
		over = how;
		open = 0;
		end(how);
	};
	const track: PublishedTrack = {
		requestId: 1n,
		trackAlias: 0n,
		parameters: {},
		ended,
		onNewGroupRequest: (handler) => {
			newGroupHandler = handler;
			if (keptRequest === undefined) return;
			const group = keptRequest;
			keptRequest = undefined;
			handler(group);
		},
		openSubgroup: async (options) => {
			if (over !== undefined) throw new PublishEndedError(over);
			if (openFailure !== undefined) {
				const error = openFailure;
				openFailure = undefined;
				throw error;
			}
			const mode =
				options.subgroupId.mode === "Explicit"
					? `explicit ${options.subgroupId.value}`
					: options.subgroupId.mode;
			const stream = `group ${options.groupId} ${mode}${options.endOfGroup ? " end-of-group" : ""}${options.properties ? "" : " no-properties"}`;
			log.push(`${stream}: opened`);
			open += 1;
			let state: "open" | "closed" | "reset" = "open";
			let busy = false;
			let last: bigint | undefined;
			let stopped: () => void = () => {};
			const reset = new Promise<void>((resolve) => {
				stopped = resolve;
			});
			const isOpen = (): boolean => state === "open";
			const writer: SubgroupWriter = {
				write: async (object) => {
					if (over !== undefined) throw new PublishEndedError(over);
					if (!isOpen()) throw new SubgroupClosedError("not open");
					if (last !== undefined && object.objectId <= last) {
						throw new Error(
							`Object ${object.objectId} after Object ${last} on one stream`,
						);
					}
					last = object.objectId;
					if (busy) {
						log.push(`${stream}: a write before the one before it was taken`);
					}
					if (writeFailure !== undefined) {
						const error = writeFailure;
						writeFailure = undefined;
						throw error;
					}
					busy = true;
					await Promise.race([held, reset, ended]);
					busy = false;
					if (over !== undefined) throw new PublishEndedError(over);
					if (!isOpen()) {
						await turns(delays.rejection);
						throw new SubgroupClosedError("reset");
					}
					log.push(
						`${stream}: object ${object.objectId} ${object.payload.length} bytes ${propertiesOf(object.properties ?? [])}`,
					);
				},
				close: async () => {
					if (over !== undefined || !isOpen()) return;
					if (closeFailure !== undefined) {
						const error = closeFailure;
						closeFailure = undefined;
						log.push(`${stream}: close failed`);
						throw error;
					}
					state = "closed";
					open -= 1;
					log.push(`${stream}: closed`);
				},
				reset: async () => {
					if (over !== undefined || !isOpen()) return;
					state = "reset";
					open -= 1;
					stopped();
					log.push(`${stream}: reset`);
					await turns(delays.reset);
				},
			};
			await heldOpens;
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
			finish({ type: "done" });
			return { type: "fin" };
		},
	};
	return {
		track,
		log,
		end: finish,
		holdWrites: (until) => {
			held = until;
		},
		holdOpens: (until) => {
			heldOpens = until;
		},
		delay: (by) => {
			delays = by;
		},
		failNextWrite: (error) => {
			writeFailure = error;
		},
		failNextClose: (error) => {
			closeFailure = error;
		},
		failNextOpen: (error) => {
			openFailure = error;
		},
		askForNewGroup: (group) => {
			if (newGroupHandler === undefined) keptRequest = group;
			else newGroupHandler(group);
		},
	};
}

/** The time the properties of an Object say, as LOC is read here: the log names no number of a property. */
const propertiesOf = (properties: KeyValuePair[]): string => {
	const read = readLocTiming(properties);
	return read.type === "ok" && read.timing !== undefined
		? `at ${microsecondsOf(read.timing)}`
		: "without a time";
};

/** A SUBSCRIBE of the peer, and how it was answered. */
export function fakeSubscribe(
	namespace: string[],
	trackName: string,
	options: {
		parameters?: MessageParameters;
		acceptFails?: Error;
		/** The answer is out when this settles; at once when left out. */
		acceptWaits?: Promise<void>;
	} = {},
): { request: PeerSubscribe; answers: string[]; accepted: FakeTrack } {
	const answers: string[] = [];
	const accepted = fakeTrack();
	const request: PeerSubscribe = {
		requestId: 1n,
		trackNamespace: namespace.map(utf8),
		trackName: utf8(trackName),
		parameters: options.parameters ?? {},
		signal: new AbortController().signal,
		accept: async (answer) => {
			await options.acceptWaits;
			if (options.acceptFails !== undefined) throw options.acceptFails;
			const largest = answer?.parameters?.largestObject;
			const properties = (answer?.trackProperties ?? []).map(
				(property) => ` property ${property.type}=${property.value}`,
			);
			answers.push(
				`SUBSCRIBE_OK largest ${largest === undefined ? "none" : `{${largest.group}, ${largest.object}}`}${properties.join("")}`,
			);
			return accepted.track;
		},
		reject: async (code) => {
			answers.push(`REQUEST_ERROR ${code}`);
		},
	};
	return { request, answers, accepted };
}

/** A FETCH of the peer for a track, and how it was answered. */
export function fakeFetch(
	namespace: string[],
	trackName: string,
): { request: PeerFetch; answers: string[] } {
	const answers: string[] = [];
	const request: PeerFetch = {
		requestId: 3n,
		trackNamespace: namespace.map(utf8),
		trackName: utf8(trackName),
		startLocation: { group: 0n, object: 0n },
		endLocation: { group: 1n, object: 0n },
		groupOrder: "Ascending",
		fillTimeout: undefined,
		parameters: {},
		signal: new AbortController().signal,
		accept: async () => {
			answers.push("FETCH_OK");
			throw new Error("not served in these tests");
		},
		reject: async (code) => {
			answers.push(`REQUEST_ERROR ${code}`);
		},
	};
	return { request, answers };
}
