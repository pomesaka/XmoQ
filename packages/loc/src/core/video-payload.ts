// The two forms the payload of a video Object of LOC can have for H.264
// and H.265 (draft-ietf-moq-loc-04 §2.1): a length before every NAL unit,
// or a start code before every NAL unit. Section numbers are of that draft.

/** §2.1.4: "The start code value is 1 in network byte order". */
const START_CODE = [0, 0, 0, 1];

/** What became of a video payload that was to have start codes. */
export type AnnexBPayload =
	| {
			type: "ok";
			/** The payload with a start code of 4 bytes before every NAL unit. */
			payload: Uint8Array;
			/** How many length prefixes were replaced; 0 for a payload that had start codes already. */
			replaced: number;
	  }
	/** The payload is in neither form; `reason` is for a log. */
	| { type: "invalid"; reason: string };

/**
 * Makes a start code of every length prefix of a video payload, for a
 * decoder that is given the "annexB" form. A payload that has start codes
 * already comes out with the same bytes. The result is always a new array
 * over a buffer of its own, and the payload that was given is not written
 * to.
 *
 * How the two are told apart is in §2.1.3: "A 4-byte length prefix can be
 * sent before each NAL Unit ... A length value of 1 SHOULD be interpreted as
 * a start code rather than a length." So the four bytes where a NAL unit
 * begins are read as a number: 1 is a start code, and what follows it is
 * left alone to the end of the payload; any other number is how many bytes
 * the NAL unit has. That everything after a start code is left alone is
 * read into the draft: a NAL unit of one byte, with lengths after it, is
 * taken for a start code, and those lengths stay lengths in a result that
 * is `ok`. Whether H.264 has such a NAL unit was not read from its
 * specification.
 *
 * Not looked at, as the draft says nothing of them or leaves them to the
 * codec:
 *
 * - **The bytes of a NAL unit**, which are copied as they are. Whether
 *   something in them looks like a start code is the codec's matter.
 * - **A start code of 3 bytes** (§2.1.4: it "MAY be used if the track never
 *   uses length prefixes or any Video Config"). Its first four bytes read
 *   as a length here, since nothing in a payload says which the track uses:
 *   a caller that knows its track has start codes does not call this. Most
 *   such payloads are `invalid`; **one whose first four bytes happen to be
 *   the number of bytes that follow is `ok` and broken**, the first byte of
 *   its NAL unit overwritten (`00 00 01 09` and 265 bytes).
 * - A length prefix of fewer than 4 bytes (§2.1.3: it "MAY be specified in
 *   the Video Config"), which is not read.
 *
 * `invalid`, where the draft says nothing of a receiver: a length of 0, a
 * length that goes beyond the payload, and fewer than four bytes where a
 * length is to begin. A payload of no bytes is `ok`, with nothing replaced,
 * and so is a start code with nothing after it, which is not looked into.
 */
export function toAnnexB(payload: Uint8Array): AnnexBPayload {
	const view = new DataView(
		payload.buffer,
		payload.byteOffset,
		payload.byteLength,
	);
	// The same length: a length prefix and a start code are 4 bytes each.
	const out = new Uint8Array(payload);
	let at = 0;
	let replaced = 0;
	while (at < payload.length) {
		if (payload.length - at < 4) {
			return {
				type: "invalid",
				reason: `${payload.length - at} bytes at ${at}, where a length of 4 bytes is to begin`,
			};
		}
		const length = view.getUint32(at);
		// A start code: from here on the payload is in the other form.
		if (length === 1) break;
		if (length === 0) {
			return { type: "invalid", reason: `a NAL unit of no bytes at ${at}` };
		}
		if (length > payload.length - at - 4) {
			return {
				type: "invalid",
				reason: `a NAL unit of ${length} bytes at ${at}, where ${payload.length - at - 4} are left`,
			};
		}
		out.set(START_CODE, at);
		replaced += 1;
		at += 4 + length;
	}
	return { type: "ok", payload: out, replaced };
}
