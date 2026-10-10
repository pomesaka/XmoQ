import {
	createFetchStreamDecoder,
	createFetchStreamEncoder,
	type FetchStreamItem,
} from "../core/fetch.ts";
import { resolveLimits } from "../core/limits.ts";
import type { SubgroupStreamLimits } from "../core/subgroup.ts";
import type { Fetch, MoqtConnection } from "./connection.ts";

/** What a fetch stream holds after its header: Objects and End of Range indicators (§11.4.4). */
export type FetchItem = Exclude<FetchStreamItem, { type: "header" }>;

/**
 * `decodeFetchBody` fails with this for a response that cannot be read
 * further and breaks no rule that ends the session: a malformed Track
 * (§2.4.2, scope "track") or an Object over a limit of this endpoint (scope
 * "stream"). The FETCH was cancelled.
 */
export class FetchBodyError extends Error {
	override readonly name = "FetchBodyError";
	readonly scope: "track" | "stream";

	constructor(scope: "track" | "stream", reason: string) {
		super(reason);
		this.scope = scope;
	}
}

export interface DecodeFetchBodyOptions {
	/** Limits on one Object, in place of the defaults of a session. */
	limits?: Partial<SubgroupStreamLimits>;
}

/**
 * The Objects and End of Range indicators of a FETCH, decoded from its
 * `body`, which this takes over: read the one or the other.
 *
 * The stream is read only as fast as the result is. Cancelling the result
 * cancels the FETCH. It closes when the publisher finished the fetch stream
 * and fails with what `body` fails with. The Group Order, by which the
 * Group IDs of the stream count, is the one the FETCH asked for
 * (`Fetch.groupOrder`): the caller cannot give another. What was decoded
 * before a failure is delivered first. On top of that:
 *
 * | The bytes | What is done | The result fails with |
 * |---|---|---|
 * | break a rule the draft closes the session for (§11.4.4.1: Serialization Flags it does not define, a first Object that refers to a prior one; a stream that ends inside an Object) | the session is closed with PROTOCOL_VIOLATION | the `MoqtError` |
 * | make the Track malformed (§2.4.2), or have an Object over a limit | the FETCH is cancelled | `FetchBodyError` |
 */
export function decodeFetchBody(
	connection: Pick<MoqtConnection, "close">,
	fetch: Pick<Fetch, "requestId" | "groupOrder" | "body" | "cancel">,
	options: DecodeFetchBodyOptions = {},
): ReadableStream<FetchItem> {
	const { groupOrder } = fetch;
	const decoder = createFetchStreamDecoder(
		resolveLimits(options.limits),
		groupOrder,
	);
	// The decoder reads a fetch stream from its first byte, and `body` starts
	// after the FETCH_HEADER, which the session took.
	decoder.push(
		createFetchStreamEncoder({ requestId: fetch.requestId }, groupOrder).header,
	);
	const reader = fetch.body.getReader();
	/**
	 * What ends the result, once the items decoded before it were read.
	 *
	 * WHY held back for a pull of its own: `controller.error()` throws away
	 * what the stream has queued, so the Objects that came whole in the read
	 * that also brought the fault would be lost with it.
	 */
	let ending:
		| ((
				controller: ReadableStreamDefaultController<FetchItem>,
		  ) => Promise<void>)
		| undefined;
	return new ReadableStream<FetchItem>(
		{
			async pull(controller) {
				for (;;) {
					if (ending !== undefined) return ending(controller);
					// A `body` that failed fails this read, and the result with it.
					const chunk = await reader.read();
					if (chunk.done) {
						const error = decoder.finish();
						if (error === undefined) {
							controller.close();
						} else {
							connection.close(error.code, error.message);
							controller.error(error);
						}
						return;
					}
					const { items, failure } = decoder.push(chunk.value);
					let queued = 0;
					for (const item of items) {
						if (item.type === "header") continue;
						controller.enqueue(item);
						queued += 1;
					}
					if (failure !== undefined) {
						ending = async (failed) => {
							if (failure.scope === "session") {
								connection.close(failure.error.code, failure.error.message);
								failed.error(failure.error);
								return;
							}
							// Before the FETCH is cancelled: the cancel fails `body`,
							// and this is the error the reader is to see.
							failed.error(new FetchBodyError(failure.scope, failure.reason));
							await fetch.cancel();
						};
					}
					// One read of `body` for one pull, unless it completed nothing.
					if (queued > 0) return;
				}
			},
			cancel: (reason) => reader.cancel(reason),
		},
		{ highWaterMark: 0 },
	);
}
