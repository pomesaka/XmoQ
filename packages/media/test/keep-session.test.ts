import { expect, test } from "bun:test";
import { keepSession } from "../src/index.ts";

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
