import { describe, expect, test } from "bun:test";
import { createByteReader } from "../../src/core/bytes.ts";
import {
	decodeControlMessage,
	encodeControlMessage,
} from "../../src/core/control-message.ts";
import { SessionErrorCode } from "../../src/core/error.ts";
import {
	propertiesToSearch,
	trackPropertiesRefusal,
} from "../../src/core/immutable-properties.ts";
import {
	decodeKeyValuePairs,
	type KeyValuePair,
} from "../../src/core/key-value-pair.ts";
import { createSubgroupStreamDecoder } from "../../src/core/subgroup.ts";
import {
	hex,
	type ImmutablePropertiesFixture,
	immutablePropertiesFixtures,
	toHex,
} from "../helpers.ts";

/** Column 6 of the shared table. */
function written(properties: readonly KeyValuePair[]): string {
	if (properties.length === 0) return "none";
	return properties
		.map(({ type, value }) => {
			const shown =
				typeof value === "bigint"
					? value.toString()
					: toHex(value).replaceAll(" ", "");
			return `${type.toString(16).padStart(2, "0")}=${shown}`;
		})
		.join(",");
}

type Outcome = Pick<ImmutablePropertiesFixture, "outcome" | "searched">;

/** An `object` row, as the Object of a subgroup stream in its Group. */
function readObject(
	propertiesHex: string,
	groupId: bigint,
	objectId: bigint,
): Outcome {
	const properties = hex(propertiesHex);
	// SUBGROUP_HEADER 0x15 (§11.4.2): Track Alias 2, the Group, Subgroup 0,
	// Publisher Priority 0. Then the first Object, whose Object ID Delta is
	// its Object ID, its Properties, and a payload of one byte.
	const stream = Uint8Array.from([
		0x15,
		0x02,
		Number(groupId),
		0x00,
		0x00,
		Number(objectId),
		properties.length,
		...properties,
		0x01,
		0x61,
	]);
	const { items, failure } = createSubgroupStreamDecoder({
		maxObjectPayloadLength: 16,
		maxObjectPropertiesLength: 64,
	}).push(stream);
	if (failure?.scope === "track") {
		return { outcome: "malformed-track", searched: "-" };
	}
	if (
		failure?.scope === "session" &&
		failure.error.code === SessionErrorCode.PROTOCOL_VIOLATION
	) {
		return { outcome: "protocol-violation", searched: "-" };
	}
	const object = items[1];
	if (failure !== undefined || object?.type !== "object") {
		throw new Error(`the Object was not read: ${JSON.stringify(failure)}`);
	}
	const reading = propertiesToSearch(object.object.properties, "Object");
	if ("malformed" in reading) throw new Error(reading.malformed);
	return { outcome: "ok", searched: written(reading.properties) };
}

/** A `track` row, as the Track Properties of a SUBSCRIBE_OK. */
function readTrack(propertiesHex: string): Outcome {
	const bytes = hex(propertiesHex);
	const message = encodeControlMessage({
		type: "SUBSCRIBE_OK",
		trackAlias: 9n,
		parameters: {},
		trackProperties: decodeKeyValuePairs(createByteReader(bytes)),
	});
	// The message carries the bytes of the row, and not ones encoded anew.
	expect(toHex(message.subarray(message.length - bytes.length))).toBe(
		propertiesHex,
	);
	const decoded = decodeControlMessage(message);
	if (decoded.status === "error") {
		expect(decoded.error.code).toBe(SessionErrorCode.PROTOCOL_VIOLATION);
		return { outcome: "protocol-violation", searched: "-" };
	}
	if (decoded.status !== "ok" || decoded.value.type !== "SUBSCRIBE_OK") {
		throw new Error(`not a SUBSCRIBE_OK: ${decoded.status}`);
	}
	const { trackProperties } = decoded.value;
	switch (trackPropertiesRefusal(trackProperties)) {
		case "malformed-track":
			return { outcome: "malformed-track", searched: "-" };
		case "unsupported-track-property":
			return { outcome: "unsupported-track", searched: "-" };
		case undefined: {
			const reading = propertiesToSearch(trackProperties, "Track");
			if ("malformed" in reading) throw new Error(reading.malformed);
			return { outcome: "ok", searched: written(reading.properties) };
		}
	}
}

const ROWS = immutablePropertiesFixtures();

describe("Immutable Properties (§12.7)", () => {
	test("the shared table has the rows this test expects", () => {
		expect(ROWS.length).toBe(32);
	});

	test.each(ROWS)(
		"$of $name: $outcome, as the shared table says",
		({ of, location, propertiesHex, outcome, searched }) => {
			const read =
				of === "object" && location !== undefined
					? readObject(propertiesHex, location.groupId, location.objectId)
					: readTrack(propertiesHex);
			expect(read).toEqual({ outcome, searched });
		},
	);
});
