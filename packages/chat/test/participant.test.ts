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
		room: { kind: "broadcast", name: "blue" },
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
		{
			room: { kind: "broadcast", name: "blue" },
			name: "ann",
			onLine() {},
			signal: stop.signal,
		},
	);
	await late.asked;
	stop.abort();
	late.accept();
	for (let turn = 0; turn < 10; turn++) await Promise.resolve();
	expect(late.cancelled()).toBe(1);
});

/** A subscription that has the Objects given, in their Groups, and ends after them. */
function subscriptionOf(
	payloads: string[],
	parameters: Subscription["parameters"] = {},
): Subscription {
	const objects = new ReadableStream<ReceivedObject>({
		start(controller) {
			for (const [group, payload] of payloads.entries()) {
				controller.enqueue({
					groupId: BigInt(group),
					subgroupId: 0n,
					publisherPriority: undefined,
					object: {
						objectId: 0n,
						properties: [],
						status: "Normal",
						payload: new TextEncoder().encode(payload),
					},
				});
			}
			controller.close();
		},
	});
	return {
		requestId: 0n,
		trackAlias: 0n,
		parameters,
		trackProperties: [],
		objects,
		ended: Promise.resolve({ type: "cancelled" }),
		goAway: new Promise(() => {}),
		cancel: async () => {},
		update: () => Promise.reject(new Error("no update in this test")),
	};
}

// The tracks of the room besides `lines` are subscribed to where the caller
// asks for them alone, and what is on them is handed on as it is read.
test.each<{
	name: string;
	reactions: boolean;
	present: boolean;
	asked: string[];
	got: string[];
}>([
	{
		name: "neither asked for: lines alone",
		reactions: false,
		present: false,
		asked: ["lines"],
		got: [],
	},
	{
		name: "both asked for, and what is no payload of its track passed over",
		reactions: true,
		present: true,
		asked: ["lines", "reactions", "present"],
		got: ["present 3 8", 'reactions [["👍",2]] 7'],
	},
])("$name", async ({ reactions, present, asked, got }) => {
	const subscribed: string[] = [];
	const handed: string[] = [];
	const stop = new AbortController();
	const tracks: Record<string, string[]> = {
		reactions: ['{"counts":{},"at":6}', '{"counts":{"👍":2},"at":7}'],
		present: ['{"count":-1,"at":5}', '{"count":3,"at":8}'],
	};
	await joinChat(
		{
			serve() {},
			async publishNamespace() {},
			async subscribe(request) {
				const name = new TextDecoder().decode(request.trackName);
				subscribed.push(name);
				const payloads = tracks[name];
				if (payloads === undefined) throw new Error("refused by the test");
				// Once: the next is never answered, so that the test ends.
				tracks[name] = [];
				if (payloads.length === 0) return new Promise(() => {});
				return subscriptionOf(payloads);
			},
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
			closed: new Promise(() => {}),
		},
		{
			room: { kind: "broadcast", name: "blue" },
			name: "ann",
			onLine() {},
			...(reactions
				? {
						onReactions: (read) =>
							handed.push(
								`reactions ${JSON.stringify([...read.counts])} ${read.at}`,
							),
					}
				: {}),
			...(present
				? {
						onPresent: (read) =>
							handed.push(`present ${read.count} ${read.at}`),
					}
				: {}),
			signal: stop.signal,
		},
	);
	for (let turn = 0; turn < 50; turn++) await Promise.resolve();
	stop.abort();
	expect([new Set(subscribed), handed.sort()]).toEqual([new Set(asked), got]);
});

test.each<{ name: string; emoji: string; outcome: unknown }>([
	{
		name: "an empty emoji is kept back",
		emoji: "",
		outcome: { type: "kept-back", why: "not-an-emoji" },
	},
	{
		name: "a reaction nobody subscribed to is not sent",
		emoji: "👍",
		outcome: { type: "not-sent", why: "nobody-subscribed" },
	},
])("react: $name", async ({ emoji, outcome }) => {
	const stop = new AbortController();
	const participant = await joinChat(recording().connection, {
		room: { kind: "broadcast", name: "blue" },
		name: "ann",
		onLine() {},
		signal: stop.signal,
	});
	const reacted: unknown = await participant.react(emoji);
	stop.abort();
	expect(reacted).toEqual(outcome);
});
