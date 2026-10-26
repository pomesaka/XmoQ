// Checks the room Worker that run.ts serves: the room of a broadcast, its
// tokens, that nothing else is answered, and the seats of a meeting of 49.
//
// Usage: node check.ts [http://127.0.0.1:8787], with the AUTH_KEY_HEX,
// RELAY_URL, RELAY_CERT_SHA256_HEX, TRANSCRIPT_PREFIX and
// MEETING_TRANSCRIPT_PREFIX run.ts was given, if any.

import {
	broadcastRoom,
	leaveSeat,
	meetingRoom,
	renewSeat,
	type Seat,
	takeSeat,
	tokenOfARoom,
} from "@xmoq/room-client";

const origin = process.argv[2] ?? "http://127.0.0.1:8787";
const room = `check-${Date.now()}`;
const say = (what: string, value: unknown): void =>
	console.log(what, JSON.stringify(value));
const fail = (what: string): void => {
	console.log(`FAILED: ${what}`);
	process.exitCode = 1;
};

/** The value at `path` in what the room said, or undefined. */
function at(value: unknown, ...path: (string | number)[]): unknown {
	let here = value;
	for (const key of path) {
		if (typeof here !== "object" || here === null) return undefined;
		here = Object.getOwnPropertyDescriptor(here, key)?.value;
	}
	return here;
}
const same = (a: unknown, b: unknown): boolean =>
	JSON.stringify(a) === JSON.stringify(b);
const textOf = (value: unknown): string =>
	typeof value === "string" ? value : "";

const masked = (token: unknown): unknown =>
	typeof token === "string" ? token.replace(/[0-9a-f]{64}$/, "<mac>") : token;
const relayTold =
	process.env.RELAY_URL && process.env.RELAY_CERT_SHA256_HEX
		? {
				url: process.env.RELAY_URL,
				certSha256Hex: process.env.RELAY_CERT_SHA256_HEX,
			}
		: null;
/** The prefix of a transcript of the setting, with the room's name put in for {room}. */
const prefixOf = (
	setting: string | undefined,
	name: string,
): string[] | null =>
	setting ? setting.replaceAll("{room}", name).split("/") : null;

// 1. The room of a broadcast, read with `@xmoq/room-client`: its relay, the
// namespace of its one broadcast, and the prefix of its transcript, the
// same before anybody publishes and after.
const broadcasting = { rooms: `${origin}/`, name: room };
{
	const described = await broadcastRoom(broadcasting);
	say("the room of a broadcast:", described);
	if (!same(described.namespace, ["xmoq", "broadcast", room]))
		fail("the room does not name its broadcast xmoq/broadcast/<name>");
	if (!same(described.relay, relayTold))
		fail("the room does not name the relay it was told");
	if (
		!same(described.transcript, prefixOf(process.env.TRANSCRIPT_PREFIX, room))
	)
		fail("the room does not name the transcript it was told");
}

// 1b. Tokens, where the rooms have a key: the text the relay checks, in the
// namespaces of the room alone, and null for any other.
{
	const asks: { role: "pub" | "sub"; namespace: string[]; given: boolean }[] = [
		{
			role: "sub",
			namespace: ["xmoq", "chat", "broadcast", room],
			given: true,
		},
		{
			role: "pub",
			namespace: ["xmoq", "chat", "broadcast", room, "ann"],
			given: true,
		},
		{ role: "pub", namespace: ["xmoq", "broadcast", room], given: true },
		{ role: "sub", namespace: ["xmoq", "broadcast", room], given: true },
		{ role: "sub", namespace: ["xmoq", "*"], given: false },
		{ role: "sub", namespace: ["xmoq/broadcast"], given: false },
		// A meeting's namespaces are its seats', and the chat of a meeting
		// of the same name is that meeting's room's.
		{ role: "pub", namespace: ["xmoq", "meet", room, "x"], given: false },
		{ role: "sub", namespace: ["xmoq", "chat", "meeting", room], given: false },
		// None above the room, which a SUBSCRIBE_NAMESPACE would list every
		// room under, and none of another room.
		{ role: "sub", namespace: ["xmoq"], given: false },
		{ role: "sub", namespace: ["xmoq", "chat"], given: false },
		{
			role: "sub",
			namespace: ["xmoq", "broadcast", `${room}-other`],
			given: false,
		},
	];
	const answers: string[] = [];
	for (const { role, namespace, given } of asks) {
		const token = await tokenOfARoom(
			"broadcasts",
			broadcasting,
			role,
			namespace,
		);
		const what = `${role} in ${namespace.join("/")}`;
		answers.push(`${what}: ${String(masked(token))}`);
		if (!given || !process.env.AUTH_KEY_HEX) {
			if (token !== null) fail(`a token was made for ${what}`);
			continue;
		}
		const form = new RegExp(
			`^v1\\|${namespace.join("/")}\\|${role}\\|\\d+\\|[0-9a-f]{64}$`,
		);
		if (!form.test(token ?? ""))
			fail(
				`the token for ${what} has not the form v1|namespace|role|expiry|mac`,
			);
	}
	say("the tokens:", answers);
}

// 2. Nothing of what happens in a room goes through it (it goes over MoQ):
// it has no WebSocket, and no path but those of `fixtures/room-api/`.
{
	const statuses = [
		(await fetch(`${origin}/broadcasts/${room}/ws`)).status,
		(await fetch(`${origin}/rooms/${room}`)).status,
	];
	say("the statuses of a socket's path and of another:", statuses);
	if (!same(statuses, [404, 404]))
		fail("the room answers a path that is not in its table");
}

// 5. A meeting, over HTTP with `@xmoq/room-client` (so that what the room
// answers is what the client reads): 49 get a seat with its two namespaces,
// the 50th is told it is full, and gets in when one leaves; refusals say why;
// a seat is renewed with its lease, and not with another's.
{
	const meeting = { rooms: `${origin}/`, name: `${room}-meeting` };
	const described = await meetingRoom(meeting);
	say("the meeting:", described);
	if (
		!same(described.namespace, ["xmoq", "meet", meeting.name]) ||
		described.most !== 49
	) {
		fail("the meeting does not say its namespace and its 49 seats");
	}
	if (!same(described.relay, relayTold)) {
		fail("the meeting does not name the relay it was told");
	}
	const seats: Seat[] = [];
	for (let index = 0; index < 49; index++) {
		const answer = await takeSeat(meeting, `p${index}`);
		if (answer.type === "seat") seats.push(answer.seat);
	}
	const [first] = seats;
	if (seats.length !== 49 || first === undefined) {
		fail("not each of 49 got its seat");
	} else {
		say("the first one's seat:", {
			...first,
			publishToken: masked(first.publishToken),
			subscribeToken: masked(first.subscribeToken),
			lease: "<lease>",
		});
		const tokens = process.env.AUTH_KEY_HEX
			? /^v1\|xmoq\/meet\/[^|]+\/p0\|pub\|\d+\|[0-9a-f]{64}$/.test(
					textOf(first.publishToken),
				) &&
				/^v1\|xmoq\/meet\/[^|]+\/\*\|sub\|\d+\|[0-9a-f]{64}$/.test(
					textOf(first.subscribeToken),
				)
			: first.publishToken === null && first.subscribeToken === null;
		if (!tokens) fail("the meeting's tokens are not as the key says");
		if (first.renewMs !== 20_000 || first.expiresAt - Date.now() > 60_000) {
			fail("a seat does not hold 60 s and is not renewed every 20 s");
		}
	}
	const fiftieth = await takeSeat(meeting, "p49");
	say("the 50th one's answer:", fiftieth);
	if (!same(fiftieth, { type: "full", most: 49 })) {
		fail("the 50th was not told the meeting is full");
	}
	const refusals: [unknown, unknown][] = [];
	for (const id of ["", "x".repeat(65), "a/b", "*", "p1"]) {
		refusals.push([id, await takeSeat(meeting, id)]);
	}
	say("the refusals:", refusals);
	if (
		!same(
			refusals.map(([, answer]) => at(answer, "why")),
			[
				"not-a-participant",
				"not-a-participant",
				"not-a-participant",
				"not-a-participant",
				"taken",
			],
		)
	) {
		fail("one of those to be refused was not refused, or not for its reason");
	}
	if (first !== undefined) {
		const renewed = await renewSeat(meeting, first);
		if (renewed.type !== "seat" || renewed.seat.expiresAt <= first.expiresAt) {
			fail("a seat was not renewed with its lease");
		}
		const stolen = await renewSeat(meeting, { ...first, lease: "another" });
		if (stolen.type !== "gone") fail("a seat was renewed with another's lease");
		// One goes: the 50th gets in.
		await leaveSeat(meeting, first);
		const after = await takeSeat(meeting, "p49");
		say("the 50th one's answer after one left:", after.type);
		if (after.type !== "seat") fail("the 50th did not get in after one left");
	}
	// A room whose name has a `*` makes no namespace of a meeting.
	const starred = await takeSeat({ ...meeting, name: "a*b" }, "p0");
	if (!same(starred, { type: "refused", why: "not-a-meeting-room" })) {
		fail("a room whose name has a * did not refuse a seat");
	}
}
console.log(process.exitCode ? "SOME CHECKS FAILED" : "ALL CHECKS PASSED");
