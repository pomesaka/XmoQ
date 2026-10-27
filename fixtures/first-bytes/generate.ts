// Writes said.tsv, the table of README.md. Run: bun fixtures/first-bytes/generate.ts
//
// WHY a script: the rows of the request are one capture cut at several
// lengths, and a length written by hand was one byte off once.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const here = import.meta.dir;
const subscribe = readFileSync(
	join(here, "..", "moqt-18", "subscribe-moqtail-rs.hex"),
	"utf8",
)
	.trim()
	.split(/\s+/);
// Type, a length of two bytes, and 31 bytes of payload that begin with the
// Request ID 0.
if (subscribe.length !== 34 || subscribe[3] !== "00") {
	throw new Error("subscribe-moqtail-rs.hex is not the SUBSCRIBE this was written for");
}
// Request ID 1: a request of the server, which a client receives.
subscribe[3] = "01";
const request = subscribe.join("");
/** The first `bytes` bytes of the request. */
const cut = (bytes: number): string => request.slice(0, bytes * 2);

const rows: [
	name: string,
	kind: "uni" | "bidi",
	sent: string,
	said: "said" | "not-yet",
	stopped: string,
	fin: string,
][] = [
	["nothing", "uni", "", "not-yet", "0x0", "nothing"],
	["half-of-a-type", "uni", "80", "not-yet", "0x0", "closed:0x3"],
	["half-of-the-control-type", "uni", "af", "not-yet", "0x0", "closed:0x3"],
	["control-type", "uni", "af00", "said", "-", "-"],
	["padding-type", "uni", "f0132b3e28", "said", "-", "-"],
	["subgroup-type", "uni", "5d", "not-yet", "0x0", "closed:0x3"],
	["subgroup-type-and-track-alias", "uni", "5d00", "not-yet", "0x0", "closed:0x3"],
	["subgroup-header", "uni", "5d00000000", "said", "-", "-"],
	["subgroup-header-and-half-an-object", "uni", "5d00000000000561", "said", "-", "-"],
	["fetch-type", "uni", "05", "not-yet", "0x0", "closed:0x3"],
	["fetch-header", "uni", "0500", "said", "-", "-"],
	["nothing", "bidi", "", "not-yet", "0x1", "nothing"],
	["message-type", "bidi", cut(1), "not-yet", "0x1", "closed:0x3"],
	["type-and-length", "bidi", cut(3), "not-yet", "0x1", "closed:0x3"],
	["request-id", "bidi", cut(4), "not-yet", "0x1", "closed:0x3"],
	["after-request-id", "bidi", cut(5), "not-yet", "0x1", "closed:0x3"],
	["all-but-the-last-byte", "bidi", cut(33), "not-yet", "0x1", "closed:0x3"],
	["whole-first-message", "bidi", request, "said", "-", "-"],
];

writeFileSync(
	join(here, "said.tsv"),
	rows.map((row) => `${row.join("\t")}\n`).join(""),
);
