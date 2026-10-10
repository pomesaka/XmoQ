// Own browser client of the interop run: a publisher and a subscriber built on
// @xmoq/moqt. `run.ts` drives this page through `window.xmoq`, which has the
// same shape as the moqtail-ts page in peers/moqtail/browser so the driver can
// swap one for the other.

import {
	connect,
	decodeFetchBody,
	type FetchRange,
	type KeyValuePair,
	type MoqtConnection,
	type PeerStreamEnd,
	PublishEndedError,
	type PublishedTrack,
	RequestFailedError,
	SubgroupClosedError,
	type Subscription,
} from "@xmoq/moqt";
import type { Diagnostics } from "@xmoq/moqt/diagnostics";
import type {
	PageFetch,
	PageFetchReport,
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
	publisherPriority: number | null = 0,
): Promise<void> {
	const subgroup = await publishedTrack().openSubgroup({
		groupId: group,
		subgroupId: { mode: "Explicit", value: 0n },
		...(publisherPriority === null ? {} : { publisherPriority }),
		// §11.4.2: the bit is of the stream, and says that every Object on it
		// has the Properties Length field.
		properties: properties.length > 0,
		// A finished stream holds the one Object of its Group, which is the
		// largest. A stream left open says nothing about what follows.
		endOfGroup: finish,
	});
	try {
		await subgroup.write({
			objectId: 0n,
			payload: fromHex(payloadHex),
			properties: properties.map(toKeyValuePair),
		});
	} catch (error) {
		// §11.4.3: "The publisher SHOULD respond with a reset" to a stream that
		// was stopped. WHY it is needed here as well: when the write fails with
		// Chromium's NetworkError (`stoppedByThePeer`), the library has not
		// learned that the stream is over, and the session would refuse the
		// PUBLISH_DONE of the track while it counts the stream as open. 4 runs
		// of 300 ended so before this reset.
		await subgroup.reset();
		throw error;
	}
	if (finish) await subgroup.close();
	console.log(
		`object sent: group=${group} object=0 payload_len=${payloadHex.length / 2} properties=${properties.length} finished=${finish}`,
	);
}

/**
 * Whether a write to a subgroup stream failed because the subscriber stopped
 * the stream or ended the subscription, and not for another reason.
 *
 * WHY these forms: a STOP_SENDING that the library has taken in rejects the
 * write with `SubgroupClosedError`, or with `PublishEndedError` when the
 * PUBLISH was cancelled with it. One the library has not taken in yet
 * rejects it with the browser's error, which `SubgroupWriter.write` passes
 * on, and Chromium has two: a `WebTransportError` with `source` "stream"
 * ("Received STOP_SENDING.", seen in Chromium 156 on 2026-10-11 with the
 * library's watch for a stop taken out), and a `DOMException` named
 * NetworkError, "The stream was aborted by the remote server", which is what
 * the failed runs had. The second is what `OutgoingStream::HandlePipeClosed`
 * makes when the data pipe of the stream closes before the STOP_SENDING is
 * reported (`third_party/blink/renderer/modules/webtransport/outgoing_stream.cc`,
 * tip of Chromium's repository, read on 2026-10-11). It has no code.
 */
function stoppedByThePeer(error: unknown): boolean {
	return (
		error instanceof SubgroupClosedError ||
		error instanceof PublishEndedError ||
		(error instanceof WebTransportError && error.source === "stream") ||
		(error instanceof DOMException && error.name === "NetworkError")
	);
}

/**
 * `sendIn` with `finish`, for a stream the subscriber is expected to stop:
 * resolves with "sent" or with "stopped" and the error of the write. The own
 * Rust client takes a stopped stream the same way (`STOP_SENDING_RECEIVED`).
 */
async function sendInWhatMayBeStopped(
	group: bigint,
	payloadHex: string,
	publisherPriority: number | null,
): Promise<string> {
	try {
		await sendIn(group, payloadHex, true, [], publisherPriority);
		return "sent";
	} catch (error) {
		if (!stoppedByThePeer(error)) throw error;
		const outcome = `stopped: ${errorText(error)}`;
		console.log(`object of group=${group} ${outcome}`);
		return outcome;
	}
}

function fetchRangeOf(asked: PageFetch): FetchRange {
	if (asked.type === "Standalone") {
		return {
			fetchType: "Standalone",
			trackNamespace: namespaceFromPath(asked.namespace),
			trackName: utf8.encode(asked.trackName),
			startLocation: {
				group: BigInt(asked.start[0]),
				object: BigInt(asked.start[1]),
			},
			endLocation: {
				group: BigInt(asked.end[0]),
				object: BigInt(asked.end[1]),
			},
		};
	}
	return {
		fetchType: asked.type,
		joiningRequestId: currentSubscription().requestId,
		joiningStart: BigInt(asked.joiningStart),
	};
}

/** Sends a FETCH, decodes its fetch stream, and reports all of it once the FETCH is over. */
async function fetchRange(
	asked: PageFetch,
	timeoutMs: number,
): Promise<PageFetchReport> {
	const session = connected();
	const fetch = await session.fetch(
		// The priority the page's SUBSCRIBE has.
		{ range: fetchRangeOf(asked), parameters: { subscriberPriority: 128 } },
		{ signal: AbortSignal.timeout(timeoutMs) },
	);
	console.log(`fetch sent: request_id=${fetch.requestId} type=${asked.type}`);
	const response: Promise<PageFetchReport["response"]> = fetch.response.then(
		(ok) => ({
			endOfTrack: ok.endOfTrack,
			endLocation: [
				ok.endLocation.group.toString(),
				ok.endLocation.object.toString(),
			],
		}),
		(error: unknown) => ({
			error: errorText(error),
			...(error instanceof RequestFailedError
				? { errorCode: error.errorCode.toString() }
				: {}),
		}),
	);
	const items: PageFetchReport["items"] = [];
	let bodyEnd = "closed";
	try {
		const reader = decodeFetchBody(session, fetch).getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) break;
			const item = next.value;
			items.push(
				item.type === "object"
					? {
							type: "object",
							group: item.object.groupId.toString(),
							object: item.object.objectId.toString(),
							payloadHex: toHex(item.object.payload, ""),
						}
					: {
							type: "end-of-range",
							kind: item.kind,
							group: item.location.group.toString(),
							object: item.location.object.toString(),
						},
			);
		}
	} catch (error) {
		bodyEnd = errorText(error);
	}
	const end = await fetch.ended;
	const report: PageFetchReport = {
		response: await response,
		items,
		bodyEnd,
		end: {
			type: end.type,
			...("code" in end && end.code !== undefined ? { code: end.code } : {}),
			...(end.type === "rejected"
				? { error: { errorCode: end.error.errorCode.toString() } }
				: {}),
		},
	};
	console.log(`fetch over: ${JSON.stringify(report)}`);
	return report;
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
			publisherPriority: number | null,
		) =>
			sendIn(BigInt(group), payloadHex, finish, properties, publisherPriority),
	),

	sendObjectInWhatMayBeStopped: withPlainErrors(
		(group: string, payloadHex: string, publisherPriority: number | null) =>
			sendInWhatMayBeStopped(BigInt(group), payloadHex, publisherPriority),
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

	fetchRange: withPlainErrors(async (asked: PageFetch, timeoutMs: number) =>
		JSON.stringify(await fetchRange(asked, timeoutMs)),
	),

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
