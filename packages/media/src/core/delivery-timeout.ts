// What a viewer asks of whoever sends it a track: how long an Object or the
// end of a Subgroup may take before the sender gives it up
// (draft-ietf-moq-transport-18 §8). No I/O.

/**
 * The delivery timeouts of a subscription, in milliseconds. Each is left
 * out of the SUBSCRIBE when it is left out here.
 *
 * - `subgroupMs` (SUBGROUP_DELIVERY_TIMEOUT, §10.2.3): counted from when
 *   the sender knows that everything of a Subgroup was published; a stream
 *   that is not delivered by then is reset. With a Group on one stream,
 *   this is "give the Group up so long after its end".
 * - `objectMs` (OBJECT_DELIVERY_TIMEOUT, §10.2.4): counted from when the
 *   sender has the first byte of an Object, and checked when the Object is
 *   handed to the transport; the stream is reset.
 *
 * §8: "a value of 0 means that there is no timeout set", and where the
 * publisher has one of its own for the track, "the smaller of the two is
 * used". **Who follows it is the sender's matter**: a sender that does not
 * act on these plays every Object as before.
 */
export interface DeliveryTimeouts {
	subgroupMs?: number;
	objectMs?: number;
}

/**
 * The Message Parameters of a SUBSCRIBE for these timeouts, in the names
 * `@xmoq/moqt` has for them. Throws a `RangeError` for a time that is no
 * whole number from 0 to 2^53 - 1.
 */
export function deliveryTimeoutParameters(timeouts: DeliveryTimeouts = {}): {
	subgroupDeliveryTimeout?: bigint;
	objectDeliveryTimeout?: bigint;
} {
	const milliseconds = (name: string, value: number): bigint => {
		if (!Number.isSafeInteger(value) || value < 0) {
			throw new RangeError(
				`${name} is a whole number of milliseconds from 0, not ${value}`,
			);
		}
		return BigInt(value);
	};
	return {
		...(timeouts.subgroupMs === undefined
			? {}
			: {
					subgroupDeliveryTimeout: milliseconds(
						"subgroupMs",
						timeouts.subgroupMs,
					),
				}),
		...(timeouts.objectMs === undefined
			? {}
			: { objectDeliveryTimeout: milliseconds("objectMs", timeouts.objectMs) }),
	};
}
