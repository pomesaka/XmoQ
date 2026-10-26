import { expect, test } from "bun:test";
import type {
	MessageParameters,
	MoqtConnection,
	ReceivedObject,
	SubscribeRequest,
	Subscription,
	SubscriptionEnd,
} from "@xmoq/moqt";
import { joinChat } from "../src/moqt/participant.ts";

/**
 * A connection that records what `joinChat` asks of it: the PUBLISH_NAMESPACE
 * is answered, and the SUBSCRIBE for `lines` is refused, so that no FETCH
 * follows. The connection's own token is the library's to add
 * (`@xmoq/moqt`'s tests); what is looked at here is what `joinChat` gives
 * each request.
 */
function recording(): {
	connection: Pick<
		MoqtConnection,
		"serve" | "publishNamespace" | "subscribe" | "fetch" | "close" | "closed"
	>;
	publishNamespace: (MessageParameters | undefined)[];
	subscribed: Promise<SubscribeRequest>;
} {
	const publishNamespace: (MessageParameters | undefined)[] = [];
	const { promise: subscribed, resolve } =
		Promise.withResolvers<SubscribeRequest>();
	return {
		publishNamespace,
		subscribed,
		connection: {
			serve() {},
			async publishNamespace(_namespace, parameters) {
				publishNamespace.push(parameters);
			},
			async subscribe(request) {
				resolve(request);
				throw new Error("refused by the test");
			},
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
			closed: new Promise(() => {}),
		},
	};
}

const hex = (bytes: Uint8Array): string =>
	[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");

test.each<{
	name: string;
	publishNamespaceToken: Uint8Array | undefined;
	publishNamespaceTokens: string[] | undefined;
}>([
	{
		name: "a token for the PUBLISH_NAMESPACE goes on it, and on no SUBSCRIBE",
		publishNamespaceToken: Uint8Array.of(0x03, 0x00, 0x70),
		publishNamespaceTokens: ["030070"],
	},
	{
		name: "without one, the PUBLISH_NAMESPACE carries no parameters",
		publishNamespaceToken: undefined,
		publishNamespaceTokens: undefined,
	},
])("$name", async ({ publishNamespaceToken, publishNamespaceTokens }) => {
	const { connection, publishNamespace, subscribed } = recording();
	const stop = new AbortController();
	await joinChat(connection, {
		room: "blue",
		name: "ann",
		publishNamespaceToken,
		onLine() {},
		signal: stop.signal,
	});
	const subscribe = await subscribed;
	stop.abort();
	expect({
		publishNamespace: publishNamespace.map((parameters) =>
			parameters?.authorizationToken?.map(hex),
		),
		subscribe: subscribe.parameters?.authorizationToken,
	}).toEqual({
		publishNamespace: [publishNamespaceTokens],
		subscribe: undefined,
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

test("a subscription that is made after the participant stopped is cancelled", async () => {
	const late = answeredLate();
	const stop = new AbortController();
	await joinChat(
		{
			serve() {},
			async publishNamespace() {},
			subscribe: () => late.subscribe(),
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
			closed: new Promise(() => {}),
		},
		{ room: "blue", name: "ann", onLine() {}, signal: stop.signal },
	);
	await late.asked;
	stop.abort();
	late.accept();
	for (let turn = 0; turn < 10; turn++) await Promise.resolve();
	expect(late.cancelled()).toBe(1);
});
