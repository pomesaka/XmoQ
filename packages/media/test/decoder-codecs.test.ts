import { describe, expect, test } from "bun:test";
import { decoderCodecs } from "../src/core/decoder-codecs.ts";

describe("decoderCodecs", () => {
	test.each([
		["avc3.42E01F", ["avc3.42E01F", "avc1.42E01F"]],
		["avc1.42E01F", ["avc1.42E01F"]],
		["vp09.00.10.08", ["vp09.00.10.08"]],
	])("%s is asked for as %j", (codec, expected) => {
		expect(decoderCodecs(codec)).toEqual(expected);
	});
});
