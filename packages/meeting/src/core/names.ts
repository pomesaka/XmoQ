// Where the broadcasts of a meeting are: every participant publishes under
// `xmoq/meet/<room>/<id>`, and is found with one SUBSCRIBE_NAMESPACE for
// `xmoq/meet/<room>` (the meetings of `docs/architecture.md`).
// `xmoq-meeting-tracks` (`crates/meeting-tracks`) has the same, and
// `namespaces.tsv` and `participant-of.tsv` of `fixtures/meeting-presence/`
// pin both. WHY no import here: the room's Worker reads this file
// (`@xmoq/meeting/names`) without the browser code of the package.

/** The first two fields of a meeting's namespace. */
export const MEET = ["xmoq", "meet"] as const;

/** The prefix of a meeting's broadcasts: `xmoq/meet/<room>`. */
export function meetingPrefix(room: string): string[] {
	return [...MEET, room];
}

/** The namespace the participant `id` publishes its broadcast under: `xmoq/meet/<room>/<id>`. */
export function participantNamespace(room: string, id: string): string[] {
	return [...MEET, room, id];
}

/**
 * The namespace of a seat's token to subscribe: `xmoq/meet/<room>/*`. WHY
 * the `*` and not the prefix: the relay takes a token for a
 * SUBSCRIBE_NAMESPACE only where its last field is `*`, which allows the
 * namespaces under the fields before it (`crates/relay/src/auth.rs`, decided
 * on 2026-10-26).
 */
export function seatSubscribeNamespace(room: string): string[] {
	return participantNamespace(room, "*");
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
