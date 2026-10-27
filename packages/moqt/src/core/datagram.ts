import { createByteReader, ShortBufferError } from "./bytes.ts";

/** What a datagram of the peer is, by its type (§11). */
export type DatagramKind =
	/** An OBJECT_DATAGRAM (§11.3.1) of a valid Type. The rest is not read. */
	| { kind: "object"; type: bigint }
	/** A PADDING DATAGRAM (§11.5.2), to be discarded. */
	| { kind: "padding" }
	/** A datagram that closes the session with PROTOCOL_VIOLATION. */
	| { kind: "fault"; reason: string };

/** §11.5.2, Figure 29. */
const DATAGRAM_TYPE_PADDING = 0x132b3e29n;
/** The bits an OBJECT_DATAGRAM Type may have: the form 0b00X0XXXX (§11.3.1). */
const OBJECT_DATAGRAM_TYPE_BITS = 0x2fn;
/** The STATUS bit (0x20) and the END_OF_GROUP bit (0x02) of §11.3.1. */
const STATUS_AND_END_OF_GROUP = 0x22n;

/**
 * Says what a datagram of the peer is from its type, the varint it starts
 * with (§11: "All MOQT datagrams start with a variable-length integer
 * indicating the type of the datagram").
 *
 * A fault, for which the session is closed:
 * - a type of neither an OBJECT_DATAGRAM nor a PADDING DATAGRAM. §11: "An
 *   endpoint that receives an unknown datagram type MUST close the
 *   session"; §11.3.1 names PROTOCOL_VIOLATION for "Type values that do not
 *   match the form 0b00X0XXXX".
 * - a Type with both STATUS (0x20) and END_OF_GROUP (0x02), 0x22 and the
 *   seven after it of that form (§11.3.1: "An object status message cannot
 *   signal end of group"; MUST close with PROTOCOL_VIOLATION).
 * - an empty datagram, and one that ends inside its type. WHY a fault: the
 *   draft has no rule for them, and a datagram has nothing after it that
 *   could bring the rest, so its type is unknown, which §11 has close the
 *   session. Both implementations keep to this rule
 *   (fixtures/moqt-18-verdicts/datagrams.tsv).
 *
 * WHY padding is not a fault, although it is not of the form 0b00X0XXXX:
 * §11.5.2 defines it ("An endpoint MAY send a datagram with a type of
 * 0x132B3E29") and says "The receiver MUST discard all data received in a
 * padding datagram", so it is a known type. Its bytes, which "MUST all be
 * set to zero", are not looked at: what the receiver does with others is
 * not said, and discarding is what it MUST do.
 *
 * WHY nothing past the type is read: Object Datagrams are not delivered,
 * so their fields lead nowhere. The rules on them (§11.3.1 PROPERTIES with
 * a Properties Length of 0, STATUS with PROPERTIES and a Status that is
 * not Normal, §11.2.1.1 an unknown Object Status) are therefore not
 * checked.
 */
export function classifyDatagram(datagram: Uint8Array): DatagramKind {
	if (datagram.length === 0) return { kind: "fault", reason: "empty datagram" };
	let type: bigint;
	try {
		type = createByteReader(datagram).varint();
	} catch (error) {
		if (!(error instanceof ShortBufferError)) throw error;
		return { kind: "fault", reason: "datagram ended inside its type" };
	}
	if (type === DATAGRAM_TYPE_PADDING) return { kind: "padding" };
	if ((type & ~OBJECT_DATAGRAM_TYPE_BITS) !== 0n) {
		return {
			kind: "fault",
			reason: `unknown datagram type 0x${type.toString(16)}`,
		};
	}
	if ((type & STATUS_AND_END_OF_GROUP) === STATUS_AND_END_OF_GROUP) {
		return {
			kind: "fault",
			reason: `datagram type 0x${type.toString(16)} has both STATUS and END_OF_GROUP`,
		};
	}
	return { kind: "object", type };
}
