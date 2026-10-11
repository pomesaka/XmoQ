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
	type PeerFetch,
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
	PageFetchServing,
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
const utf8Decoder = new TextDecoder();

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
/** The subscription `serveTrack` accepted, once it has. */
let served: Promise<PublishedTrack> | undefined;
/**
 * Every Object this page has, in ascending order: the ones it sent, and
 * with "serve-unsent" three that it never sent. It answers a FETCH from
 * these. The library keeps none, so this is the application's part.
 */
const sentObjects: { group: bigint; object: bigint; payload: Uint8Array }[] =
	[];

/** Takes an Object into `sentObjects`, at its place. */
function keep(group: bigint, payload: Uint8Array): void {
	sentObjects.push({ group, object: 0n, payload });
	sentObjects.sort((a, b) => (before(a, b) ? -1 : before(b, a) ? 1 : 0));
}
/** How each FETCH this page accepted ended, in the order they ended. */
const servedFetchEnds: string[] = [];

/** Whether `a` comes before `b`. */
const before = (
	a: { group: bigint; object: bigint },
	b: { group: bigint; object: bigint },
): boolean => a.group < b.group || (a.group === b.group && a.object < b.object);

/** 2^64 - 1: the Object ID of the last Location a Group has. */
const LAST_OBJECT = 2n ** 64n - 1n;

/**
 * Answers a Standalone FETCH from `sentObjects`, in ascending order.
 *
 * FETCH_OK has the End Location that was asked for, or the Location after
 * the last Object this page has where the range goes beyond it. The fetch
 * stream has the Objects of the range, and, where the range ends at or
 * before the last Object this page has and its tail holds no Object, one
 * End of Range indicator that says so: the page is where the Track comes
 * from, so what it does not have does not exist.
 */
async function answerFetch(request: PeerFetch): Promise<void> {
	const largest = sentObjects.at(-1);
	if (largest === undefined || before(largest, request.startLocation)) {
		// INVALID_RANGE: the range starts after everything there is.
		await request.reject(0x11n, "nothing of the Track is that far");
		return;
	}
	const asked = request.endLocation;
	// The last Location of what was asked for: Object 0 names the whole Group.
	const last =
		asked.object === 0n
			? { group: asked.group, object: LAST_OBJECT }
			: { group: asked.group, object: asked.object - 1n };
	const beyond = before({ group: largest.group, object: largest.object }, last);
	const writer = await request.accept({
		endLocation: beyond
			? { group: largest.group, object: largest.object + 1n }
			: asked,
	});
	void writer.ended.then((end) => {
		servedFetchEnds.push(describe(end));
		console.log(
			`fetch served: request_id=${request.requestId} ${describe(end)}`,
		);
	});
	let written: { group: bigint; object: bigint } | undefined;
	for (const each of sentObjects) {
		if (before(each, request.startLocation) || before(last, each)) continue;
		await writer.write({
			groupId: each.group,
			subgroupId: 0n,
			objectId: each.object,
			publisherPriority: 0,
			properties: [],
			payload: each.payload,
		});
		written = each;
	}
	if (!beyond && (written === undefined || before(written, last))) {
		await writer.endOfRange("NonExistent", last);
	}
	await writer.close();
}
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
	const payload = fromHex(payloadHex);
	await subgroup.write({
		objectId: 0n,
		payload,
		properties: properties.map(toKeyValuePair),
	});
	keep(group, payload);
	if (finish) await subgroup.close();
	console.log(
		`object sent: group=${group} object=0 payload_len=${payloadHex.length / 2} properties=${properties.length} finished=${finish}`,
	);
}

/**
 * Whether a call on a subgroup stream failed because the subscriber stopped
 * the stream or ended the subscription, and not for another reason. The
 * library says so by the class of the error, whichever form the browser gave
 * the stop (docs/experiments/2026-10-11-write-abort-in-chromium.md), and
 * resets a stream that was stopped itself (§11.4.3).
 */
function stoppedByThePeer(error: unknown): boolean {
	return (
		error instanceof SubgroupClosedError || error instanceof PublishEndedError
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
		// `code=undefined` with `metStop=true` is a stop whose code the
		// browser did not pass on.
		const how =
			error instanceof SubgroupClosedError
				? ` metStop=${error.metStop} code=${error.code}`
				: "";
		const outcome = `stopped:${how} ${errorText(error)}`;
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
		{
			range: fetchRangeOf(asked),
			parameters: {
				// The priority the page's SUBSCRIBE has.
				subscriberPriority: 128,
				...(asked.type === "Standalone" && asked.fillTimeout !== undefined
					? { fillTimeout: BigInt(asked.fillTimeout) }
					: {}),
			},
		},
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

	serveTrack: withPlainErrors(
		async (
			namespace: string,
			trackName: string,
			fetches: PageFetchServing,
			unsentPayloadHex: string,
		) => {
			if (fetches === "serve-unsent") {
				// Object 0 of three Groups far ahead, which no relay has seen.
				for (const group of [100n, 101n, 102n]) {
					keep(group, fromHex(unsentPayloadHex));
				}
			}
			const moq = connected();
			const wanted = utf8.encode(trackName);
			const sameName = (name: Uint8Array): boolean =>
				name.length === wanted.length &&
				name.every((byte, index) => byte === wanted[index]);
			let asked = false;
			served = new Promise((resolve, reject) => {
				// Before the namespace is announced: a relay asks as soon as it has
				// a subscriber.
				moq.serve({
					subscribe(request) {
						console.log(
							`asked: request_id=${request.requestId} track=${utf8Decoder.decode(request.trackName)}`,
						);
						if (asked || !sameName(request.trackName)) {
							// DOES_NOT_EXIST. A request that went meanwhile needs none.
							request.reject(0x10n, "not a track of this page").catch(() => {});
							return;
						}
						asked = true;
						// §10.2.11: a publisher that has published Objects of the
						// Track says how far it is.
						const largest = sentObjects.at(-1);
						request
							.accept(
								largest === undefined
									? {}
									: {
											parameters: {
												largestObject: {
													group: largest.group,
													object: largest.object,
												},
											},
										},
							)
							.then((track) => {
								published = track;
								console.log(`serving: alias=${track.trackAlias}`);
								resolve(track);
							}, reject);
					},
					fetch(request) {
						console.log(
							`fetch asked: request_id=${request.requestId} start={${request.startLocation.group}, ${request.startLocation.object}} end={${request.endLocation.group}, ${request.endLocation.object}}`,
						);
						if (!sameName(request.trackName) || fetches === "refuse") {
							// DOES_NOT_EXIST.
							request.reject(0x10n, "not a track of this page").catch(() => {});
							return;
						}
						if (fetches === "stall") {
							// Accepted, and then nothing: for a subscriber that gives
							// its FETCH up.
							void request.accept({ endLocation: request.endLocation }).then(
								(writer) =>
									writer.ended.then((end) => {
										servedFetchEnds.push(describe(end));
										console.log(
											`fetch served: request_id=${request.requestId} ${describe(end)}`,
										);
									}),
								() => {},
							);
							return;
						}
						return answerFetch(request);
					},
				});
			});
			// Observed here already: a failure is reported by `servingTrack`.
			served.catch(() => {});
			await moq.publishNamespace(namespaceFromPath(namespace));
			console.log(`namespace announced: ${namespace}`);
		},
	),

	servedFetchEnds: withPlainErrors(async () => [...servedFetchEnds]),

	servingTrack: withPlainErrors(async () => {
		if (served === undefined) throw new Error("serveTrack() was not called");
		return (await served).trackAlias.toString();
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
