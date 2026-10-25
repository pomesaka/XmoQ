import { describe, expect, test } from "bun:test";
import {
	decodeControlMessage,
	encodeControlMessage,
} from "../../src/core/control-message.ts";
import { MAX_NAMESPACES_OF_A_SUBSCRIPTION } from "../../src/core/requests.ts";
import { createSession, type Session } from "../../src/core/session.ts";
import type { SessionOutput } from "../../src/core/session-output.ts";
import { fromALibrary, hex, toHex, utf8 } from "../helpers.ts";

// SUBSCRIBE_NAMESPACE of this endpoint (§6.1, §10.16 to §10.18): REQUEST_OK
// or REQUEST_ERROR first, then NAMESPACE and NAMESPACE_DONE with the suffix
// after the prefix. The same rules as the Rust session (`on_namespace`,
// `answered_namespaces`), with moqtail's bytes (`fixtures/moqt-18-from-libraries/`).

/** moqtail's SUBSCRIBE_NAMESPACE for (`xmoq`, `chat`), with Request ID 1. */
const SUBSCRIBE_NAMESPACE = hex(fromALibrary("subscribe-namespace-moqtail-ts"));
/** moqtail's SUBSCRIBE_NAMESPACE for every namespace, with Request ID 3. */
const EVERY_NAMESPACE = hex(
	fromALibrary("subscribe-namespace-every-namespace-moqtail-ts"),
);
/** moqtail's NAMESPACE and NAMESPACE_DONE of the suffix (`room`, `alice`). */
const NAMESPACE = fromALibrary("namespace-moqtail-ts");
const NAMESPACE_DONE = fromALibrary("namespace-done-moqtail-ts");

/** REQUEST_OK without a parameter or a property. */
const REQUEST_OK = "07 00 01 00";

function started(): Session {
	const session = createSession({});
	session.start();
	return session;
}

/** Bytes with the Request ID at their fourth byte set to 0, as the first request of a client has. */
function withRequestIdZero(bytes: Uint8Array): Uint8Array {
	const copy = bytes.slice();
	copy[3] = 0;
	return copy;
}

const text = (namespace: readonly Uint8Array[]): string =>
	namespace.map((field) => new TextDecoder().decode(field)).join("/");

/** An output in a line. */
function brief(output: SessionOutput): string {
	switch (output.type) {
		case "open-bidi-stream": {
			const decoded = decodeControlMessage(output.data);
			return `open bidi ${output.stream}: ${decoded.status === "ok" ? decoded.value.type : "?"} ${toHex(output.data)}`;
		}
		case "finish":
			return `finish ${output.stream}`;
		case "reset":
			return `reset ${output.stream} code ${output.code}`;
		case "close-session":
			return `close ${output.code}`;
		case "event": {
			const { event } = output;
			switch (event.type) {
				case "namespace":
					return `${event.done ? "done" : "namespace"} ${event.requestId} ${text(event.trackNamespace)}`;
				case "request-error":
					return `event request-error ${event.requestId} code ${event.errorCode}`;
				case "request-closed":
					return `event request-closed ${event.requestId} ${event.reason}`;
				case "namespaces-subscribed":
					return `event namespaces-subscribed ${event.requestId}`;
				default:
					return `event ${event.type}`;
			}
		}
		default:
			return output.type;
	}
}

const briefly = (outputs: SessionOutput[]): string[] => outputs.map(brief);

describe("a SUBSCRIBE_NAMESPACE of this endpoint", () => {
	test.each([
		{
			name: "for (xmoq, chat), as moqtail writes it",
			prefix: ["xmoq", "chat"],
			bytes: withRequestIdZero(SUBSCRIBE_NAMESPACE),
		},
		{
			name: "for every namespace, a prefix of no field (§6.1)",
			prefix: [],
			bytes: withRequestIdZero(EVERY_NAMESPACE),
		},
	])("is sent on a request stream of its own: $name", ({ prefix, bytes }) => {
		const { value, outputs } = started().subscribeNamespace({
			trackNamespacePrefix: prefix.map(utf8),
		});
		expect(value).toBe(0n);
		// Stream 0 is the control stream.
		expect(briefly(outputs)).toEqual([
			`open bidi 1: SUBSCRIBE_NAMESPACE ${toHex(bytes)}`,
		]);
	});

	test.each([
		{
			// §10.2.2 lists SUBSCRIBE_NAMESPACE for AUTHORIZATION_TOKEN alone.
			name: "a parameter other than AUTHORIZATION_TOKEN",
			prefix: ["xmoq"],
			parameters: { rendezvousTimeout: 1n },
		},
		{
			name: "a prefix of 33 fields (§10.18: between 0 and 32)",
			prefix: Array.from({ length: 33 }, () => "a"),
			parameters: {},
		},
	])("is refused at the call for $name", ({ prefix, parameters }) => {
		const session = started();
		expect(() =>
			session.subscribeNamespace({
				trackNamespacePrefix: prefix.map(utf8),
				parameters,
			}),
		).toThrow(RangeError);
		// No Request ID was used up.
		expect(session.subscribeNamespace({ trackNamespacePrefix: [] }).value).toBe(
			0n,
		);
	});

	// Each step is what the peer does on the request stream of the
	// SUBSCRIBE_NAMESPACE for (xmoq, chat), in a row, and what the session
	// does with it.
	test.each([
		{
			name: "answered, told of a namespace, and of its end",
			steps: [
				{ bytes: REQUEST_OK, outputs: ["event namespaces-subscribed 0"] },
				{ bytes: NAMESPACE, outputs: ["namespace 0 xmoq/chat/room/alice"] },
				{ bytes: NAMESPACE_DONE, outputs: ["done 0 xmoq/chat/room/alice"] },
				// §10.18: a NAMESPACE_DONE before its NAMESPACE is a
				// PROTOCOL_VIOLATION, and one after the end has none.
				{ bytes: NAMESPACE_DONE, outputs: ["close 3"] },
			],
		},
		{
			// The draft does not make it an error; the Rust session tells it
			// again.
			name: "told of a namespace twice",
			steps: [
				{
					bytes: `${REQUEST_OK} ${NAMESPACE} ${NAMESPACE}`,
					outputs: [
						"event namespaces-subscribed 0",
						"namespace 0 xmoq/chat/room/alice",
						"namespace 0 xmoq/chat/room/alice",
					],
				},
				{ bytes: NAMESPACE_DONE, outputs: ["done 0 xmoq/chat/room/alice"] },
			],
		},
		{
			name: "rejected",
			steps: [
				{
					// REQUEST_ERROR PREFIX_OVERLAP, no Retry Interval, no reason.
					bytes: "05 00 03 30 00 00",
					outputs: ["event request-error 0 code 48", "finish 1"],
				},
			],
		},
		{
			// §10.18: "If the subscriber receives any message other than a
			// REQUEST_OK or a REQUEST_ERROR as the first message ... it MUST
			// close the session with a PROTOCOL_VIOLATION."
			name: "told of a namespace before its answer",
			steps: [{ bytes: NAMESPACE, outputs: ["close 3"] }],
		},
		{
			name: "told of the end of a namespace it was not told of",
			steps: [
				{ bytes: REQUEST_OK, outputs: ["event namespaces-subscribed 0"] },
				{ bytes: NAMESPACE_DONE, outputs: ["close 3"] },
			],
		},
		{
			// No parameter of §10.2 lists this REQUEST_OK.
			name: "answered with a REQUEST_OK that carries FORWARD",
			steps: [{ bytes: "07 00 03 01 10 01", outputs: ["close 3"] }],
		},
		{
			name: "answered with a REQUEST_OK that carries a Track Property",
			steps: [{ bytes: "07 00 03 00 02 05", outputs: ["close 3"] }],
		},
		{
			name: "sent a SUBSCRIBE_OK after its answer",
			steps: [
				{ bytes: REQUEST_OK, outputs: ["event namespaces-subscribed 0"] },
				{ bytes: "04 00 02 01 00", outputs: ["close 3"] },
			],
		},
	] satisfies {
		name: string;
		steps: { bytes: string; outputs: string[] }[];
	}[])("$name", ({ steps }) => {
		const session = started();
		session.subscribeNamespace({
			trackNamespacePrefix: [utf8("xmoq"), utf8("chat")],
		});
		for (const { bytes, outputs } of steps) {
			expect(
				briefly(session.receiveRequestStreamData(1, hex(bytes), false)),
			).toEqual(outputs);
		}
	});

	// After the answer and one namespace: how the request ends.
	test.each([
		{
			name: "the peer finishes its direction",
			end: (session: Session) =>
				session.receiveRequestStreamData(1, new Uint8Array(0), true),
			outputs: ["event request-closed 0 peer-fin", "finish 1"],
		},
		{
			name: "the peer resets its direction",
			end: (session: Session) => session.requestStreamReset(1, 0),
			outputs: ["event request-closed 0 peer-reset", "reset 1 code 1"],
		},
		{
			name: "this endpoint cancels it",
			end: (session: Session) => session.cancelRequest(0n),
			outputs: ["reset 1 code 1"],
		},
	] satisfies {
		name: string;
		end: (session: Session) => SessionOutput[];
		outputs: string[];
	}[])("ends where $name", ({ end, outputs }) => {
		const session = started();
		session.subscribeNamespace({
			trackNamespacePrefix: [utf8("xmoq"), utf8("chat")],
		});
		session.receiveRequestStreamData(
			1,
			hex(`${REQUEST_OK} ${NAMESPACE}`),
			false,
		);
		expect(briefly(end(session))).toEqual(outputs);
		// Nothing of it is kept: what comes later on the stream is not read.
		expect(
			briefly(session.receiveRequestStreamData(1, hex(NAMESPACE), false)),
		).toEqual([]);
	});

	test("hands on the whole namespace of a NAMESPACE", () => {
		const session = started();
		session.subscribeNamespace({ trackNamespacePrefix: [utf8("xmoq")] });
		const outputs = session.receiveRequestStreamData(
			1,
			hex(`${REQUEST_OK} ${NAMESPACE}`),
			false,
		);
		expect(outputs.at(-1)).toEqual({
			type: "event",
			event: {
				type: "namespace",
				requestId: 0n,
				done: false,
				trackNamespace: [utf8("xmoq"), utf8("room"), utf8("alice")],
			},
		});
	});

	test("is cancelled alone when told of one namespace more than it keeps", () => {
		expect(MAX_NAMESPACES_OF_A_SUBSCRIPTION).toBe(4096);
		const session = started();
		session.subscribeNamespace({ trackNamespacePrefix: [utf8("xmoq")] });
		session.receiveRequestStreamData(1, hex(REQUEST_OK), false);
		const namespace = (index: number): Uint8Array =>
			encodeControlMessage({
				type: "NAMESPACE",
				trackNamespaceSuffix: [utf8(`p${index}`)],
			});
		for (let index = 0; index < 4096; index++) {
			session.receiveRequestStreamData(1, namespace(index), false);
		}
		// One told of again is no more.
		expect(
			briefly(session.receiveRequestStreamData(1, namespace(0), false)),
		).toEqual(["namespace 0 xmoq/p0"]);
		expect(
			briefly(session.receiveRequestStreamData(1, namespace(4096), false)),
		).toEqual(["event request-closed 0 too-many-namespaces", "reset 1 code 1"]);
		// The session goes on.
		expect(session.closed).toBe(false);
	});

	test("closes the session for a namespace of more than 32 fields, with the prefix", () => {
		const session = started();
		session.subscribeNamespace({ trackNamespacePrefix: [utf8("xmoq")] });
		session.receiveRequestStreamData(1, hex(REQUEST_OK), false);
		const suffix = encodeControlMessage({
			type: "NAMESPACE",
			trackNamespaceSuffix: Array.from({ length: 32 }, () => utf8("a")),
		});
		expect(briefly(session.receiveRequestStreamData(1, suffix, false))).toEqual(
			["close 3"],
		);
	});
});
