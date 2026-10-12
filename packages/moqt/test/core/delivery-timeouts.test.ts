import { expect, test } from "bun:test";
import {
	type ControlMessage,
	decodeControlMessage,
	encodeControlMessage,
} from "../../src/core/control-message.ts";
import { hex, toHex, utf8 } from "../helpers.ts";

// A SUBSCRIBE with the delivery timeouts of §10.2.3 and §10.2.4, as
// moqtail npm 0.13.0 serializes it (`Subscribe.newLatestObject(0n, name,
// [new ObjectDeliveryTimeout(250n), new SubgroupDeliveryTimeout(500n)])`,
// run on 2026-10-12). WHY here and not in fixtures/moqt-18/: no client of
// moqtail sends these parameters, so the bytes were never on a wire, and
// every file there is a capture that the Rust implementation reads too.
test.each<{
	name: string;
	bytes: string;
	parameters: Extract<ControlMessage, { type: "SUBSCRIBE" }>["parameters"];
}>([
	{
		name: "OBJECT_DELIVERY_TIMEOUT 250 and SUBGROUP_DELIVERY_TIMEOUT 500",
		bytes:
			"03 00 1f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 03 02 80 fa 04 81 f4 1b 01 02",
		parameters: {
			objectDeliveryTimeout: 250n,
			subgroupDeliveryTimeout: 500n,
			subscriptionFilter: { filterType: "LargestObject" },
		},
	},
	{
		name: "SUBGROUP_DELIVERY_TIMEOUT 500 alone",
		bytes:
			"03 00 1c 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 02 06 81 f4 1b 01 02",
		parameters: {
			subgroupDeliveryTimeout: 500n,
			subscriptionFilter: { filterType: "LargestObject" },
		},
	},
])(
	"a SUBSCRIBE with $name, as moqtail-ts writes it",
	({ bytes, parameters }) => {
		const message: ControlMessage = {
			type: "SUBSCRIBE",
			requestId: 0n,
			trackNamespace: [utf8("xmoq"), utf8("phase0")],
			trackName: utf8("object"),
			parameters,
		};
		const input = hex(bytes);
		expect(decodeControlMessage(input)).toEqual({
			status: "ok",
			value: message,
			bytesRead: input.length,
		});
		expect(toHex(encodeControlMessage(message))).toBe(bytes);
	},
);
