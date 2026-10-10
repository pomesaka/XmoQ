import type { Bytes } from "./bytes.ts";
import type { Redirect } from "./control-message.ts";
import type { KeyValuePair } from "./key-value-pair.ts";
import type { Location, MessageParameters } from "./parameters.ts";
import type { SubgroupObject } from "./subgroup.ts";

export type RequestClosedReason =
	/** The peer closed the request stream with a FIN and without PUBLISH_DONE. */
	| "peer-fin"
	/** The peer reset the request stream or sent STOP_SENDING for it. */
	| "peer-reset"
	/**
	 * §10.9.1: the peer sent a REQUEST_UPDATE for a subscription made with
	 * PUBLISH. Updates are not implemented, so it was refused and the
	 * subscription ended with PUBLISH_DONE UPDATE_FAILED.
	 */
	| "update-failed"
	/**
	 * §2.5.1: SUBSCRIBE_OK or FETCH_OK carried a Mandatory Track Property
	 * this endpoint does not understand.
	 */
	| "unsupported-track-property"
	/** A FETCH has nothing more to come: its FETCH_OK came and its fetch stream ended. */
	| "fetch-served"
	/** §2.4.2: the Track was malformed and its subscription was cancelled. */
	| "malformed-track"
	/**
	 * After PUBLISH_DONE: every stream counted by its Stream Count ended, or
	 * the caller reported that the wait for late streams expired (§10.11).
	 */
	| "publish-done";

export type SessionEvent =
	/** The peer's SETUP arrived. */
	| { type: "setup"; setupOptions: KeyValuePair[] }
	| { type: "publish-namespace-ok"; requestId: bigint }
	| { type: "publish-ok"; requestId: bigint; parameters: MessageParameters }
	| {
			type: "subscribe-ok";
			requestId: bigint;
			trackAlias: bigint;
			parameters: MessageParameters;
			trackProperties: KeyValuePair[];
	  }
	| {
			type: "request-error";
			requestId: bigint;
			errorCode: bigint;
			retryInterval: bigint;
			errorReason: string;
			redirect: Redirect | undefined;
	  }
	/**
	 * Late streams can still deliver Objects after this. The subscription ends
	 * with `request-closed`, reason "publish-done".
	 */
	| {
			type: "publish-done";
			requestId: bigint;
			statusCode: bigint;
			streamCount: bigint;
			errorReason: string;
	  }
	/** The request ended and no REQUEST_ERROR said so. */
	| {
			/**
			 * After this endpoint's PUBLISH_DONE, the subscriber ended its
			 * direction of the request stream: with a FIN, or with a reset.
			 * Nothing in the draft gives that a meaning; the implementation
			 * notes have what an XmoQ relay means by it.
			 */
			type: "publish-done-peer-ended";
			requestId: bigint;
			how: "fin" | "reset";
			/** With "reset": the error code of the reset, when the transport reported one. */
			resetCode?: number;
	  }
	| {
			type: "request-closed";
			requestId: bigint;
			reason: RequestClosedReason;
			/** With "peer-reset": the error code of the reset, when the transport reported one. */
			resetCode?: number;
	  }
	| {
			type: "object";
			requestId: bigint;
			trackAlias: bigint;
			groupId: bigint;
			subgroupId: bigint;
			/** `undefined` when the SUBGROUP_HEADER has the DEFAULT_PRIORITY bit. */
			publisherPriority: number | undefined;
			object: SubgroupObject;
	  }
	| {
			type: "fetch-ok";
			requestId: bigint;
			endOfTrack: boolean;
			endLocation: Location;
			parameters: MessageParameters;
			trackProperties: KeyValuePair[];
	  }
	/**
	 * Bytes of the fetch stream of a FETCH, after its FETCH_HEADER: Objects
	 * and End of Range indicators as §11.4.4 has them, not decoded, cut
	 * wherever a read ended. They can come before `fetch-ok` (§10.13).
	 */
	| { type: "fetch-data"; requestId: bigint; data: Uint8Array }
	/**
	 * The fetch stream of a FETCH ended: the peer finished it or reset it.
	 * Not reported for a stream this endpoint stopped. The FETCH goes on
	 * until `request-closed`.
	 */
	| {
			type: "fetch-stream-ended";
			requestId: bigint;
			how: "fin" | "reset";
			/** With "reset": the error code of the reset, when the transport reported one. */
			resetCode?: number;
	  }
	/** An incoming subgroup stream was dropped; Objects on it are lost to the application. */
	| {
			type: "subgroup-abandoned";
			trackAlias: bigint | undefined;
			reason: string;
	  }
	/** A request the peer opened was answered with REQUEST_ERROR, with this code. */
	| {
			type: "peer-request-rejected";
			messageType: bigint;
			requestId: bigint;
			errorCode: bigint;
	  };

/**
 * What the I/O layer has to do, in order. `stream` numbers name streams this
 * endpoint opens and are allocated by the session; `peerStream` numbers name
 * streams the peer opened and are allocated by the I/O layer.
 */
export type SessionOutput =
	| { type: "open-uni-stream"; stream: number; data: Bytes }
	| { type: "open-bidi-stream"; stream: number; data: Bytes }
	| { type: "send"; stream: number; data: Bytes }
	/** Close the send side with a FIN. */
	| { type: "finish"; stream: number }
	/** Reset the send side and, on a bidirectional stream, send STOP_SENDING (§3.3.2). */
	| { type: "reset"; stream: number; code: number }
	/** Write `data` to the peer-opened bidirectional stream, then FIN. */
	| { type: "respond"; peerStream: number; data: Bytes }
	| { type: "stop-sending"; peerStream: number; code: number }
	/** Send STOP_SENDING for the peer's direction of a bidirectional stream this endpoint opened. Its own direction is left as it is. */
	| { type: "stop-peer-direction"; stream: number; code: number }
	| { type: "close-session"; code: number; reason: string }
	| { type: "event"; event: SessionEvent };

/** Outputs of a call that also hands a value back to the caller. */
export interface SessionResult<T> {
	value: T;
	outputs: SessionOutput[];
}
