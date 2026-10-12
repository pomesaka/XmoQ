import { expect, test } from "bun:test";
import {
	type ControlMessage,
	decodeControlMessage,
	encodeControlMessage,
} from "../../src/core/control-message.ts";
import {
	fromALibrary,
	fromALibraryNames,
	hex,
	toHex,
	utf8,
} from "../helpers.ts";

// A SUBSCRIBE with the delivery timeouts of §10.2.3 and §10.2.4, as
// moqtail npm 0.13.0 serializes it. The bytes are in
// fixtures/moqt-18-from-libraries/, which the Rust implementation reads
// too; its README has how they were made. WHY not fixtures/moqt-18/: no
// client of moqtail sends these parameters, so no wire carried them.
const SUBSCRIBE = {
	type: "SUBSCRIBE",
	requestId: 0n,
	trackNamespace: [utf8("xmoq"), utf8("phase0")],
	trackName: utf8("object"),
} as const;

// The REQUEST_UPDATEs are what a subscriber sends to change the timeout of
// its subscription (§10.9), as the same package serializes them.
const FILES: { file: string; message: ControlMessage }[] = [
	{
		file: "subscribe-delivery-timeouts-moqtail-ts",
		message: {
			...SUBSCRIBE,
			parameters: {
				objectDeliveryTimeout: 250n,
				subgroupDeliveryTimeout: 500n,
				subscriptionFilter: { filterType: "LargestObject" },
			},
		},
	},
	{
		file: "subscribe-subgroup-delivery-timeout-moqtail-ts",
		message: {
			...SUBSCRIBE,
			parameters: {
				subgroupDeliveryTimeout: 500n,
				subscriptionFilter: { filterType: "LargestObject" },
			},
		},
	},
	{
		file: "request-update-subgroup-delivery-timeout-moqtail-ts",
		message: {
			type: "REQUEST_UPDATE",
			requestId: 2n,
			parameters: { subgroupDeliveryTimeout: 250n },
		},
	},
	{
		file: "request-update-no-subgroup-delivery-timeout-moqtail-ts",
		message: {
			type: "REQUEST_UPDATE",
			requestId: 2n,
			parameters: { subgroupDeliveryTimeout: 0n },
		},
	},
];

test("every file a library wrote is read here", () => {
	expect(FILES.map(({ file }) => file).sort()).toEqual(fromALibraryNames());
});

test.each(FILES)(
	"$file decodes to its value and is encoded to the same bytes",
	({ file, message }) => {
		const bytes = fromALibrary(file);
		const input = hex(bytes);
		expect(decodeControlMessage(input)).toEqual({
			status: "ok",
			value: message,
			bytesRead: input.length,
		});
		expect(toHex(encodeControlMessage(message))).toBe(bytes);
	},
);
