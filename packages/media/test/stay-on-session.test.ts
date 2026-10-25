import { expect, test } from "bun:test";
import { stayOnSession } from "../src/index.ts";

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
		name: "a 'not yet' that names its pause is asked again after that long, and one that names none after the pause of the caller",
		plays: ["not-yet after 250", "not-yet", "not-yet after 300", "closes"],
		log: [
			"play",
			"not yet",
			"wait 250",
			"play",
			"not yet",
			"wait 1000",
			"play",
			"not yet",
			"wait 300",
			"play",
			"playing 1",
		],
		result: "session-ended",
	},
	{
		name: "a peer that says 'at once' again and again is asked after 100 ms, then twice as long each time, up to 5 s",
		plays: [
			"not-yet after 0",
			"not-yet after 0",
			"not-yet after 0",
			"not-yet after 0",
			"not-yet after 0",
			"not-yet after 0",
			"not-yet after 0",
			"not-yet after 0",
			"closes",
		],
		log: [
			"play",
			"not yet",
			"wait 100",
			"play",
			"not yet",
			"wait 200",
			"play",
			"not yet",
			"wait 400",
			"play",
			"not yet",
			"wait 800",
			"play",
			"not yet",
			"wait 1600",
			"play",
			"not yet",
			"wait 3200",
			"play",
			"not yet",
			"wait 5000",
			"play",
			"not yet",
			"wait 5000",
			"play",
			"playing 1",
		],
		result: "session-ended",
	},
	{
		name: "the shortest pause is 100 ms again once something was played",
		plays: [
			"not-yet after 0",
			"not-yet after 0",
			"ends",
			"not-yet after 0",
			"closes",
		],
		log: [
			"play",
			"not yet",
			"wait 100",
			"play",
			"not yet",
			"wait 200",
			"play",
			"playing 1",
			"ended",
			"wait 1000",
			"play",
			"not yet",
			"wait 100",
			"play",
			"playing 2",
		],
		result: "session-ended",
	},
	{
		name: "a pause the peer names is 30 s at most",
		plays: ["not-yet after 30001", "not-yet after 2147483647", "closes"],
		log: [
			"play",
			"not yet",
			"wait 30000",
			"play",
			"not yet",
			"wait 30000",
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
