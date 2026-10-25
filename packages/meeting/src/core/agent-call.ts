// The one button that calls an agent and stops it: what a press does in
// each state. WHY a state of its own for the call that is under way: a
// call waits for the person (the picker, the permission) and for the
// folder, and a second press meanwhile would make a second writer of the
// same folder, which says each line of the inbox twice and cannot be
// stopped.

/** Where the call stands. */
export type AgentCallState =
	/** Nothing is written: a press calls. */
	| { type: "idle" }
	/** The call is being made: a press does nothing. */
	| { type: "calling" }
	/** Written into the folder: a press stops it. */
	| { type: "on" }
	/** The call failed: as `idle`, with why. */
	| { type: "failed"; error: unknown };

export interface AgentCallOptions {
	/** Makes the call; resolves with how to stop it. */
	start(): Promise<() => void>;
	onState?(state: AgentCallState): void;
}

export interface AgentCall {
	/** The button was pressed. */
	press(): void;
	state(): AgentCallState;
}

export function createAgentCall(options: AgentCallOptions): AgentCall {
	let state: AgentCallState = { type: "idle" };
	let stop: (() => void) | undefined;
	const become = (next: AgentCallState): void => {
		state = next;
		options.onState?.(next);
	};
	return {
		press() {
			switch (state.type) {
				case "calling":
					return;
				case "on":
					stop?.();
					stop = undefined;
					become({ type: "idle" });
					return;
				case "idle":
				case "failed":
					become({ type: "calling" });
					options.start().then(
						(stopping) => {
							stop = stopping;
							become({ type: "on" });
						},
						(error: unknown) => become({ type: "failed", error }),
					);
			}
		},
		state: () => state,
	};
}
