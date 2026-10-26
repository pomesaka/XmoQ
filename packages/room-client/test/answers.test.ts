import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	type BroadcastRoom,
	type MeetingRoom,
	readBroadcastRoom,
	readMeetingRoom,
	readSeatAnswer,
	readToken,
	type SeatAnswer,
} from "../src/core/answers.ts";

/** The rows of `fixtures/room-api/answers.tsv`, which the Rust client reads as well. */
function rows(): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"room-api",
		"answers.tsv",
	);
	const table = readFileSync(path, "utf8");
	// WHY not filter empty lines away: every row ends with one line feed,
	// the last one too, as the Rust side reads it; an empty row is an error.
	if (!table.endsWith("\n"))
		throw new Error(`${path}: no line feed at its end`);
	const read = table
		.slice(0, -1)
		.split("\n")
		.map((line) => line.split("\t"));
	if (read.some((row) => row.length !== 5)) {
		throw new Error(`${path}: a row of other than five columns`);
	}
	return read;
}

const orDash = (value: string | null): string => value ?? "-";

function roomForm(room: BroadcastRoom): string {
	return [
		`relay=${orDash(room.relay?.url ?? null)}`,
		`cert=${orDash(room.relay?.certSha256Hex ?? null)}`,
		`namespace=${room.namespace.join("/")}`,
		`transcript=${orDash(room.transcript?.join("/") ?? null)}`,
	].join(" ");
}

function seatForm(answer: SeatAnswer): string {
	switch (answer.type) {
		case "seat": {
			const { seat } = answer;
			return [
				"seat",
				`participant=${seat.participant}`,
				`pub=${orDash(seat.publishToken)}`,
				`sub=${orDash(seat.subscribeToken)}`,
				`lease=${seat.lease}`,
				`expires=${seat.expiresAt}`,
				`renew=${seat.renewMs}`,
			].join(" ");
		}
		case "full":
			return `full ${answer.most}`;
		case "refused":
			return `refused ${answer.why}`;
		case "gone":
			return "gone";
	}
}

/** What this client reads of an answer to `request`, in the forms of the table's README. */
function readForm(request: string, status: number, body: unknown): string {
	const [method, path = ""] = request.split(" ");
	const malformed = "malformed";
	if (method === "DELETE") return status === 204 ? "left" : malformed;
	if (path.endsWith("/tokens")) {
		const token = readToken(status, body);
		return token === undefined ? malformed : `token ${orDash(token.token)}`;
	}
	if (path.includes("/seats")) {
		const answer = readSeatAnswer(
			path.endsWith("/renew") ? "renew" : "take",
			status,
			body,
		);
		return answer === undefined ? malformed : seatForm(answer);
	}
	if (path.startsWith("/meetings/")) {
		const room: MeetingRoom | undefined = readMeetingRoom(status, body);
		return room === undefined
			? malformed
			: `meeting ${roomForm(room)} most=${room.most}`;
	}
	const room = readBroadcastRoom(status, body);
	return room === undefined ? malformed : `broadcast ${roomForm(room)}`;
}

const table = rows();

test("the table has its rows", () => {
	expect(table.length).toBe(30);
});

test.each(table)("%s", (_name, request, status, body, read) => {
	const parsed: unknown = body === "-" ? undefined : JSON.parse(body ?? "");
	expect(readForm(request ?? "", Number(status), parsed)).toBe(read ?? "");
});
