import { expect, test } from "bun:test";
import type {
	MoqtConnection,
	ReceivedObject,
	SubscribeRequest,
	Subscription,
	SubscriptionEnd,
} from "@xmoq/moqt";
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

/**
 * A SUBSCRIBE whose answer comes when the test says: `accept()` resolves it
 * with a subscription that ends, as `cancelled`, when it is cancelled, and
 * whose `objects` closes then, as `@xmoq/moqt`'s does.
 */
function answeredLate(): {
	subscribe(): Promise<Subscription>;
	asked: Promise<void>;
	accept(): void;
	cancelled(): number;
} {
	let cancelled = 0;
	const { promise: asked, resolve: ask } = Promise.withResolvers<void>();
	const { promise: answer, resolve } = Promise.withResolvers<Subscription>();
	const { promise: ended, resolve: end } =
		Promise.withResolvers<SubscriptionEnd>();
	let close: () => void = () => {};
	const objects = new ReadableStream<ReceivedObject>({
		start(controller) {
			close = () => controller.close();
		},
	});
	return {
		subscribe() {
			ask();
			return answer;
		},
		asked,
		accept() {
			resolve({
				requestId: 0n,
				trackAlias: 0n,
				parameters: {},
				trackProperties: [],
				objects,
				ended,
				goAway: new Promise(() => {}),
				async cancel() {
					cancelled += 1;
					close();
					end({ type: "cancelled" });
				},
				update: () => Promise.reject(new Error("no update in this test")),
			});
		},
		cancelled: () => cancelled,
	};
}

test("a subscription that is made after the reader stopped is cancelled", async () => {
	const late = answeredLate();
	const reader = followTranscript(
		{
			subscribe: () => late.subscribe(),
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
		},
		{ listened: ["xmoq", "demo"], onSpeech() {} },
	);
	await late.asked;
	reader.stop();
	late.accept();
	for (let turn = 0; turn < 10; turn++) await Promise.resolve();
	expect(late.cancelled()).toBe(1);
});
