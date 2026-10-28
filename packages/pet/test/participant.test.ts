import { expect, test } from "bun:test";
import type {
	MessageParameters,
	MoqtConnection,
	ReceivedObject,
	SubscribeRequest,
	Subscription,
} from "@xmoq/moqt";
import type { Action } from "../src/core/payloads.ts";
import { joinGarden } from "../src/moqt/participant.ts";

type GardenConnection = Pick<
	MoqtConnection,
	"serve" | "publishNamespace" | "subscribe" | "fetch" | "close" | "closed"
>;

const decoder = new TextDecoder();
const nameOf = (request: SubscribeRequest): string =>
	[...request.trackNamespace, request.trackName]
		.map((field) => decoder.decode(field))
		.join("/");

/**
 * A connection that records what `joinGarden` asks of it: the
 * PUBLISH_NAMESPACE is answered, and the SUBSCRIBEs for `garden` and
 * `state` are refused, so that no FETCH follows. The connection's own
 * token is the library's to add.
 */
function recording(): {
	connection: GardenConnection;
	publishNamespace: {
		namespace: string;
		parameters: MessageParameters | undefined;
	}[];
	subscribed: Promise<SubscribeRequest[]>;
} {
	const publishNamespace: {
		namespace: string;
		parameters: MessageParameters | undefined;
	}[] = [];
	const requests: SubscribeRequest[] = [];
	const { promise: subscribed, resolve } =
		Promise.withResolvers<SubscribeRequest[]>();
	return {
		publishNamespace,
		subscribed,
		connection: {
			serve() {},
			async publishNamespace(namespace, parameters) {
				publishNamespace.push({
					namespace: namespace.map((field) => decoder.decode(field)).join("/"),
					parameters,
				});
			},
			async subscribe(request) {
				requests.push(request);
				if (requests.length === 2) resolve(requests);
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
	await joinGarden(connection, {
		room: "garden",
		name: "alice",
		publishNamespaceToken,
		onGarden() {},
		onState() {},
		signal: stop.signal,
	});
	const subscribes = await subscribed;
	stop.abort();
	expect({
		publishNamespace: publishNamespace.map(({ namespace, parameters }) => [
			namespace,
			parameters?.authorizationToken?.map(hex),
		]),
		subscribe: subscribes
			.map((request) => [
				nameOf(request),
				request.parameters?.authorizationToken,
			])
			.sort(),
	}).toEqual({
		publishNamespace: [["xmoq/pet/garden/alice", publishNamespaceTokens]],
		// The garden under the room, the pet's body under the pet.
		subscribe: [
			["xmoq/pet/garden/garden", undefined],
			["xmoq/pet/garden/pet/state", undefined],
		],
	});
});

test("a page named as the pet is refused before anything is sent", async () => {
	const { connection, publishNamespace } = recording();
	await expect(
		joinGarden(connection, {
			room: "garden",
			name: "pet",
			onGarden() {},
			onState() {},
		}),
	).rejects.toThrow(RangeError);
	expect(publishNamespace).toEqual([]);
});

/** A subscription that has the Objects given, in their Groups, and ends after them. */
function subscriptionOf(payloads: string[]): Subscription {
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
		parameters: {},
		trackProperties: [],
		objects,
		ended: Promise.resolve({ type: "cancelled" }),
		goAway: new Promise(() => {}),
		cancel: async () => {},
		update: () => Promise.reject(new Error("no update in this test")),
	};
}

const stateAt = (time: number, doing: string): string =>
	`{"from":[1,1],"to":[1,1],"began":1,"arrives":1,"facing":"left","doing":"${doing}","mood":2,"bubble":null,"by":null,"at":${time}}`;

const gardenAt = (time: number, things: number): string =>
	`{"things":[${Array.from({ length: things }, (_, id) => `{"id":${id},"kind":"ball","at":[1,1],"by":"alice"}`).join(",")}],"events":[],"at":${time}}`;

test.each<{
	name: string;
	track: "garden" | "state";
	payloads: string[];
	given: string[];
	malformed: string[];
}>([
	{
		name: "each state is given, in the order it came",
		track: "state",
		payloads: [stateAt(5, "sit"), stateAt(6, "walk")],
		given: ["state 5 sit", "state 6 walk"],
		malformed: [],
	},
	{
		name: "a state older than the one given is not given",
		track: "state",
		payloads: [stateAt(6, "walk"), stateAt(5, "sit")],
		given: ["state 6 walk"],
		malformed: [],
	},
	{
		name: "an Object that is no state is told, and the next one given",
		track: "state",
		payloads: ['{"doing":"sit"}', stateAt(5, "sit")],
		given: ["state 5 sit"],
		malformed: ["state 0 from"],
	},
	{
		name: "each garden is given, in the order it came",
		track: "garden",
		payloads: [gardenAt(5, 1), gardenAt(6, 2)],
		given: ["garden 5 1", "garden 6 2"],
		malformed: [],
	},
	{
		name: "a garden older than the one given is not given",
		track: "garden",
		payloads: [gardenAt(6, 2), gardenAt(5, 1)],
		given: ["garden 6 2"],
		malformed: [],
	},
	{
		name: "an Object that is no garden is told, and the next one given",
		track: "garden",
		payloads: [gardenAt(5, 9), gardenAt(6, 1)],
		given: ["garden 6 1"],
		malformed: ["garden 0 things"],
	},
])("$name", async ({ track, payloads, given, malformed }) => {
	const got: string[] = [];
	const told: string[] = [];
	const stop = new AbortController();
	let answered = false;
	await joinGarden(
		{
			serve() {},
			async publishNamespace() {},
			async subscribe(request) {
				// Once, and for `track` alone: the others are never
				// answered, so that the test ends.
				if (answered || decoder.decode(request.trackName) !== track) {
					return new Promise(() => {});
				}
				answered = true;
				return subscriptionOf(payloads);
			},
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
			closed: new Promise(() => {}),
		},
		{
			room: "garden",
			name: "alice",
			onGarden: (garden) =>
				got.push(`garden ${garden.at} ${garden.things.length}`),
			onState: (state) => got.push(`state ${state.at} ${state.doing}`),
			onMalformed: (of, number, why) => told.push(`${of} ${number} ${why}`),
			signal: stop.signal,
		},
	);
	for (let turn = 0; turn < 50; turn++) await Promise.resolve();
	stop.abort();
	expect({ got, told }).toEqual({ got: given, told: malformed });
});

test.each<{ name: string; action: Action; outcome: unknown }>([
	{
		name: "a throw beyond the floor is kept back",
		action: { kind: "throw", to: [1001, 0] },
		outcome: { type: "kept-back", why: "to" },
	},
	{
		name: "a feed beyond the floor is kept back",
		action: { kind: "feed", to: [0, 1001] },
		outcome: { type: "kept-back", why: "to" },
	},
	{
		name: "a talk over 120 bytes is kept back",
		action: { kind: "talk", text: "a".repeat(121) },
		outcome: { type: "kept-back", why: "text" },
	},
	{
		name: "an eat of a thing that is no id is kept back",
		action: { kind: "eat", thing: -1 },
		outcome: { type: "kept-back", why: "thing" },
	},
	{
		name: "an action nobody subscribed to is not sent",
		action: { kind: "pet" },
		outcome: { type: "not-sent", why: "nobody-subscribed" },
	},
])("act: $name", async ({ action, outcome }) => {
	const stop = new AbortController();
	const actor = await joinGarden(recording().connection, {
		room: "garden",
		name: "alice",
		onGarden() {},
		onState() {},
		signal: stop.signal,
	});
	const acted: unknown = await actor.act(action);
	stop.abort();
	expect(acted).toEqual(outcome);
});
