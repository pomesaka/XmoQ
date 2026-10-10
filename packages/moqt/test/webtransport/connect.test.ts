import { expect, test } from "bun:test";
import {
	createStreamError,
	protocolRefusal,
} from "../../src/webtransport/connect.ts";
import { FakeWebTransportError } from "./fake-transport.ts";

// §3.1: only a session that negotiated "moqt-18" speaks draft 18.
test.each([
	{ protocol: "moqt-18", refusal: undefined },
	{
		protocol: "moqt-17",
		refusal: "the server selected protocol moqt-17, not moqt-18",
	},
	{
		// A browser without `WebTransport.protocol`, or a server that chose none.
		protocol: undefined,
		refusal: "the server selected protocol undefined, not moqt-18",
	},
	{ protocol: "", refusal: "the server selected protocol , not moqt-18" },
])("protocolRefusal($protocol)", ({ protocol, refusal }) => {
	expect(protocolRefusal(protocol)).toBe(refusal);
});

/** `WebTransportError` as the W3C specification has it (§12): a message, then options. */
class SpecifiedWebTransportError extends Error {
	/** How many were built, by any call. */
	static built = 0;
	readonly source: string;
	readonly streamErrorCode: number | null;

	constructor(message: unknown = "", options: unknown = {}) {
		SpecifiedWebTransportError.built += 1;
		// WebIDL turns whatever stands first into a string.
		super(String(message));
		if (typeof options !== "object" || options === null) {
			throw new TypeError("options is not a dictionary");
		}
		const code = "streamErrorCode" in options ? options.streamErrorCode : null;
		this.streamErrorCode = typeof code === "number" ? code : null;
		this.source = "stream";
	}
}

/** Takes neither form. */
class UnusableWebTransportError extends Error {
	readonly source = "stream";
	readonly streamErrorCode = null;

	constructor() {
		super("unusable");
		throw new TypeError("no form is accepted");
	}
}

test.each([
	{
		form: "one dictionary, as Chromium 156",
		errorConstructor: FakeWebTransportError,
	},
	{
		form: "message and options, as specified",
		errorConstructor: SpecifiedWebTransportError,
	},
])(
	"createStreamError builds an error with its code and message: $form",
	({ errorConstructor }) => {
		const error = createStreamError(
			errorConstructor,
			18,
			"reset by the MOQT session",
		);
		expect(error).toBeInstanceOf(errorConstructor);
		expect(error.streamErrorCode).toBe(18);
		expect(error.source).toBe("stream");
		expect(error instanceof Error ? error.message : undefined).toBe(
			"reset by the MOQT session",
		);
	},
);

test("createStreamError fails when the browser takes neither form", () => {
	expect(() =>
		createStreamError(UnusableWebTransportError, 18, "reset"),
	).toThrow(
		"WebTransportError could not be built with stream error code 18 in either of its two forms",
	);
});

// WHY the order is pinned: a browser that follows the specification takes a
// dictionary given first for the message and builds an error without a code.
// Trying the dictionary first would build that error and throw it away on
// every call, and would send code 0 if the code were not compared.
test("createStreamError tries the form of the specification first", () => {
	SpecifiedWebTransportError.built = 0;
	createStreamError(SpecifiedWebTransportError, 18, "reset");
	expect(SpecifiedWebTransportError.built).toBe(1);
});

// Chromium 156 clamps the code to 255: an error with another code than the
// one asked for must not be used, or the peer is told the wrong reason.
test("createStreamError fails for a code the browser does not keep", () => {
	expect(() => createStreamError(FakeWebTransportError, 256, "reset")).toThrow(
		"WebTransportError could not be built with stream error code 256 in either of its two forms",
	);
});
