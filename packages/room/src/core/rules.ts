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
 * The namespaces of broadcasts of the setting `BROADCAST_NAMESPACES`: each
 * its fields joined with `/`, the namespaces separated by `,`;
 * none where it is not set or is empty. WHY none and not the demo's:
 * the room is not the demo's, and one run without the setting would give
 * every room's asker `pub` in a namespace nobody chose for it. Throws a `RangeError` for a field that is empty or has a `*`, as
 * for a key that is set and is none (`keyOfSetting`).
 */
export function broadcastNamespaces(setting: string | undefined): string[][] {
	if (setting === undefined || setting === "") return [];
	const text = setting;
	const namespaces = text.split(",").map((namespace) => namespace.split("/"));
	if (
		namespaces.some((fields) =>
			fields.some((field) => field === "" || field.includes("*")),
		)
	) {
		throw new RangeError(
			`BROADCAST_NAMESPACES is namespaces of fields with no * between /, separated by ",": ${text}`,
		);
	}
	return namespaces;
}

/** What a room gives tokens in, besides its meeting (join-meeting's). */
export interface RoomNamespaces {
	/** The room's name. */
	room: string;
	/** The namespaces of the broadcasts (`broadcastNamespaces`). */
	broadcasts: readonly (readonly string[])[];
	/** The prefixes of the room's transcripts (`transcriptPrefix`), those that are there. */
	transcripts: readonly (readonly string[])[];
}

const sameFields = (a: readonly string[], b: readonly string[]): boolean =>
	a.length === b.length && a.every((field, index) => field === b[index]);

/**
 * The namespace a token-for asks for, where the room makes a token for it;
 * undefined where it makes none (and answers with token null). Only namespaces of the room
 * are answered (provisional: the user's answer to Q65 is awaited):
 *
 * | Role | Namespace |
 * |---|---|
 * | `sub` | `xmoq/chat/<room>`; `xmoq/transcript/<prefix>` of a transcript of the room; a broadcast's |
 * | `pub` | `xmoq/chat/<room>/<one field>`; a broadcast's |
 *
 * WHY only these: a token for `sub` in `xmoq` allows a SUBSCRIBE_NAMESPACE
 * for that prefix (transport §10.18; the relay checks its prefix as a
 * namespace, `allowed` of `crates/relay-core/src/lib.rs`), which tells of every
 * room's meetings and the ids of their participants. WHY a broadcast's from
 * the setting and not the one a publisher registered: anybody registers
 * any namespace, `xmoq` among them. Never one for a field with a `*` or a
 * `/`, or for a meeting's namespace, which join-meeting's tokens are for.
 */
export function tokenForNamespace(
	role: "pub" | "sub",
	asked: readonly unknown[],
	of: RoomNamespaces,
): string[] | undefined {
	const namespace = asked.map(String);
	if (!namespace.every(fieldOfANamespace)) return undefined;
	if (meetingNamespace(namespace)) return undefined;
	if (!fieldOfANamespace(of.room) || of.room === "") return undefined;
	const chat = ["xmoq", "chat", of.room];
	const ofTheRoom =
		of.broadcasts.some((broadcast) => sameFields(namespace, broadcast)) ||
		(role === "sub"
			? sameFields(namespace, chat) ||
				of.transcripts.some((prefix) =>
					sameFields(namespace, ["xmoq", "transcript", ...prefix]),
				)
			: namespace.length === chat.length + 1 &&
				sameFields(namespace.slice(0, chat.length), chat));
	return ofTheRoom ? namespace : undefined;
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
