import type { SessionEvent } from "../core/session-output.ts";

/**
 * Hooks into the inside of a connection, for logging and for capturing byte
 * fixtures. What they see is the session's internal vocabulary and can change
 * with it, which is why they are not part of the main entry of the package.
 *
 * Neither hook may throw: a throw ends the session.
 */
export interface Diagnostics {
	/** Called with every session event, before the connection acts on it. */
	onEvent?: (event: SessionEvent) => void;
	/** Called with the bytes of every stream. */
	onBytes?: (
		direction: "sent" | "received",
		stream: string,
		data: Uint8Array,
	) => void;
}
