import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createSubgroupStreamDecoder } from "../../src/core/subgroup.ts";
import { hex } from "../helpers.ts";

// A subgroup stream another implementation than moqtail wrote, with an
// Object Property on every Object: the start of a video stream of
// moqlivemock's publisher (fixtures/loc-04/README.md).

const FIXTURES = join(
	import.meta.dir,
	"..",
	"..",
	"..",
	"..",
	"fixtures",
	"loc-04",
);
const fixture = (name: string): Uint8Array =>
	hex(readFileSync(join(FIXTURES, `${name}.hex`), "utf8"));

describe("fixtures/loc-04: a subgroup stream of moqlivemock's publisher", () => {
	test("decodes to its header and two Objects, each with one property of type 0x10 and the payload of its own file", () => {
		const decoder = createSubgroupStreamDecoder({
			maxObjectPayloadLength: 1024 * 1024,
			maxObjectPropertiesLength: 1024,
		});
		expect(
			decoder.push(fixture("subgroup-stream-two-objects-from-moqlivemock")),
		).toEqual({
			items: [
				{
					type: "header",
					header: {
						trackAlias: 0n,
						groupId: 1791690527n,
						subgroupId: { mode: "Explicit", value: 0n },
						publisherPriority: 128,
						properties: true,
						endOfGroup: false,
						// The stream type is 0x15: the bit 0x40 is not set.
						firstObject: false,
					},
				},
				{
					type: "object",
					object: {
						objectId: 0n,
						// The wall-clock time in microseconds: the Group ID is the
						// same time in seconds.
						properties: [{ type: 0x10n, value: 1791690527000000n }],
						status: "Normal",
						payload: fixture("video-key-length-prefixed-from-moqlivemock"),
					},
				},
				{
					type: "object",
					object: {
						objectId: 1n,
						// 40 ms later: 25 frames a second.
						properties: [{ type: 0x10n, value: 1791690527040000n }],
						status: "Normal",
						payload: fixture("video-delta-length-prefixed-from-moqlivemock"),
					},
				},
			],
		});
	});
});
