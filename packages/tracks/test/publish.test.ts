import { expect, test } from "bun:test";
import {
	type PeerFetch,
	type PeerSubscribe,
	type PublishEnd,
	PublishEndedError,
	type PublishedTrack,
	type SubscribeAnswer,
} from "@xmoq/moqt";
import { publishTrack, tellEnd, wholeNumber } from "../src/publish.ts";

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
const NAMESPACE = [utf8("xmoq"), utf8("demo")];

const turns = async (): Promise<void> => {
	for (let turn = 0; turn < 10; turn++) await Promise.resolve();
};

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

/**
 * A subscription that `done` notes in `log`, and fails with `fails` where
 * given; `end()` ends it as its subscriber would.
 */
function subscription(
	log: string[],
	fails?: Error,
): { track: PublishedTrack; end(): void } {
	const { promise: ended, resolve } = Promise.withResolvers<PublishEnd>();
	return {
		track: {
			requestId: 1n,
			trackAlias: 0n,
			parameters: {},
			ended,
			onNewGroupRequest: () => {},
			openSubgroup: () => Promise.reject(new Error("no stream in this test")),
			done: async (code) => {
				if (fails !== undefined) throw fails;
				log.push(`PUBLISH_DONE ${code}`);
				return { type: "fin" };
			},
		},
		end: () => resolve({ type: "peer-closed" }),
	};
}

/**
 * A SUBSCRIBE for the track, answered as `how` says: at once, once
 * `answerOut` is called, with a `RangeError` (an answer that cannot be
 * sent), or gone before its answer.
 */
function subscribeRequest(
	log: string[],
	how: "accepted" | "later" | "unsendable" | "gone",
	trackName = "t",
): { request: PeerSubscribe; answerOut(): void; ended: () => void } {
	const { promise: out, resolve: answerOut } = Promise.withResolvers<void>();
	const signal = new AbortController();
	const accepted = subscription(log);
	const request: PeerSubscribe = {
		requestId: 1n,
		trackNamespace: NAMESPACE,
		trackName: utf8(trackName),
		parameters: {},
		signal: signal.signal,
		accept: async (answer) => {
			if (how === "later") await out;
			if (how === "unsendable")
				throw new RangeError("a value that does not fit");
			if (how === "gone") {
				signal.abort();
				throw new Error("the request went");
			}
			log.push(
				`SUBSCRIBE_OK${answer?.parameters?.largestObject === undefined ? "" : ` largest ${answer.parameters.largestObject.group}`}`,
			);
			return accepted.track;
		},
		reject: async (code) => {
			log.push(`REQUEST_ERROR ${code}`);
		},
	};
	return { request, answerOut, ended: accepted.end };
}

function fetchRequest(log: string[]): PeerFetch {
	return {
		requestId: 3n,
		trackNamespace: NAMESPACE,
		trackName: utf8("t"),
		startLocation: { group: 0n, object: 0n },
		endLocation: { group: 1n, object: 0n },
		groupOrder: "Ascending",
		fillTimeout: undefined,
		parameters: {},
		signal: new AbortController().signal,
		accept: () => Promise.reject(new Error("not served in this test")),
		reject: async (code) => {
			log.push(`REQUEST_ERROR ${code}`);
		},
	};
}

type Step =
	| "subscribe"
	| "subscribe-later"
	| "subscribe-unsendable"
	| "subscribe-gone"
	| "subscribe-other"
	| "answer-out"
	| "subscriber-ends"
	| "fetch"
	| "end";

test.each<{
	name: string;
	subscribers: "one" | "many";
	steps: Step[];
	/** What was sent, and what the caller was handed, in order. */
	log: string[];
	reports: number;
}>([
	{
		name: "a SUBSCRIBE is accepted with the answer of the caller, and handed on",
		subscribers: "one",
		steps: ["subscribe"],
		log: ["SUBSCRIBE_OK largest 7", "subscribed"],
		reports: 0,
	},
	{
		name: "a SUBSCRIBE for another track is not taken",
		subscribers: "one",
		steps: ["subscribe-other"],
		log: ["not taken"],
		reports: 0,
	},
	{
		name: "one: a second SUBSCRIBE is refused with DUPLICATE_SUBSCRIPTION",
		subscribers: "one",
		steps: ["subscribe", "subscribe"],
		log: ["SUBSCRIBE_OK largest 7", "subscribed", "REQUEST_ERROR 25"],
		reports: 0,
	},
	{
		name: "one: so is one while the first is being answered",
		subscribers: "one",
		steps: ["subscribe-later", "subscribe"],
		log: ["REQUEST_ERROR 25"],
		reports: 0,
	},
	{
		name: "one: a SUBSCRIBE after the first ended is accepted",
		subscribers: "one",
		steps: ["subscribe", "subscriber-ends", "subscribe"],
		log: [
			"SUBSCRIBE_OK largest 7",
			"subscribed",
			"SUBSCRIBE_OK largest 7",
			"subscribed",
		],
		reports: 0,
	},
	{
		name: "many: every SUBSCRIBE is accepted",
		subscribers: "many",
		steps: ["subscribe", "subscribe"],
		log: [
			"SUBSCRIBE_OK largest 7",
			"subscribed",
			"SUBSCRIBE_OK largest 7",
			"subscribed",
		],
		reports: 0,
	},
	{
		name: "an answer that cannot be sent is refused with INTERNAL_ERROR, and reported",
		subscribers: "one",
		steps: ["subscribe-unsendable"],
		log: ["REQUEST_ERROR 0"],
		reports: 1,
	},
	{
		name: "one: a SUBSCRIBE after an answer that could not be sent is accepted",
		subscribers: "one",
		steps: ["subscribe-unsendable", "subscribe"],
		log: ["REQUEST_ERROR 0", "SUBSCRIBE_OK largest 7", "subscribed"],
		reports: 1,
	},
	{
		name: "a request that went before its answer is not answered",
		subscribers: "one",
		steps: ["subscribe-gone"],
		log: [],
		reports: 0,
	},
	{
		name: "a FETCH is refused with NOT_SUPPORTED where the caller answers none",
		subscribers: "one",
		steps: ["fetch"],
		log: ["REQUEST_ERROR 3"],
		reports: 0,
	},
	{
		name: "after the end, a SUBSCRIBE and a FETCH are refused with DOES_NOT_EXIST",
		subscribers: "many",
		steps: ["end", "subscribe", "fetch"],
		log: ["REQUEST_ERROR 16", "REQUEST_ERROR 16"],
		reports: 0,
	},
	{
		name: "a SUBSCRIBE answered after the end gets PUBLISH_DONE, and is not handed on",
		subscribers: "one",
		steps: ["subscribe-later", "end", "answer-out"],
		log: ["SUBSCRIBE_OK largest 7", "PUBLISH_DONE 2", "end resolved"],
		reports: 0,
	},
])("$name", async ({ subscribers, steps, log, reports }) => {
	const seen: string[] = [];
	const published = publishTrack({
		trackNamespace: NAMESPACE,
		trackName: utf8("t"),
		subscribers,
		answer: (): SubscribeAnswer => ({
			parameters: { largestObject: { group: 7n, object: 0n } },
		}),
		subscribed: () => {
			seen.push("subscribed");
		},
	});
	let later: ReturnType<typeof subscribeRequest> | undefined;
	let last: ReturnType<typeof subscribeRequest> | undefined;
	let ending: Promise<void> | undefined;
	const errors = await reported(async () => {
		for (const step of steps) {
			switch (step) {
				case "subscribe":
					last = subscribeRequest(seen, "accepted");
					published.subscribe(last.request);
					break;
				case "subscribe-later":
					later = subscribeRequest(seen, "later");
					published.subscribe(later.request);
					break;
				case "subscribe-unsendable":
					published.subscribe(subscribeRequest(seen, "unsendable").request);
					break;
				case "subscribe-gone":
					published.subscribe(subscribeRequest(seen, "gone").request);
					break;
				case "subscribe-other":
					if (
						!published.subscribe(
							subscribeRequest(seen, "accepted", "u").request,
						)
					)
						seen.push("not taken");
					break;
				case "answer-out":
					later?.answerOut();
					await ending;
					seen.push("end resolved");
					break;
				case "subscriber-ends":
					last?.ended();
					break;
				case "fetch":
					published.fetch(fetchRequest(seen));
					break;
				case "end":
					ending = published.end();
					break;
			}
			await turns();
		}
	});
	expect({ seen, reports: errors.length }).toEqual({ seen: log, reports });
});

test.each([
	["a PUBLISH_DONE that is sent", undefined, ["PUBLISH_DONE 2"], 0],
	[
		"one the subscriber made needless",
		new PublishEndedError({ type: "peer-closed" }),
		[],
		0,
	],
	[
		"one that fails otherwise is reported",
		new Error("the stream broke"),
		[],
		1,
	],
])("tellEnd: %s", async (_case, fails, log, reports) => {
	const seen: string[] = [];
	const errors = await reported(() => tellEnd(subscription(seen, fails).track));
	expect({ seen, reports: errors.length }).toEqual({ seen: log, reports });
});

test.each([
	["the least", 1, 1, undefined],
	["below the least", 0, 1, RangeError],
	["a fraction", 1.5, 1, RangeError],
	["no number", Number.NaN, 0, RangeError],
	["longer than a timer takes", 2 ** 31, 0, RangeError],
])("wholeNumber: %s", (_case, value, least, error) => {
	if (error === undefined) expect(wholeNumber("n", value, least)).toBe(value);
	else expect(() => wholeNumber("n", value, least)).toThrow(error);
});
