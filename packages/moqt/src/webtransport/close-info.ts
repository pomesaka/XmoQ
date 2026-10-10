/**
 * The longest reason of a session close, in bytes of UTF-8.
 * `draft-ietf-webtrans-http3-16` §6: the Application Error Message of
 * WT_CLOSE_SESSION "MUST NOT exceed 1024 bytes". The Rust implementation has
 * the same limit (`MAX_CLOSE_REASON_LEN` in crates/moqt-io).
 */
export const MAX_CLOSE_REASON_BYTES = 1024;

/** The largest Session Termination Error Code: the capsule carries 32 bits. */
const MAX_CLOSE_CODE = 0xffff_ffff;

/**
 * `code` as it is sent as a Session Termination Error Code, or a `RangeError`
 * when it cannot be sent: it has to be a whole number from 0 to 2^32 - 1.
 *
 * WHY refused here and not left to the browser: `closeCode` is an `unsigned
 * long` without `[EnforceRange]` (W3C WebTransport §6.10), so the browser
 * wraps instead of refusing. Chromium 156.0.8078.4 sent 2^32 as 0, -1 as
 * 2^32 - 1, 1.5 as 1 and NaN as 0 (checked against the own relay on
 * 2026-10-10, spikes/wt-close-info). The peer would be told another code than
 * the one this endpoint reports in `closed`.
 * WHY 32 bits when the codes of draft 18 are variable-length integers of up
 * to 62 bits (§3.5): WT_CLOSE_SESSION carries 32. The codes the draft names
 * are 0x1A or less (table in §15.10.1). It also reserves 0x7f * N + 0x9D
 * for greasing; of those, only the ones below 2^32 can be sent.
 */
export function closeCode(code: number): number {
	if (!Number.isInteger(code) || code < 0 || code > MAX_CLOSE_CODE) {
		throw new RangeError(
			`a session is closed with a whole number from 0 to ${MAX_CLOSE_CODE}, not ${String(code)}`,
		);
	}
	// -0 is a whole number and is sent as 0; `closed` must not report -0.
	return code + 0;
}

/**
 * The reason of a close as it is sent: at most 1024 bytes of UTF-8, cut
 * between two characters (`draft-ietf-webtrans-http3-16` §6: a sender that
 * truncates "MUST do so at a UTF-8 character boundary"). A code unit that is
 * half of a pair is sent as U+FFFD.
 *
 * WHY cut here when the browser cuts too: Chromium 156 sent the cut reason
 * and resolved `closed` with the whole one (same check as above), so what
 * this endpoint reported would differ from what the peer got.
 *
 * The bytes for the inputs of fixtures/webtransport-close/reasons.tsv are the
 * ones Chromium sent; the Rust implementation (`close_reason` in
 * crates/moqt-io) has to give the same.
 */
export function closeReason(reason: string): string {
	// WHY `ignoreBOM`: without it the decoder drops a leading U+FEFF, and the
	// browser sends it (fixture "leading-bom").
	const decoder = new TextDecoder("utf-8", { ignoreBOM: true });
	const bytes = new TextEncoder().encode(reason);
	if (bytes.length <= MAX_CLOSE_REASON_BYTES) return decoder.decode(bytes);
	let end = MAX_CLOSE_REASON_BYTES;
	// 0b10xxxxxx continues a character that began before `end`.
	while (end > 0 && ((bytes[end] ?? 0) & 0xc0) === 0x80) end -= 1;
	return decoder.decode(bytes.subarray(0, end));
}
