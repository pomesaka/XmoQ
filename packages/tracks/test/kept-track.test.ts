import { expect, jest, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	RequestAbortedError,
	RequestFailedError,
	type SessionClose,
	type Subscription,
	type SubscriptionEnd,
} from "@xmoq/moqt";
import { followTrack } from "../src/follow.ts";
import { keptTrackRules } from "../src/keep-track.ts";
import { stayOnSession } from "../src/stay-on-session.ts";

/**
 * The rows of `fixtures/kept-track/kept-track.tsv`, which `xmoq-subscriber`
 * reads too. Every row ends with one line feed, as its README says.
 */
function rows(): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"kept-track",
		"kept-track.tsv",
	);
	const table = readFileSync(path, "utf8");
	if (!table.endsWith("\n"))
		throw new Error(`${path}: no line feed at its end`);
	const read = table
		.slice(0, -1)
		.split("\n")
		.map((row) => row.split("\t"));
	if (read.some((row) => row.length !== 6)) {
		throw new Error(`${path}: a row of other than six columns`);
	}
	return read;
}

const FAILURES = ["not-sent", "not-answered"];

/**
 * The rows of a track asked for again however many times in a row (column
 * 5 `-`) and that its publisher is to publish (column 6 `required`). WHY
 * NOT the others: neither `keepTrackOnSession` nor `followTrack` gives a
 * track up after a number of times, or tells an optional track apart; the
 * gatherer of the chat does, by `kept::decide` of `xmoq-subscriber`.
 */
const table = rows().filter(
	([, , , , times, publisher]) => times === "-" && publisher === "required",
);
const stepsOf = (steps: string | undefined): string[] =>
	(steps ?? "").split(" ");

/**
 * The rows of `keepTrackOnSession`. WHY NOT the others: it gives up on a
 * failure (`keptTrackRules`), so the rows of `again` on a failure are of
 * `followTrack`, below; and a message that breaks the protocol closes the
 * session in `@xmoq/moqt`, which the kept track only sees end.
 */
const here = table.filter(
	([, onFailure, steps]) =>
		!steps?.split(" ").includes("violation") &&
		(onFailure === "give-up" ||
			!steps?.split(" ").some((step) => FAILURES.includes(step))),
);

test("the table has its rows of keepTrackOnSession", () => {
	expect(here.length).toBe(13);
});

/** What `play` rejects with, for a step that is no subscription. */
function failureOf(step: string): Error | undefined {
	const [type, code, ms] = step.split(":");
	switch (type) {
		case "refused":
			return new RequestFailedError({
				errorCode: BigInt(code ?? ""),
				// §10.6.2: the milliseconds plus one, and 0 for none named.
				retryInterval: ms === undefined ? 0n : BigInt(ms) + 1n,
				errorReason: "",
			});
		case "not-sent":
			return new RequestAbortedError({ type: "not-sent-in-time" });
		case "not-answered":
			return new RequestAbortedError({ type: "no-answer-in-time" });
		default:
			return undefined;
	}
}

test.each(here)("%s", async (_case, _onFailure, steps, becomes) => {
	const left = (steps ?? "").split(" ");
	const became: string[] = [];
	let stopped = false;
	const { promise: closed, resolve: close } = Promise.withResolvers<void>();
	try {
		await stayOnSession({
			async play() {
				const step = left.shift() ?? "";
				const failure = failureOf(step);
				if (failure !== undefined) throw failure;
				// Subscribed: the step after it, where there is one, ends it.
				const end = left.shift();
				if (end === undefined) close();
				return {
					ended:
						end === undefined
							? new Promise<never>(() => {})
							: Promise.resolve(end),
				};
			},
			closed,
			wait: async (ms) => {
				became.push(`again:${ms}`);
				if (left.length === 0) stopped = true;
			},
			stopped: () => stopped,
			...keptTrackRules,
			onPlaying: () => became.push("subscribed"),
		});
	} catch {
		became.push("given-up");
	}
	expect(became.join(" ")).toBe(becomes ?? "");
});

/** How a subscription of the table ends, as `@xmoq/moqt` tells it. */
const ENDS: Record<string, SubscriptionEnd> = {
	"publish-done": { type: "publish-done", statusCode: 0n, errorReason: "" },
	fin: { type: "peer-closed" },
	reset: { type: "peer-reset", code: 0 },
	malformed: { type: "malformed-track" },
};

/**
 * The rows of `followTrack`, which asks again after every end and every
 * failure: those of `again` on a failure. WHY NOT `lost`: a subscription
 * of `@xmoq/moqt` has no end for a stream lost while its session goes on.
 */
const followed = table.filter(
	([, onFailure, steps]) =>
		onFailure === "again" &&
		stepsOf(steps).every(
			(step) =>
				step === "subscribed" ||
				step in ENDS ||
				step === "goaway" ||
				step.startsWith("refused:") ||
				FAILURES.includes(step),
		),
);

test("the table has its rows of followTrack", () => {
	expect(followed.length).toBe(11);
});

/**
 * A subscription that ends at once as the step `end` says (`undefined`:
 * never). `goaway` resolves its `goAway`, and it ends `cancelled` once it is
 * cancelled, as one of `@xmoq/moqt` does.
 */
function subscription(end: string | undefined): Subscription {
	const { promise: cancelled, resolve: cancelledNow } =
		Promise.withResolvers<SubscriptionEnd>();
	let close: () => void = () => {};
	const told = end === undefined ? undefined : ENDS[end];
	return {
		requestId: 0n,
		trackAlias: 0n,
		parameters: {},
		trackProperties: [],
		objects: new ReadableStream({
			start(controller) {
				close = () => controller.close();
				if (told !== undefined) controller.close();
			},
		}),
		ended: told === undefined ? cancelled : Promise.resolve(told),
		goAway:
			end === "goaway"
				? Promise.resolve({ newSessionUri: "", timeoutMs: 0 })
				: new Promise(() => {}),
		cancel: async () => {
			close();
			cancelledNow({ type: "cancelled" });
		},
		update: () => Promise.reject(new Error("no update in this test")),
	};
}

const turns = async (): Promise<void> => {
	for (let turn = 0; turn < 20; turn++) await Promise.resolve();
};

test.each(followed)(
	"followTrack: %s",
	async (_case, _onFailure, steps, becomes) => {
		jest.useFakeTimers();
		const stop = new AbortController();
		try {
			const left = stepsOf(steps);
			const became: string[] = [];
			let asked = 0;
			/** Milliseconds since the step before: the pause it was asked again after. */
			let waited = 0;
			followTrack(
				{
					async subscribe() {
						if (asked > 0) became.push(`again:${waited}`);
						waited = 0;
						asked += 1;
						const step = left.shift();
						if (step === undefined) return new Promise<never>(() => {});
						const failure = failureOf(step);
						if (failure !== undefined) throw failure;
						became.push("subscribed");
						// Subscribed: the step after it, where there is one, ends it.
						const end = left.shift();
						return subscription(end);
					},
					fetch: () => Promise.reject(new Error("no FETCH in this test")),
					close() {},
					closed: new Promise<SessionClose>(() => {}),
				},
				{
					trackNamespace: [new TextEncoder().encode("test")],
					trackName: new TextEncoder().encode("items"),
					recent: 16,
					catchUp: 16,
					remembered: 16,
					maxPayloadLength: 1024,
					take() {},
					signal: stop.signal,
				},
			);
			await turns();
			// The pause after each step, a millisecond at a time, until the steps
			// are over and the track is asked for once more.
			while (left.length > 0 || asked <= stepsOf(steps).length) {
				const before = asked;
				while (asked === before && waited < 60_000) {
					// Before the clock moves: the timers that are due run their
					// microtasks within it, and the SUBSCRIBE with them.
					waited += 1;
					jest.advanceTimersByTime(1);
					await turns();
				}
				if (asked === before) break;
			}
			expect(became.join(" ")).toBe(becomes ?? "");
		} finally {
			stop.abort();
			jest.useRealTimers();
		}
	},
);
