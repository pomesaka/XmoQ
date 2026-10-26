// What the room decides, with no Durable Object, socket or clock: which
// asks it answers, the prefixes of its transcripts, and a meeting's seats.
// Why each is so is at the top of `src/worker.ts`.

/** The participants of a meeting, the agents among them (decided on 2026-10-25). */
export const MOST_IN_A_MEETING = 49;

/** Whether the namespace is a meeting's: `xmoq/meet` or under it. */
export const meetingNamespace = (namespace: readonly string[]): boolean =>
	namespace[0] === "xmoq" && namespace[1] === "meet";

/** Whether `id` can be the last field of a participant's namespace. */
export const participantId = (id: unknown): id is string =>
	typeof id === "string" &&
	id !== "" &&
	id.length <= 64 &&
	!id.includes("/") &&
	!id.includes("*");

/** Whether `name` can be a field of a namespace: the relay reads a `*` as every namespace under the ones before it, and the token joins the fields with `/`. */
const fieldOfANamespace = (name: string): boolean =>
	!name.includes("/") && !name.includes("*");

/**
 * The namespace a token-for asks for, where the room makes a token for it;
 * undefined where it leaves the ask unanswered: a field with a `*` or a
 * `/`, and a meeting's namespace, which join-meeting's tokens are for.
 */
export function tokenForNamespace(
	asked: readonly unknown[],
): string[] | undefined {
	const namespace = asked.map(String);
	if (!namespace.every(fieldOfANamespace)) return undefined;
	if (meetingNamespace(namespace)) return undefined;
	return namespace;
}

/**
 * The prefix a transcriber listens under for the room named `room`, its
 * fields, from the setting `prefix` with `{room}` put in the room's name;
 * null where there is no setting, or where a field would be empty or have a
 * `*`. A room's name with a `/` or a `*` makes no prefix.
 */
export function transcriptPrefix(
	prefix: unknown,
	room: string,
): string[] | null {
	if (typeof prefix !== "string" || prefix === "") return null;
	if (!fieldOfANamespace(room)) return null;
	const fields = prefix.replaceAll("{room}", room).split("/");
	if (fields.some((field) => field === "" || field.includes("*"))) return null;
	return fields;
}

/** Where the relay of the rooms is, or null where the Worker was not told. */
export function relayOf(
	relayUrl: unknown,
	certSha256Hex: unknown,
): { relayUrl: string; certSha256Hex: string } | null {
	if (typeof relayUrl !== "string" || relayUrl === "") return null;
	if (typeof certSha256Hex !== "string" || certSha256Hex === "") return null;
	return { relayUrl, certSha256Hex };
}

/** What a join-meeting gets. */
export type SeatAnswer =
	/** Nothing: the ask is not one the room answers. */
	| { type: "unanswered" }
	/** `{type: "meeting-full", most}`. */
	| { type: "full" }
	/** A seat, and its two namespaces; `counted` where the socket is counted from now on. */
	| { type: "seat"; publish: string[]; subscribe: string[]; counted: boolean };

/**
 * The answer to join-meeting for the participant `id`, on a socket of the
 * room `room` that joined as `mine` (undefined where it has not), while the
 * other sockets of the room joined as `others` (undefined for one that has
 * not). Unanswered: an id that is no participant's, a room whose name has a
 * `/` or a `*`, another id on a socket that joined, and an id another socket
 * has. The same id on the same socket is answered again, full or not.
 */
export function seatFor(
	id: unknown,
	room: string,
	mine: string | undefined,
	others: readonly (string | undefined)[],
): SeatAnswer {
	if (!participantId(id) || room === "" || !fieldOfANamespace(room)) {
		return { type: "unanswered" };
	}
	if (mine !== undefined && mine !== id) return { type: "unanswered" };
	if (others.includes(id)) return { type: "unanswered" };
	const counted = mine === undefined;
	if (counted) {
		const seated = others.filter((other) => other !== undefined).length;
		if (seated >= MOST_IN_A_MEETING) return { type: "full" };
	}
	return {
		type: "seat",
		publish: ["xmoq", "meet", room, id],
		subscribe: ["xmoq", "meet", room, "*"],
		counted,
	};
}
