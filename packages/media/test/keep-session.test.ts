import { expect, test } from "bun:test";
import { keepSession, stayOnSession } from "../src/index.ts";

test.each<{
	name: string;
	/** What each attempt does: "ends", "fails", or "stop" (the caller stops while it runs, and it ends). */
	attempts: string[];
	delayMs: number | undefined;
	log: string[];
}>([
	{
		name: "a session that ends is followed by the next, after the pause",
		attempts: ["ends", "stop"],
		delayMs: undefined,
		log: ["attempt 1", "ended 1", "wait 2000", "attempt 2", "ended 2"],
	},
	{
		name: "a session that cannot be made is tried again, after the same pause",
		attempts: ["fails", "fails", "ends", "stop"],
		delayMs: 500,
		log: [
			"attempt 1",
			"failed 1",
			"wait 500",
			"attempt 2",
			"failed 2",
			"wait 500",
			"attempt 3",
			"ended 3",
			"wait 500",
			"attempt 4",
			"ended 4",
		],
	},
	{
		name: "stopped in the first session, nothing is waited for",
		attempts: ["stop"],
		delayMs: undefined,
		log: ["attempt 1", "ended 1"],
	},
])("$name", async ({ attempts, delayMs, log }) => {
	const stop = { aborted: false };
	const seen: string[] = [];
	await keepSession({
		async run(attempt) {
			const does = attempts[attempt - 1];
			if (does === "fails") throw new Error("no relay");
			if (does === "stop") stop.aborted = true;
			return attempt;
		},
		async wait(ms) {
			seen.push(`wait ${ms}`);
		},
		...(delayMs === undefined ? {} : { delayMs }),
		stopped: () => stop.aborted,
		onAttempt: (attempt) => seen.push(`attempt ${attempt}`),
		onOutcome: (outcome) => seen.push(`${outcome.type} ${outcome.attempt}`),
	});
	expect(seen).toEqual(log);
});

test("stopped while it waits, no session follows", async () => {
	const stop = { aborted: false };
	const seen: string[] = [];
	await keepSession({
		async run(attempt) {
			seen.push(`run ${attempt}`);
		},
		async wait() {
			stop.aborted = true;
		},
		stopped: () => stop.aborted,
	});
	expect(seen).toEqual(["run 1"]);
});

test.each<{
	name: string;
	/**
	 * What each call of `play` does: "not-yet" and "fails" reject; "ends"
	 * plays and what is played ends; "closes" plays and the session ends;
	 * "stop" plays, and the caller stops while it does.
	 */
	plays: string[];
	log: string[];
	result: string;
}>([
	{
		name: "nothing to play yet is asked for again on the session, after the pause",
		plays: ["not-yet", "not-yet", "closes"],
		log: [
			"play",
			"not yet",
			"wait 1000",
			"play",
			"not yet",
			"wait 1000",
			"play",
			"playing 1",
		],
		result: "session-ended",
	},
	{
		name: "what was played and ended is asked for again on the session, after the pause",
		plays: ["ends", "ends", "closes"],
		log: [
			"play",
			"playing 1",
			"ended",
			"wait 1000",
			"play",
			"playing 2",
			"ended",
			"wait 1000",
			"play",
			"playing 3",
		],
		result: "session-ended",
	},
	{
		name: "a 'not yet' that names its pause is asked again after that long, and no sooner than 100 ms",
		plays: ["not-yet after 250", "not-yet", "not-yet after 0", "closes"],
		log: [
			"play",
			"not yet",
			"wait 250",
			"play",
			"not yet",
			"wait 1000",
			"play",
			"not yet",
			"wait 100",
			"play",
			"playing 1",
		],
		result: "session-ended",
	},
	{
		name: "a failure that is no 'not yet' is the caller's: nothing is asked again",
		plays: ["not-yet", "fails"],
		log: ["play", "not yet", "wait 1000", "play"],
		result: "rejected: broken",
	},
	{
		name: "a caller that stops while it plays is not asked again",
		plays: ["ends", "stop"],
		log: ["play", "playing 1", "ended", "wait 1000", "play", "playing 2"],
		result: "stopped",
	},
])("on one session: $name", async ({ plays, log, result }) => {
	const seen: string[] = [];
	let stopped = false;
	let close: (how: string) => void = () => {};
	const closed = new Promise<string>((resolve) => {
		close = resolve;
	});
	const steps = [...plays];
	const outcome = await stayOnSession<string, string>({
		play() {
			seen.push("play");
			const step = steps.shift();
			if (step?.startsWith("not-yet")) {
				return Promise.reject(new Error(step.replace("not-yet", "not yet")));
			}
			if (step === "fails") return Promise.reject(new Error("broken"));
			if (step === "closes") {
				queueMicrotask(() => close("gone"));
				return Promise.resolve({ ended: new Promise<string>(() => {}) });
			}
			if (step === "stop") {
				queueMicrotask(() => {
					stopped = true;
				});
			}
			return Promise.resolve({ ended: Promise.resolve("over") });
		},
		closed,
		notYet: (error) =>
			error instanceof Error && error.message.startsWith("not yet"),
		againAfter(error) {
			const named =
				error instanceof Error
					? /^not yet after (\d+)$/.exec(error.message)?.[1]
					: undefined;
			return named === undefined ? undefined : Number(named);
		},
		wait(ms) {
			seen.push(`wait ${ms}`);
			return Promise.resolve();
		},
		stopped: () => stopped,
		onPlaying: (times) => seen.push(`playing ${times}`),
		onNotYet: () => seen.push("not yet"),
		onEnded: () => seen.push("ended"),
	}).then(
		(over) => over.type,
		(error: unknown) =>
			`rejected: ${error instanceof Error ? error.message : String(error)}`,
	);
	expect({ seen, outcome }).toEqual({ seen: log, outcome: result });
});
