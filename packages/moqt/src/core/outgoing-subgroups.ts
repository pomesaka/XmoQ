import { InactiveError, StreamResetErrorCode } from "./error.ts";
import type { KeyValuePair } from "./key-value-pair.ts";
import type { SessionOutput, SessionResult } from "./session-output.ts";
import {
	encodeSubgroupHeader,
	encodeSubgroupObject,
	type ObjectStatus,
	type SubgroupHeader,
	type SubgroupId,
} from "./subgroup.ts";

export interface SubgroupOptions {
	groupId: bigint;
	subgroupId: SubgroupId;
	/** Omit to set the DEFAULT_PRIORITY bit. */
	publisherPriority?: number;
	/** Whether Objects on this stream carry Object Properties. */
	properties: boolean;
	endOfGroup: boolean;
}

export interface OutgoingObject {
	objectId: bigint;
	payload: Uint8Array;
	properties?: KeyValuePair[];
	status?: ObjectStatus;
}

/**
 * The publisher's side of data streams: the subgroup streams this endpoint
 * writes, grouped by the subscription (Request ID of its PUBLISH) they belong
 * to. Whether a subscription may send at all is decided by the caller.
 */
export interface OutgoingSubgroups {
	/** Opens a subgroup stream with its SUBGROUP_HEADER (§11.4.2). Returns the stream number. */
	open(
		requestId: bigint,
		trackAlias: bigint,
		options: SubgroupOptions,
	): SessionResult<number>;
	write(stream: number, object: OutgoingObject): SessionOutput[];
	/** Every Object of the subgroup was written: FIN (§11.4.3). */
	close(stream: number): SessionOutput[];
	/** The subgroup is incomplete: reset (§11.4.3). */
	reset(stream: number, code: number): SessionOutput[];
	/**
	 * The stream could not be opened: the peer allowed no further stream.
	 * It is forgotten and is none of the streams opened for the
	 * subscription, so that the Stream Count of PUBLISH_DONE stays exact
	 * (§10.11: "the number of data streams the publisher opened for this
	 * subscription"). Does nothing for a stream that is gone.
	 */
	notOpened(stream: number): void;
	/** The peer sent STOP_SENDING for the stream: it is reset, as §11.4.3 asks ("The publisher SHOULD respond with a reset"). */
	stopped(stream: number, out: SessionOutput[]): void;
	/** Resets every open stream of a subscription (§5.1.1, §11.4.3). */
	resetAll(requestId: bigint, out: SessionOutput[]): void;
	hasOpen(requestId: bigint): boolean;
	/** Data streams opened for the subscription so far: Stream Count of PUBLISH_DONE (§10.11). */
	streamCount(requestId: bigint): bigint;
	/** Drops the counters of a subscription that ended. Its streams have to be closed or reset first. */
	forget(requestId: bigint): void;
	clear(): void;
}

interface OutgoingSubgroup {
	requestId: bigint;
	header: SubgroupHeader;
	previousObjectId: bigint | undefined;
}

interface SubscriptionStreams {
	open: Set<number>;
	opened: bigint;
}

export function createOutgoingSubgroups(
	allocateStream: () => number,
): OutgoingSubgroups {
	const streams = new Map<number, OutgoingSubgroup>();
	const subscriptions = new Map<bigint, SubscriptionStreams>();

	const openStream = (stream: number): OutgoingSubgroup => {
		const subgroup = streams.get(stream);
		if (subgroup === undefined) {
			throw new InactiveError(
				`stream ${stream} is not an open subgroup stream`,
			);
		}
		return subgroup;
	};

	const forgetStream = (stream: number): void => {
		const subgroup = openStream(stream);
		streams.delete(stream);
		subscriptions.get(subgroup.requestId)?.open.delete(stream);
	};

	return {
		open(requestId, trackAlias, options) {
			const header: SubgroupHeader = {
				trackAlias,
				groupId: options.groupId,
				subgroupId: options.subgroupId,
				publisherPriority: options.publisherPriority,
				properties: options.properties,
				endOfGroup: options.endOfGroup,
				// §2.2: the Original Publisher sets FIRST_OBJECT when it opens a new
				// subgroup.
				// FIXME: a subgroup opened again after a reset is not new, and
				// FIRST_OBJECT would be wrong for it. Take the bit from the caller
				// once reopening is supported.
				firstObject: true,
			};
			const data = encodeSubgroupHeader(header);
			const stream = allocateStream();
			streams.set(stream, { requestId, header, previousObjectId: undefined });
			const subscription = subscriptions.get(requestId) ?? {
				open: new Set(),
				opened: 0n,
			};
			subscription.open.add(stream);
			subscription.opened += 1n;
			subscriptions.set(requestId, subscription);
			return {
				value: stream,
				outputs: [{ type: "open-uni-stream", stream, data }],
			};
		},

		write(stream, object) {
			const subgroup = openStream(stream);
			const data = encodeSubgroupObject(
				subgroup.header,
				subgroup.previousObjectId,
				{
					objectId: object.objectId,
					properties: object.properties ?? [],
					status: object.status ?? "Normal",
					payload: object.payload,
				},
			);
			subgroup.previousObjectId = object.objectId;
			return [{ type: "send", stream, data }];
		},

		close(stream) {
			forgetStream(stream);
			return [{ type: "finish", stream }];
		},

		reset(stream, code) {
			forgetStream(stream);
			return [{ type: "reset", stream, code }];
		},

		notOpened(stream) {
			const subgroup = streams.get(stream);
			if (subgroup === undefined) return;
			forgetStream(stream);
			const subscription = subscriptions.get(subgroup.requestId);
			if (subscription !== undefined) subscription.opened -= 1n;
		},

		stopped(stream, out) {
			if (!streams.has(stream)) return;
			forgetStream(stream);
			out.push({ type: "reset", stream, code: StreamResetErrorCode.CANCELLED });
		},

		resetAll(requestId, out) {
			const subscription = subscriptions.get(requestId);
			if (subscription === undefined) return;
			for (const stream of subscription.open) {
				streams.delete(stream);
				out.push({
					type: "reset",
					stream,
					code: StreamResetErrorCode.CANCELLED,
				});
			}
			subscription.open.clear();
		},

		hasOpen: (requestId) => (subscriptions.get(requestId)?.open.size ?? 0) > 0,
		streamCount: (requestId) => subscriptions.get(requestId)?.opened ?? 0n,

		forget(requestId) {
			subscriptions.delete(requestId);
		},

		clear() {
			streams.clear();
			subscriptions.clear();
		},
	};
}
