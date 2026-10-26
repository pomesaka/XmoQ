import type { MessageParameters } from "./parameters.ts";
import type { SubgroupStreamLimits } from "./subgroup.ts";

/**
 * The limits an application can set. The fixed limits of the implementation
 * (how many Track Aliases are remembered, how many Key-Value-Pairs a message
 * may hold) are constants next to the code they bound.
 */
export interface SessionLimits extends SubgroupStreamLimits {
	/**
	 * Bytes held for subgroup streams that arrived before the SUBSCRIBE_OK
	 * carrying their Track Alias (§11.4.2 allows buffering them "for a brief
	 * period"). Each held Object counts its payload, its properties and a fixed
	 * overhead, so Objects without a payload are bounded too.
	 */
	maxHeldSubgroupBytes: number;
	/**
	 * Bytes all the subgroup streams of the session hold together of Objects
	 * and headers that have not arrived whole; a stream whose bytes would
	 * take the session over it is stopped (`STOP_SENDING` with
	 * INTERNAL_ERROR), and the session goes on. A finite number from 1:
	 * `resolveLimits` takes no other, so that it cannot be left unbounded.
	 *
	 * WHY on top of `maxObjectPayloadLength`: each stream holds up to one
	 * Object meanwhile, and the I/O layer reads 64 streams at once
	 * (`maxPeerUniStreams`), which is about 1 GiB for a peer that sends 64
	 * Objects of 16 MiB slowly. **Provisional**: 4 MiB, the Rust session's
	 * (`session_buffer_limit`, row `session-subgroup-bytes` of
	 * `fixtures/limits/`); an Object larger than it is not received whole by
	 * default, whatever `maxObjectPayloadLength` allows.
	 */
	maxSessionSubgroupBytes: number;
	/**
	 * Requests of the peer that are kept at once: the ones that wait for the
	 * application's answer, the subscriptions and FETCHes it accepted, and
	 * the ones that are over (refused, served, ended with PUBLISH_DONE) for
	 * as long as the peer has not ended its direction of their stream. With
	 * that many kept, a SUBSCRIBE or a FETCH is answered with REQUEST_ERROR
	 * `EXCESSIVE_LOAD`, and a request that is refused is not kept.
	 *
	 * WHY 64 where the Rust implementation has 256: an accepted subscription
	 * keeps its request stream open, and the I/O layer reads 64 streams of
	 * that kind at once (`maxPeerBidiStreams`), which a relay sets to 264.
	 * With 64 subscriptions accepted, a 65th request of the peer, and the
	 * stream of any other request of it, is not read until one of them ends.
	 *
	 * With the two numbers the same, this limit is reached only through
	 * subscriptions whose subscriber finished its direction of the request
	 * stream: those give their place among the streams back and stay open.
	 */
	maxPeerRequests: number;
	/**
	 * FETCHes of the peer that are open at once, among `maxPeerRequests`:
	 * the ones that wait for the application's answer and the ones whose
	 * fetch stream is being written. One more is answered with REQUEST_ERROR
	 * `EXCESSIVE_LOAD`.
	 *
	 * WHY 32, where an XmoQ relay asks one publisher for 16 at once: the
	 * relay has room for its next FETCH when it cancels one, and this
	 * endpoint has it when it has read that cancellation, which travels on
	 * another stream than the next FETCH and can be read after it. With the
	 * same number, a FETCH that follows a cancellation closely could be
	 * refused for a place that is about to be free. Not seen in a run: the
	 * number is twice the relay's by that argument alone.
	 */
	maxPeerFetches: number;
}

/** Provisional values: the draft sets no maximum and the project has not chosen any. */
const DEFAULT_SESSION_LIMITS: SessionLimits = {
	maxObjectPayloadLength: 16 * 1024 * 1024,
	maxObjectPropertiesLength: 64 * 1024,
	maxHeldSubgroupBytes: 1024 * 1024,
	maxSessionSubgroupBytes: 4 * 1024 * 1024,
	maxPeerRequests: 64,
	maxPeerFetches: 32,
};

/** `value` where it is a whole number from 1; a `RangeError` for any other, `Infinity` among them. */
function boundOf(name: string, value: number): number {
	if (!Number.isSafeInteger(value) || value < 1) {
		throw new RangeError(`${name} is a whole number from 1, not ${value}`);
	}
	return value;
}

/** The limits to run with: the defaults, with what the application set on top. */
export function resolveLimits(
	limits: Partial<SessionLimits> = {},
): SessionLimits {
	return {
		maxObjectPayloadLength:
			limits.maxObjectPayloadLength ??
			DEFAULT_SESSION_LIMITS.maxObjectPayloadLength,
		maxObjectPropertiesLength:
			limits.maxObjectPropertiesLength ??
			DEFAULT_SESSION_LIMITS.maxObjectPropertiesLength,
		maxHeldSubgroupBytes:
			limits.maxHeldSubgroupBytes ??
			DEFAULT_SESSION_LIMITS.maxHeldSubgroupBytes,
		maxSessionSubgroupBytes: boundOf(
			"maxSessionSubgroupBytes",
			limits.maxSessionSubgroupBytes ??
				DEFAULT_SESSION_LIMITS.maxSessionSubgroupBytes,
		),
		maxPeerRequests:
			limits.maxPeerRequests ?? DEFAULT_SESSION_LIMITS.maxPeerRequests,
		maxPeerFetches:
			limits.maxPeerFetches ?? DEFAULT_SESSION_LIMITS.maxPeerFetches,
	};
}

/**
 * How long the answer to a request is waited for at the least, in
 * milliseconds, before the parameters of the request are looked at: the time
 * the peer's SETUP gets. **Provisional**, and the value of the Rust
 * implementation (`ANSWER_WAIT_MS` in crates/moqt).
 */
export const DEFAULT_ANSWER_WAIT_MS = 10_000;

/**
 * The longest delay `setTimeout` takes, in milliseconds (HTML, "timer
 * initialization steps": a signed 32-bit number). Every wait the session is
 * handed or takes is no longer: a Retry Interval, the wait for an answer, the
 * waits of the connect options. The Rust implementation has the same
 * (`LONGEST_TIMER_MS` of crates/moqt).
 */
export const LONGEST_TIMER_MS = 2 ** 31 - 1;

/**
 * How long the answer to a request with these parameters is waited for, in
 * milliseconds: `baseMs`, and as long again as the request itself tells a
 * relay to wait.
 *
 * WHY the two parameters are added: with RENDEZVOUS_TIMEOUT "The relay does
 * not send SUBSCRIBE_OK until a publisher becomes available" (§10.2.6), and
 * FILL_TIMEOUT is "the maximum total duration in milliseconds a relay SHOULD
 * spend waiting for upstream sources" (§10.2.5). A requester that gave up
 * sooner would give up what it asked for. The draft names no time (§13.6:
 * "Each implementation is expected to set its own timeouts");
 * `fixtures/request-answer-wait/` has the rows both implementations go by.
 */
export function answerWaitMs(
	baseMs: number,
	parameters: MessageParameters | undefined,
): number {
	const asked =
		(parameters?.rendezvousTimeout ?? 0n) + (parameters?.fillTimeout ?? 0n);
	const wait = BigInt(baseMs) + asked;
	return wait > BigInt(LONGEST_TIMER_MS) ? LONGEST_TIMER_MS : Number(wait);
}
