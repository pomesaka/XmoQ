// Writes opened.tsv, the table of README.md. Run: bun fixtures/peer-subscribe/generate.ts
//
// WHY a script: every row is the one captured SUBSCRIBE with a byte or two
// changed, and the rows say which.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const here = import.meta.dir;
const captured = readFileSync(
	join(here, "..", "moqt-18", "subscribe-moqtail-rs.hex"),
	"utf8",
)
	.trim()
	.split(/\s+/);
// Type, a length of two bytes, and 31 bytes of payload: Request ID 0, the
// namespace ("xmoq", "phase0"), and the Track Name "object", whose last
// byte is at index 23.
if (captured.length !== 34 || captured[3] !== "00" || captured[23] !== "74") {
	throw new Error("subscribe-moqtail-rs.hex is not the SUBSCRIBE this was written for");
}
/** The SUBSCRIBE with this Request ID, for the captured track or for "objecu". */
const subscribe = (requestId: number, track: "object" | "objecu" = "object"): string => {
	const bytes = [...captured];
	bytes[3] = requestId.toString(16).padStart(2, "0");
	if (track === "objecu") bytes[23] = "75";
	return bytes.join("");
};

/** What this endpoint did, or saw, before the row's SUBSCRIBE: see README.md. */
type Before =
	| "nothing"
	| "publish"
	| "subscribe"
	| "subscribe-accepted"
	| "subscribe-done"
	| "subscribe-rejected";
type Outcome = "handed" | "refused 0x19" | "closed 0x4";

const rows: [name: string, before: Before, sent: string, outcome: Outcome][] = [
	["first-request", "nothing", subscribe(1), "handed"],
	["track-this-endpoint-published", "publish", subscribe(1), "refused 0x19"],
	["another-track-than-published", "publish", subscribe(1, "objecu"), "handed"],
	["track-asked-for-and-not-answered", "subscribe", subscribe(3), "refused 0x19"],
	["another-track-than-asked-for", "subscribe", subscribe(3, "objecu"), "handed"],
	["track-of-an-accepted-subscribe", "subscribe-accepted", subscribe(3), "refused 0x19"],
	["track-after-publish-done", "subscribe-done", subscribe(3), "handed"],
	["track-after-a-rejection", "subscribe-rejected", subscribe(3), "handed"],
	["request-id-of-the-wrong-parity", "nothing", subscribe(2), "closed 0x4"],
	["request-id-used-before", "subscribe", subscribe(1, "objecu"), "closed 0x4"],
];

writeFileSync(
	join(here, "opened.tsv"),
	rows.map((row) => `${row.join("\t")}\n`).join(""),
);
