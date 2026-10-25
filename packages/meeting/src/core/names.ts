// Where the broadcasts of a meeting are: every participant publishes under
// `xmoq/meet/<room>/<id>`, and is found with one SUBSCRIBE_NAMESPACE for
// `xmoq/meet/<room>` (decided on 2026-10-25, `docs/scale-and-meetings.md`).
// `crates/viewer/src/meeting.rs` has the same, and
// `fixtures/meeting-presence/participant-of.tsv` pins both.

/** The first two fields of a meeting's namespace. */
export const MEET = ["xmoq", "meet"] as const;

/** The prefix of a meeting's broadcasts: `xmoq/meet/<room>`. */
export function meetingPrefix(room: string): string[] {
	return [...MEET, room];
}

/**
 * The participant a namespace is the broadcast of: the field after the
 * meeting's prefix, where the namespace has exactly one more. Undefined for
 * any other, the prefix itself and a deeper one among them.
 */
export function participantOf(
	room: string,
	namespace: readonly string[],
): string | undefined {
	if (namespace.length !== MEET.length + 2) return undefined;
	const [xmoq, meet, ofRoom, id] = namespace;
	return xmoq === MEET[0] && meet === MEET[1] && ofRoom === room
		? id
		: undefined;
}
