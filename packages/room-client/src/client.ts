// Asking a room (`@xmoq/room`) over HTTP: where its relay is, tokens, and
// the seats of a meeting. What each answer is, `fixtures/room-api/README.md`.

import {
	type BroadcastRoom,
	type MeetingRoom,
	readBroadcastRoom,
	readMeetingRoom,
	readSeatAnswer,
	readToken,
	type Seat,
	type SeatAnswer,
} from "./core/answers.ts";

/** A room that answered with a status or a body this client does not take. */
export class RoomAnswerError extends Error {
	// WHY a field and not a parameter property: the room's check runs under
	// Node.js, whose stripping of types does not take parameter properties.
	readonly status: number;

	constructor(status: number, what: string) {
		super(
			`the room answered ${what} with ${status} and a body it does not take`,
		);
		this.name = "RoomAnswerError";
		this.status = status;
	}
}

/** Where a room is: the address of the rooms and its name. */
export interface RoomAddress {
	/** The address of the rooms, as `http://127.0.0.1:8787/`. */
	rooms: string;
	name: string;
}

const pathOf = (kind: "broadcasts" | "meetings", room: RoomAddress): string =>
	new URL(`${kind}/${encodeURIComponent(room.name)}`, room.rooms).href;

async function bodyOf(response: Response): Promise<unknown> {
	const text = await response.text();
	if (text === "") return undefined;
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

async function asked<T>(
	what: string,
	request: Promise<Response>,
	read: (status: number, body: unknown) => T | undefined,
	signal?: AbortSignal,
): Promise<T> {
	const response = await request;
	signal?.throwIfAborted();
	const value = read(response.status, await bodyOf(response));
	if (value === undefined) throw new RoomAnswerError(response.status, what);
	return value;
}

const post = (url: string, body: object, signal?: AbortSignal) =>
	fetch(url, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(body),
		signal,
	});

/** `GET /broadcasts/<name>`. */
export const broadcastRoom = (
	room: RoomAddress,
	signal?: AbortSignal,
): Promise<BroadcastRoom> =>
	asked(
		"a broadcast",
		fetch(pathOf("broadcasts", room), { signal }),
		readBroadcastRoom,
		signal,
	);

/** `GET /meetings/<name>`. */
export const meetingRoom = (
	room: RoomAddress,
	signal?: AbortSignal,
): Promise<MeetingRoom> =>
	asked(
		"a meeting",
		fetch(pathOf("meetings", room), { signal }),
		readMeetingRoom,
		signal,
	);

/** A token of the room of `kind` for `role` in `namespace`; null where the room makes none. */
export const tokenOfARoom = (
	kind: "broadcasts" | "meetings",
	room: RoomAddress,
	role: "pub" | "sub",
	namespace: readonly string[],
	signal?: AbortSignal,
): Promise<string | null> =>
	asked(
		"a token",
		post(`${pathOf(kind, room)}/tokens`, { role, namespace }, signal),
		(status, body) => readToken(status, body)?.token,
		signal,
	);

/** `POST /meetings/<name>/seats`: a seat for `participant`, or why there is none. */
export const takeSeat = (
	room: RoomAddress,
	participant: string,
	signal?: AbortSignal,
): Promise<SeatAnswer> =>
	asked(
		"a seat",
		post(`${pathOf("meetings", room)}/seats`, { participant }, signal),
		(status, body) => readSeatAnswer("take", status, body),
		signal,
	);

const seatPath = (room: RoomAddress, participant: string): string =>
	`${pathOf("meetings", room)}/seats/${encodeURIComponent(participant)}`;

/** `POST /meetings/<name>/seats/<participant>/renew`. */
export const renewSeat = (
	room: RoomAddress,
	seat: Seat,
	signal?: AbortSignal,
): Promise<SeatAnswer> =>
	asked(
		"a renewed seat",
		post(
			`${seatPath(room, seat.participant)}/renew`,
			{ lease: seat.lease },
			signal,
		),
		(status, body) => readSeatAnswer("renew", status, body),
		signal,
	);

/** `DELETE /meetings/<name>/seats/<participant>`: the seat is free at once. */
export async function leaveSeat(room: RoomAddress, seat: Seat): Promise<void> {
	const response = await fetch(seatPath(room, seat.participant), {
		method: "DELETE",
		headers: { "content-type": "application/json" },
		body: JSON.stringify({ lease: seat.lease }),
	});
	if (response.status !== 204) {
		throw new RoomAnswerError(response.status, "leaving a seat");
	}
}

/** A seat kept by renewing it, until it is let go of or lost. */
export interface KeptSeat {
	/** The seat as it was last taken or renewed: its tokens are the newest. */
	readonly seat: Seat;
	/** Resolves with why the seat was lost: it ran out, or a renewal failed. Never rejects. */
	readonly lost: Promise<{ type: "gone" } | { type: "failed"; error: unknown }>;
	/** Stops renewing it, and leaves it (`DELETE`). */
	leave(): Promise<void>;
}

/**
 * Keeps `seat` by renewing it every `seat.renewMs` (the room's own time).
 * WHY lost on the first renewal that fails, and not asked again: the next
 * renewal would come `renewMs` later, and a seat runs out at its
 * `expiresAt`, which the room puts some renewals after the last one (60 and
 * 20 seconds, `fixtures/room-api/README.md`), so a client that tells its
 * caller at once can take a new seat before the old one is gone. The tokens of a renewal are the caller's to use for its next
 * connection to the relay.
 */
export function keepSeat(room: RoomAddress, seat: Seat): KeptSeat {
	let current = seat;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let stopped = false;
	let lose: (
		why: { type: "gone" } | { type: "failed"; error: unknown },
	) => void = () => {};
	const lost = new Promise<
		{ type: "gone" } | { type: "failed"; error: unknown }
	>((resolve) => {
		lose = (why) => {
			stopped = true;
			resolve(why);
		};
	});
	const renew = (): void => {
		timer = setTimeout(() => {
			if (stopped) return;
			renewSeat(room, current).then(
				(answer) => {
					if (stopped) return;
					if (answer.type === "seat") {
						current = answer.seat;
						renew();
					} else lose({ type: "gone" });
				},
				(error: unknown) => {
					if (!stopped) lose({ type: "failed", error });
				},
			);
		}, current.renewMs);
	};
	renew();
	return {
		get seat() {
			return current;
		},
		lost,
		async leave() {
			if (stopped) return;
			stopped = true;
			clearTimeout(timer);
			await leaveSeat(room, current);
		},
	};
}
