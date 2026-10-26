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

/** Why a meeting gives no seat, other than a full meeting. */
export type SeatRefusal =
	/** The id is no participant's (`participantId`). */
	| "not-a-participant"
	/** The name of the room is empty, or has a `/` or a `*`: it makes no namespace of a meeting. */
	| "not-a-meeting-room"
	/** A seat that has not run out has the id. */
	| "taken";

/** A seat of a meeting, as the room holds it. */
export interface HeldSeat {
	/** What it is renewed and left with. */
	lease: string;
	/** When it runs out, in milliseconds since the Unix epoch. */
	expiresAt: number;
}

/** What asking for a seat gets. */
export type SeatAnswer =
	| { type: "refused"; why: SeatRefusal }
	| { type: "full" }
	/** A seat, and its two namespaces. */
	| { type: "seat"; publish: string[]; subscribe: string[] };

/** The seats that have not run out at `now`. */
const heldAt = (
	seats: ReadonlyMap<string, HeldSeat>,
	now: number,
): [string, HeldSeat][] =>
	[...seats].filter(([, seat]) => seat.expiresAt > now);

/**
 * The answer to asking for a seat for the participant `id` in the meeting of
 * the room `room`, while `seats` are held, at `now`. A seat that ran out
 * counts for nothing: its id can be taken, and its place is free.
 *
 * WHY a seat holds by its lease and not by a connection: the room is asked
 * over HTTP, and nothing of it stays open while a participant is there
 * (decided on 2026-10-26). A participant that leaves without saying so
 * holds its seat until it runs out (`seatTimesOf`). WHY NOT the namespaces
 * the relay announces: the room does not talk to the relay, and a token
 * outlives a seat for its 600 s, so a participant that left may stay at the
 * relay that long; the room counts who it gives a token to now.
 */
export function seatFor(
	id: unknown,
	room: string,
	seats: ReadonlyMap<string, HeldSeat>,
	now: number,
): SeatAnswer {
	if (!participantId(id)) return { type: "refused", why: "not-a-participant" };
	if (room === "" || !fieldOfANamespace(room)) {
		return { type: "refused", why: "not-a-meeting-room" };
	}
	const held = heldAt(seats, now);
	if (held.some(([each]) => each === id)) {
		return { type: "refused", why: "taken" };
	}
	if (held.length >= MOST_IN_A_MEETING) return { type: "full" };
	return {
		type: "seat",
		publish: ["xmoq", "meet", room, id],
		subscribe: ["xmoq", "meet", room, "*"],
	};
}

/**
 * Whether the seat of `id` can be renewed, or left, with `lease` at `now`:
 * it has not run out, and the lease is its own. WHY the lease and not the
 * id alone: any client can name any id, and would take another's seat.
 */
export function holdsSeat(
	id: string,
	lease: unknown,
	seats: ReadonlyMap<string, HeldSeat>,
	now: number,
): boolean {
	const seat = seats.get(id);
	return seat !== undefined && seat.expiresAt > now && seat.lease === lease;
}

/** How long a seat holds, and how often a participant renews it, in milliseconds. */
export interface SeatTimes {
	seatMs: number;
	renewMs: number;
}

/**
 * **Provisional** (2026-10-26): a seat holds 60 s, and is renewed every 20 s,
 * so that two renewals can fail before it runs out. Short, so that the seat
 * of a participant that left without saying so is free soon; long beside
 * the time a renewal takes. Changed with `MEETING_SEAT_MS` and
 * `MEETING_RENEW_MS`.
 */
export const DEFAULT_SEAT_TIMES: SeatTimes = {
	seatMs: 60_000,
	renewMs: 20_000,
};

/**
 * The times of the settings `MEETING_SEAT_MS` and `MEETING_RENEW_MS`, the
 * default for one that is not set or is empty. Throws a `RangeError` for one
 * that is no whole number from 1, or a renewal that is not shorter than the
 * seat, which would let a seat run out between two renewals; as for a key
 * that is set and is none (`keyOfSetting`).
 */
export function seatTimesOf(
	seatMs: string | undefined,
	renewMs: string | undefined,
): SeatTimes {
	const time = (
		name: string,
		value: string | undefined,
		by: number,
	): number => {
		if (value === undefined || value === "") return by;
		const ms = Number(value);
		if (!Number.isSafeInteger(ms) || ms < 1) {
			throw new RangeError(
				`${name} is a whole number of milliseconds from 1, not ${value}`,
			);
		}
		return ms;
	};
	const times = {
		seatMs: time("MEETING_SEAT_MS", seatMs, DEFAULT_SEAT_TIMES.seatMs),
		renewMs: time("MEETING_RENEW_MS", renewMs, DEFAULT_SEAT_TIMES.renewMs),
	};
	if (times.renewMs >= times.seatMs) {
		throw new RangeError(
			`MEETING_RENEW_MS (${times.renewMs}) is to be shorter than MEETING_SEAT_MS (${times.seatMs})`,
		);
	}
	return times;
}
