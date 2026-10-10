// Own browser client of the interop run: a publisher and a subscriber built on
// @xmoq/moqt. `run.ts` drives this page through `window.xmoq`, which has the
// same shape as the moqtail-ts page in peers/moqtail/browser so the driver can
// swap one for the other.

import {
	connect,
	type KeyValuePair,
	type MoqtConnection,
	type PeerStreamEnd,
	type PublishedTrack,
	type Subscription,
} from "@xmoq/moqt";
import type { Diagnostics } from "@xmoq/moqt/diagnostics";
import type {
	PageProperty,
	PageReceivedObject,
	SubscriptionReport,
} from "./xmoq-page.ts";

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

/** The Objects read from a subscription so far, and how reading ended. */
interface Reading {
	objects: PageReceivedObject[];
	/** "closed" or the error `objects` failed with; undefined while Objects can still come. */
	end: string | undefined;
	/** Called once when an Object is added or `end` is set. */
	wake: (() => void)[];
}

let connection: MoqtConnection | undefined;
let published: PublishedTrack | undefined;
let subscribed: Subscription | undefined;
/** Absent while nothing is subscribed, and for a subscription that is not read. */
let reading: Reading | undefined;
/** Stream Count of the PUBLISH_DONE the subscription received. */
let publishDoneStreamCount: bigint | undefined;

/** PUBLISH_DONE Status Code TRACK_ENDED (§10.11). */
const TRACK_ENDED = 0x2n;

function connected(): MoqtConnection {
	if (connection === undefined) throw new Error("connect() has not completed");
	return connection;
}

/** Logs what the session does, and the bytes the fixtures are taken from. */
const diagnostics: Diagnostics = {
	onEvent(event) {
		// This page holds one subscription, so the event is about that one.
		if (event.type === "publish-done") {
			publishDoneStreamCount = event.streamCount;
		}
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

function toKeyValuePair(property: PageProperty): KeyValuePair {
	return "varint" in property
		? { type: BigInt(property.type), value: BigInt(property.varint) }
		: { type: BigInt(property.type), value: fromHex(property.bytesHex) };
}

function toPageProperty({ type, value }: KeyValuePair): PageProperty {
	return typeof value === "bigint"
		? { type: type.toString(), varint: value.toString() }
		: { type: type.toString(), bytesHex: toHex(value, "") };
}

function errorText(error: unknown): string {
	return error instanceof Error
		? `${error.name}: ${error.message}`
		: String(error);
}

/**
 * Reads every Object of the subscription as it arrives, so that the
 * connection is never held back by this page. Objects without a payload are
 * left out.
 */
function readAll(subscription: Subscription): Reading {
	const started: Reading = { objects: [], end: undefined, wake: [] };
	const notify = (): void => {
		const waiting = started.wake;
		started.wake = [];
		for (const resolve of waiting) resolve();
	};
	const read = async (): Promise<string> => {
		const reader = subscription.objects.getReader();
		for (;;) {
			const result = await reader.read();
			if (result.done) return "closed";
			const { groupId, object } = result.value;
			if (object.payload.length === 0) continue;
			started.objects.push({
				group: groupId.toString(),
				object: object.objectId.toString(),
				payloadHex: toHex(object.payload, ""),
				properties: object.properties.map(toPageProperty),
			});
			notify();
		}
	};
	void read()
		.catch(errorText)
		.then((end) => {
			started.end = end;
			notify();
		});
	return started;
}

/**
 * The first Object that `pick` accepts. When reading ends before one
 * arrives, the error says how the subscription ended.
 */
async function received(
	what: string,
	pick: (object: PageReceivedObject) => boolean,
): Promise<PageReceivedObject> {
	if (reading === undefined || subscribed === undefined) {
		throw new Error("subscribe() has not completed");
	}
	const from = reading;
	for (;;) {
		const found = from.objects.find(pick);
		if (found !== undefined) return found;
		if (from.end !== undefined) {
			throw new Error(
				`the subscription ended before ${what}: ${describe(await subscribed.ended)}; objects: ${from.end}`,
			);
		}
		await new Promise<void>((resolve) => {
			from.wake.push(resolve);
		});
	}
}

async function subscribeTo(
	namespace: string,
	trackName: string,
): Promise<Subscription> {
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
	subscribed = subscription;
	console.log(
		`subscribed: request_id=${subscription.requestId} alias=${subscription.trackAlias}`,
	);
	return subscription;
}

/** Sends one Object as Object 0 of the Group, on a subgroup stream of its own. */
async function sendIn(
	group: bigint,
	payloadHex: string,
	finish: boolean,
	properties: PageProperty[] = [],
): Promise<void> {
	const subgroup = await publishedTrack().openSubgroup({
		groupId: group,
		subgroupId: { mode: "Explicit", value: 0n },
		publisherPriority: 0,
		// §11.4.2: the bit is of the stream, and says that every Object on it
		// has the Properties Length field.
		properties: properties.length > 0,
		// A finished stream holds the one Object of its Group, which is the
		// largest. A stream left open says nothing about what follows.
		endOfGroup: finish,
	});
	await subgroup.write({
		objectId: 0n,
		payload: fromHex(payloadHex),
		properties: properties.map(toKeyValuePair),
	});
	if (finish) await subgroup.close();
	console.log(
		`object sent: group=${group} object=0 payload_len=${payloadHex.length / 2} properties=${properties.length} finished=${finish}`,
	);
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
			throw new Error(errorText(error));
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

	sendObject: withPlainErrors((payloadHex: string) =>
		sendIn(0n, payloadHex, true),
	),

	subscribe: withPlainErrors(async (namespace: string, trackName: string) => {
		const subscription = await subscribeTo(namespace, trackName);
		reading = readAll(subscription);
		return subscription.requestId.toString();
	}),

	firstObject: withPlainErrors(() =>
		received("any Object with a payload", () => true),
	),

	disconnect: withPlainErrors(async () => {
		connection?.close();
		connection = undefined;
	}),
};

function publishedTrack(): PublishedTrack {
	if (published === undefined)
		throw new Error("publishTrack() has not completed");
	return published;
}

function currentSubscription(): Subscription {
	if (subscribed === undefined)
		throw new Error("subscribe() has not completed");
	return subscribed;
}

/** What `done()` resolved with, once it has. */
let peerStreamEnd: PeerStreamEnd | undefined;

async function endWithPublishDone(): Promise<void> {
	peerStreamEnd = await publishedTrack().done(TRACK_ENDED, "");
	// The line of the Rust client, with the library's value.
	console.log(`PUBLISH_DONE_PEER_ENDED how=${describe(peerStreamEnd)}`);
}

window.xmoqOwn = {
	announceNamespace: withPlainErrors(async (namespace: string) => {
		await connected().publishNamespace(namespaceFromPath(namespace));
		console.log(`namespace announced: ${namespace}`);
	}),

	endTrack: withPlainErrors(async () => {
		await endWithPublishDone();
		console.log(`track ended: ${describe(await publishedTrack().ended)}`);
	}),

	sendObjectIn: withPlainErrors(
		(
			group: string,
			payloadHex: string,
			finish: boolean,
			properties: PageProperty[],
		) => sendIn(BigInt(group), payloadHex, finish, properties),
	),

	sendLastObjectAndClose: withPlainErrors(
		async (group: string, payloadHex: string, publishDone: boolean) => {
			await sendIn(BigInt(group), payloadHex, true);
			if (publishDone) await endWithPublishDone();
			connected().close();
			console.log(`session closed after the last Object: done=${publishDone}`);
		},
	),

	publishEnd: withPlainErrors(async () =>
		describe(await publishedTrack().ended),
	),

	peerStreamEnd: withPlainErrors(async () => describe(peerStreamEnd ?? null)),

	subscribeWithoutReading: withPlainErrors(
		async (namespace: string, trackName: string) =>
			(await subscribeTo(namespace, trackName)).requestId.toString(),
	),

	objectOfGroup: withPlainErrors(
		async (group: string) =>
			(
				await received(
					`the Object of Group ${group}`,
					(object) => object.group === group,
				)
			).payloadHex,
	),

	payloadsOfGroup: withPlainErrors(async (group: string) => {
		if (reading === undefined) throw new Error("subscribe() has not completed");
		return describe(
			reading.objects
				.filter((object) => object.group === group)
				.map((object) => object.payloadHex),
		);
	}),

	propertiesOfGroup: withPlainErrors(async (group: string) =>
		describe(
			(
				await received(
					`the Object of Group ${group}`,
					(object) => object.group === group,
				)
			).properties ?? [],
		),
	),

	state: withPlainErrors(async () => {
		const open = Symbol("open");
		const closed = await Promise.race([
			connected().closed,
			new Promise<typeof open>((resolve) => setTimeout(resolve, 0, open)),
		]);
		return describe({
			session: closed === open ? null : closed,
			objectsRead: reading?.objects.length ?? null,
			objectsEnd: reading?.end ?? null,
		});
	}),

	cancelSubscription: withPlainErrors(async () => {
		await currentSubscription().cancel();
		console.log("subscription cancelled");
	}),

	subscriptionReport: withPlainErrors(async () => {
		const end = await currentSubscription().ended;
		const from = reading;
		// Reading goes on after the end, until what had arrived is taken.
		while (from !== undefined && from.end === undefined) {
			await new Promise<void>((resolve) => {
				from.wake.push(resolve);
			});
		}
		const report: SubscriptionReport = {
			end: {
				type: end.type,
				...(end.type === "publish-done"
					? {
							statusCode: end.statusCode.toString(),
							errorReason: end.errorReason,
						}
					: {}),
			},
			groups: from?.objects.map((object) => object.group) ?? [],
			streamCount: publishDoneStreamCount?.toString() ?? null,
			objectsEnd: from?.end ?? "not read",
		};
		return JSON.stringify(report);
	}),
};

console.log("page ready");
