// Where a page connects, and how: the relay with its certificate's hash, a
// token of a room, and what a refusal is said as. Shared by the
// applications of the demo.

import { connect, type MoqtConnection } from "@xmoq/moqt";

/** Where a broadcast is: what a viewer needs to connect to it. */
export interface Broadcast {
	relayUrl: string;
	certSha256Hex: string;
	namespace: string[];
}


/** §10.2.2: Alias Type USE_VALUE (0x3), Token Type 0, the text the room gave. */
export const tokenOf = (text: string | null): Uint8Array | undefined =>
	text === null
		? undefined
		: new Uint8Array([0x03, 0x00, ...new TextEncoder().encode(text)]);

/**
 * `?via=PORT`: the relay through another port of this machine, where
 * something stands in between (`spikes/lossy-link`).
 */
export function through(broadcast: Broadcast): Broadcast {
	const via = new URLSearchParams(location.search).get("via");
	if (via === null) return broadcast;
	const url = new URL(broadcast.relayUrl);
	url.port = via;
	return { ...broadcast, relayUrl: url.href };
}

/** With `?diag=1`: when the first bytes of each stream of the peer came, by this page's clock, and how many they were. */
export const firstBytes = new Map<string, { at: number; length: number }>();

/**
 * Connects to the relay of a broadcast. With a token, every request of the
 * connection carries it (`authorizationToken` of `connect`).
 */
export function connectToRelay(
	to: Broadcast,
	token?: Uint8Array,
): Promise<MoqtConnection> {
	const hash = Uint8Array.from(to.certSha256Hex.match(/../g) ?? [], (byte) =>
		Number.parseInt(byte, 16),
	);
	return connect(to.relayUrl, {
		implementation: "xmoq-demo/0.0.0",
		...(token === undefined ? {} : { authorizationToken: token }),
		// WHY: the relay's certificate is self-signed. Chromium takes it when
		// its hash is given, and it is valid for less than two weeks.
		serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
		// What the README of @xmoq/media asks of a viewer.
		limits: { maxObjectPayloadLength: 1024 * 1024 },
		// `?uni=N`: how many streams of the peer are read at once, for
		// looking into where something waits; the library's when left out.
		...(new URLSearchParams(location.search).get("uni") === null
			? {}
			: {
					maxPeerUniStreams: Number(
						new URLSearchParams(location.search).get("uni"),
					),
				}),
		// `?diag=1`: when the first bytes of every stream of the peer came,
		// for looking into where something waits.
		...(new URLSearchParams(location.search).get("diag") === null
			? {}
			: {
					diagnostics: {
						onBytes(direction: string, stream: string, data: Uint8Array) {
							if (direction !== "received" || firstBytes.has(stream)) return;
							firstBytes.set(stream, { at: Date.now(), length: data.length });
						},
					},
				}),
	});
}

/** What went wrong, for the line under the picture: a refusal for want of a token is said as that. */
export const why = (error: unknown): string =>
	/REQUEST_ERROR 0x1:/.test(String(error))
		? "the relay refused: not authorized (no token of the room, or one it does not take)"
		: /closed the session: code=(2|22|24) /.test(String(error))
			? `the relay closed the session: not authorized (${String(error).slice(0, 90)})`
			: String(error).slice(0, 120);

