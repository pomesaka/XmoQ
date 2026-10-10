// Own browser client of the interop run: a publisher and a subscriber built on
// @xmoq/moqt. `run.ts` drives this page through `window.xmoq`, which has the
// same shape as the moqtail-ts page in peers/moqtail/browser so the driver can
// swap one for the other.

import {
	connect,
	type MoqtConnection,
	type PublishedTrack,
	type Subscription,
} from "@xmoq/moqt";
import type { Diagnostics } from "@xmoq/moqt/diagnostics";
import type { PageReceivedObject } from "./xmoq-page.ts";

/** Provisional: any value works, the relay maps it to its own alias per subscriber. */
const PUBLISH_TRACK_ALIAS = 1n;

function fromHex(hex: string): Uint8Array<ArrayBuffer> {
	if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(hex)) {
		throw new Error(`not a hex string of whole bytes: "${hex}"`);
	}
	const bytes = new Uint8Array(hex.length / 2);
	for (let i = 0; i < bytes.length; i++) {
		bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
	}
	return bytes;
}

function toHex(bytes: Uint8Array, separator: string): string {
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
		separator,
	);
}

const utf8 = new TextEncoder();

/** "xmoq/phase0" names the Track Namespace Fields "xmoq" and "phase0", as moqtail's `Tuple.fromUtf8Path` does. */
function namespaceFromPath(path: string): Uint8Array[] {
	return path.split("/").map((field) => utf8.encode(field));
}

/** JSON with bigint and bytes made printable, for the event log. */
function describe(value: unknown): string {
	return JSON.stringify(value, (_key, item: unknown) => {
		if (typeof item === "bigint") return item.toString();
		if (item instanceof Uint8Array) return `hex:${toHex(item, "")}`;
		return item;
	});
}

type Settled<T> = { ok: true; value: T } | { ok: false; error: unknown };

let connection: MoqtConnection | undefined;
let published: PublishedTrack | undefined;
let received: Promise<Settled<PageReceivedObject>> | undefined;
let subscribed: Subscription | undefined;

/** PUBLISH_DONE Status Code TRACK_ENDED (§10.11). */
const TRACK_ENDED = 0x2n;

function connected(): MoqtConnection {
	if (connection === undefined) throw new Error("connect() has not completed");
	return connection;
}

/** Logs what the session does, and the bytes the fixtures are taken from. */
const diagnostics: Diagnostics = {
	onEvent(event) {
		// Object events carry the payload; their size is enough for the log.
		if (event.type === "object") {
			console.log(
				`event object: track_alias=${event.trackAlias} group=${event.groupId} subgroup=${event.subgroupId} object=${event.object.objectId} status=${event.object.status} payload_len=${event.object.payload.length}`,
			);
		} else {
			console.log(`event ${describe(event)}`);
		}
	},
	onBytes(direction, stream, data) {
		console.log(
			`bytes ${direction} ${stream} (${data.length}): ${toHex(data, " ")}`,
		);
	},
};

/**
 * The first Object with a payload. When the subscription ends before one
 * arrives, the error says how it ended: `objects` fails with the ending if the
 * subscription was cut off, and `ended` has it if the stream closed.
 */
async function firstPayload(
	subscription: Subscription,
): Promise<PageReceivedObject> {
	const reader = subscription.objects.getReader();
	try {
		for (;;) {
			const result = await reader.read();
			if (result.done) {
				const end = await subscription.ended;
				throw new Error(
					`the subscription ended before any Object with a payload: ${describe(end)}`,
				);
			}
			const { groupId, object } = result.value;
			if (object.payload.length === 0) continue;
			return {
				group: groupId.toString(),
				object: object.objectId.toString(),
				payloadHex: toHex(object.payload, ""),
			};
		}
	} finally {
		reader.releaseLock();
	}
}

/**
 * WHY rethrow as a plain Error: with playwright-core 1.64.0, `page.evaluate`
 * resolved with `undefined` when the page promise rejected with an Error that
 * has a bigint property, as `RequestFailedError` does. The driver would then
 * carry on as if the call had succeeded.
 */
function withPlainErrors<Args extends unknown[], Result>(
	method: (...args: Args) => Promise<Result>,
): (...args: Args) => Promise<Result> {
	return async (...args) => {
		try {
			return await method(...args);
		} catch (error) {
			throw new Error(
				error instanceof Error
					? `${error.name}: ${error.message}`
					: String(error),
			);
		}
	};
}

window.xmoq = {
	connect: withPlainErrors(
		async (relayUrl: string, certSha256Hex: string | null) => {
			connection = await connect(relayUrl, {
				implementation: "xmoq-moqt/0.0.0",
				// WHY serverCertificateHashes: the relay's certificate is self-signed and
				// no root store trusts it. Pinning needs no browser flag and no system CA.
				...(certSha256Hex === null
					? {}
					: {
							serverCertificateHashes: [
								{ algorithm: "sha-256", value: fromHex(certSha256Hex) },
							],
						}),
				diagnostics,
			});
			console.log(
				`connected: peer_setup_options=${describe(connection.peerSetupOptions)}`,
			);
		},
	),

	publishTrack: withPlainErrors(
		async (namespace: string, trackName: string) => {
			const moq = connected();
			const trackNamespace = namespaceFromPath(namespace);
			await moq.publishNamespace(trackNamespace);
			published = await moq.publish({
				trackNamespace,
				trackName: utf8.encode(trackName),
				trackAlias: PUBLISH_TRACK_ALIAS,
				parameters: { forward: true },
			});
			console.log(`track published: alias=${published.trackAlias}`);
			return published.trackAlias.toString();
		},
	),

	sendObject: withPlainErrors(async (payloadHex: string) => {
		if (published === undefined)
			throw new Error("publishTrack() has not completed");
		const subgroup = await published.openSubgroup({
			groupId: 0n,
			subgroupId: { mode: "Explicit", value: 0n },
			publisherPriority: 0,
			properties: false,
			// The one Object sent is the largest in its Group.
			endOfGroup: true,
		});
		await subgroup.write({ objectId: 0n, payload: fromHex(payloadHex) });
		await subgroup.close();
		console.log(
			`object sent: group=0 object=0 payload_len=${payloadHex.length / 2}`,
		);
	}),

	subscribe: withPlainErrors(async (namespace: string, trackName: string) => {
		const subscription = await connected().subscribe({
			trackNamespace: namespaceFromPath(namespace),
			trackName: utf8.encode(trackName),
			// The parameters the moqtail Rust client in peers/moqtail/rust-client sends.
			parameters: {
				forward: true,
				subscriberPriority: 128,
				subscriptionFilter: { filterType: "LargestObject" },
				groupOrder: "Ascending",
			},
		});
		// WHY settle here instead of keeping the promise: the read can fail before
		// the driver calls firstObject(), and a promise rejected with nobody
		// awaiting it is reported as an unhandled rejection instead of reaching
		// the driver.
		received = firstPayload(subscription).then(
			(value) => ({ ok: true, value }),
			(error: unknown) => ({ ok: false, error }),
		);
		subscribed = subscription;
		console.log(
			`subscribed: request_id=${subscription.requestId} alias=${subscription.trackAlias}`,
		);
		return subscription.requestId.toString();
	}),

	firstObject: withPlainErrors(async () => {
		if (received === undefined)
			throw new Error("subscribe() has not completed");
		const outcome = await received;
		if (!outcome.ok) throw outcome.error;
		return outcome.value;
	}),

	disconnect: withPlainErrors(async () => {
		connection?.close();
		connection = undefined;
	}),
};

window.xmoqOwn = {
	endTrack: withPlainErrors(async () => {
		if (published === undefined)
			throw new Error("publishTrack() has not completed");
		await published.done(TRACK_ENDED, "");
		console.log(`track ended: ${describe(await published.ended)}`);
	}),

	subscriptionEnd: withPlainErrors(async () => {
		if (subscribed === undefined)
			throw new Error("subscribe() has not completed");
		return describe(await subscribed.ended);
	}),
};

console.log("page ready");
