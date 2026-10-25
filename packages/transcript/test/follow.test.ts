import { expect, test } from "bun:test";
import type { MoqtConnection, SubscribeRequest } from "@xmoq/moqt";
import { followTranscript } from "../src/moqt/follow.ts";

/** A connection whose SUBSCRIBE is refused, so that no FETCH follows; it records the request. */
function refusing(): {
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "close">;
	subscribed: Promise<SubscribeRequest>;
} {
	const { promise: subscribed, resolve } =
		Promise.withResolvers<SubscribeRequest>();
	return {
		connection: {
			async subscribe(request) {
				resolve(request);
				throw new Error("refused by the test");
			},
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
		},
		subscribed,
	};
}

test("the reader subscribes to speech of xmoq/transcript/<prefix>, and waits for it to be published", async () => {
	const { connection, subscribed } = refusing();
	const reader = followTranscript(connection, {
		listened: ["xmoq", "demo"],
		onSpeech() {},
	});
	const request = await subscribed;
	reader.stop();
	const text = (bytes: Uint8Array): string => new TextDecoder().decode(bytes);
	expect({
		namespace: request.trackNamespace.map(text),
		name: text(request.trackName),
		rendezvousMs: request.parameters?.rendezvousTimeout,
	}).toEqual({
		namespace: ["xmoq", "transcript", "xmoq", "demo"],
		name: "speech",
		rendezvousMs: 10_000n,
	});
});
