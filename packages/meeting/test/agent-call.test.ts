import { expect, test } from "bun:test";
import { createAgentCall } from "../src/core/agent-call.ts";

/** What happens, in order: a press, or the call under way settling. */
type Step = "press" | "succeeds" | "fails";

test.each<{ name: string; steps: Step[]; log: string[] }>([
	{
		name: "a press calls, and a press when it is on stops it",
		steps: ["press", "succeeds", "press"],
		log: ["calling", "start", "on", "stop", "idle"],
	},
	{
		name: "a press while the call is being made does nothing",
		steps: ["press", "press", "press", "succeeds"],
		log: ["calling", "start", "on"],
	},
	{
		name: "a call that fails can be made again",
		steps: ["press", "fails", "press", "succeeds"],
		log: ["calling", "start", "failed", "calling", "start", "on"],
	},
])("$name", async ({ steps, log }) => {
	const seen: string[] = [];
	let settle: { resolve(): void; reject(): void } | undefined;
	const call = createAgentCall({
		start() {
			seen.push("start");
			return new Promise((resolve, reject) => {
				settle = {
					resolve: () => resolve(() => seen.push("stop")),
					reject: () => reject(new Error("closed the picker")),
				};
			});
		},
		onState: (state) => seen.push(state.type),
	});
	for (const step of steps) {
		if (step === "press") call.press();
		if (step === "succeeds") settle?.resolve();
		if (step === "fails") settle?.reject();
		await Promise.resolve();
		await Promise.resolve();
	}
	expect(seen).toEqual(log);
});
