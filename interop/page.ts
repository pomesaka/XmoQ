// Own browser client of the interop run: a publisher and a subscriber built on
// @xmoq/moqt. `run.ts` drives this page through `window.xmoq`, which has the
// same shape as the moqtail-ts page in peers/moqtail/browser so the driver can
// swap one for the other.

import { type ChatParticipant, joinChat } from "@xmoq/chat";
import {
	type AudioPlayback,
	type AudioTrackEncoder,
	createAudioTrackEncoder,
	createPlayout,
	createVideoTrackEncoder,
	keepPlaying,
	keepPublishing,
	playAudioTrack,
	playVideoTrack,
	type VideoPlayback,
	type VideoTrackEncoder,
} from "@xmoq/media";
import {
	connect,
	decodeFetchBody,
	type FetchRange,
	type KeyValuePair,
	type MoqtConnection,
	type NamespaceSubscription,
	type PeerFetch,
	type PeerStreamEnd,
	PublishEndedError,
	type PublishedTrack,
	RequestErrorCode,
	RequestFailedError,
	SubgroupClosedError,
	type SubgroupWriter,
	type Subscription,
} from "@xmoq/moqt";
import type { Diagnostics } from "@xmoq/moqt/diagnostics";
import {
	type Catalog,
	type CatalogPublisher,
	createCatalogPublisher,
	followCatalog,
	namespaceTuple,
} from "@xmoq/msf";
import { followTranscript } from "@xmoq/transcript";
import type {
	PageAudioReport,
	PageBroadcastReport,
	PageCatalogReport,
	PageChatCatchUp,
	PageChatReport,
	PageFetch,
	PageFetchReport,
	PageFetchServing,
	PageKeptReport,
	PageProperty,
	PageReceivedObject,
	PageTranscriptPiece,
	PageVideoPublishReport,
	PageVideoReport,
	PageVideoSetup,
	PageVoiceReport,
	SubscriptionReport,
} from "./xmoq-page.ts";
import { BROADCAST, KEPT_DELAY_MS, VOICE } from "./xmoq-page.ts";

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
/** The SUBSCRIBE_NAMESPACE of `watchNamespaces`, and the lines of what came of it. */
let watchedNamespaces: NamespaceSubscription | undefined;
const namespaceLines: string[] = [];
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
/** The streams `writeToSubgroup` left open, by "group/subgroup". */
const openSubgroups = new Map<string, SubgroupWriter>();
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
	if (error instanceof RequestFailedError) {
		// What the library hands the application of the Retry Interval, for
		// the drivers that look at it.
		const wait = error.retryWaitMs;
		return `${error.name}: ${error.message} (retry: ${wait === undefined ? "no" : `after ${wait} ms`})`;
	}
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
			const { groupId, subgroupId, publisherPriority, object } = result.value;
			if (object.payload.length === 0) continue;
			started.objects.push({
				group: groupId.toString(),
				object: object.objectId.toString(),
				payloadHex: toHex(object.payload, ""),
				properties: object.properties.map(toPageProperty),
				subgroup: subgroupId.toString(),
				publisherPriority: publisherPriority ?? null,
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
	/** RENDEZVOUS_TIMEOUT in milliseconds (§10.2.6); left out when absent. */
	rendezvousTimeoutMs?: number,
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
			...(rendezvousTimeoutMs === undefined
				? {}
				: { rendezvousTimeout: BigInt(rendezvousTimeoutMs) }),
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
							subgroup: item.object.subgroupId?.toString() ?? null,
							publisherPriority: item.object.publisherPriority,
							properties: item.object.properties.map(toPageProperty),
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
		async (
			relayUrl: string,
			certSha256Hex: string | null,
			maxObjectPayloadLength?: number,
			answerTimeoutMs?: number,
			setupTimeoutMs?: number,
			openStreamTimeoutMs?: number,
			idleSubgroupStreamTimeoutMs?: number,
		) => {
			connection = await connect(relayUrl, {
				implementation: "xmoq-moqt/0.0.0",
				...(answerTimeoutMs === undefined ? {} : { answerTimeoutMs }),
				...(setupTimeoutMs === undefined ? {} : { setupTimeoutMs }),
				...(openStreamTimeoutMs === undefined ? {} : { openStreamTimeoutMs }),
				...(idleSubgroupStreamTimeoutMs === undefined
					? {}
					: { idleSubgroupStreamTimeoutMs }),
				...(maxObjectPayloadLength === undefined
					? {}
					: { limits: { maxObjectPayloadLength } }),
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

	subscribe: withPlainErrors(
		async (
			namespace: string,
			trackName: string,
			rendezvousTimeoutMs?: number,
		) => {
			const subscription = await subscribeTo(
				namespace,
				trackName,
				rendezvousTimeoutMs,
			);
			reading = readAll(subscription);
			return subscription.requestId.toString();
		},
	),

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

/** What `followCatalog` has given this page so far. */
interface CatalogReading {
	states: PageCatalogReport["states"];
	/** Absent while the Joining FETCH is under way. */
	joiningFetch: string | undefined;
	/** How the subscription ended; absent while it is open. */
	ended: string | undefined;
}
let catalogReading: CatalogReading | undefined;
let catalogPublisher: CatalogPublisher | undefined;

/** What `playVideoTrack` has given this page so far. */
interface VideoPlaying {
	playback: VideoPlayback;
	frames: PageVideoReport["frames"];
	/** How the playing ended; absent while it goes on. */
	ended: string | undefined;
	firstFrameAfterMs(): number | null;
}
let videoPlaying: VideoPlaying | undefined;

/** What `publishAudio` started. */
let audioPublishing:
	| {
			encoder: AudioTrackEncoder;
			made: number;
			timer: ReturnType<typeof setInterval>;
	  }
	| undefined;
/** What `playAudio` has so far. */
let audioPlaying:
	| { playback: AudioPlayback; pieces: PageAudioReport["pieces"] }
	| undefined;

/** What `publishBroadcast` started. */
let broadcastPublishing:
	| {
			video: VideoTrackEncoder;
			audio: AudioTrackEncoder;
			catalog: CatalogPublisher;
			/** The timers that paint and make sound. */
			timers: ReturnType<typeof setInterval>[];
			painted: number;
			made: number;
	  }
	| undefined;

/** What `followTranscript` was handed. */
const transcriptPieces: PageTranscriptPiece[] = [];

/** What `publishVoice` started. */
let voicePublishing:
	| {
			audio: AudioTrackEncoder;
			/** Samples of one round: the voice, then the pause. */
			round: number;
			/** Pieces of 20 ms made, counted by the timer. */
			counted: { made: number };
	  }
	| undefined;

/**
 * The samples of `voice` (base64 of 16-bit little-endian samples at 16,000
 * Hz) at 48,000 Hz: two more between each two, on the line between them.
 */
function voiceAt48kHz(voice: string): Float32Array {
	const bytes = Uint8Array.from(atob(voice), (character) =>
		character.charCodeAt(0),
	);
	const view = new DataView(bytes.buffer);
	const count = Math.floor(bytes.length / 2);
	const at = (index: number): number =>
		view.getInt16(Math.min(index, count - 1) * 2, true) / 32768;
	const samples = new Float32Array(count * 3);
	for (let index = 0; index < count; index++) {
		const from = at(index);
		const to = at(index + 1);
		for (let step = 0; step < 3; step++) {
			samples[index * 3 + step] = from + ((to - from) * step) / 3;
		}
	}
	return samples;
}

/** What `publishVideo` started. */
interface VideoPublishing {
	encoder: VideoTrackEncoder;
	keyChunksNotSent: PageVideoPublishReport["keyChunksNotSent"];
	painted: number;
	timer: ReturnType<typeof setInterval>;
}
let videoPublishing: VideoPublishing | undefined;

/** After this frame `publishVideo` hands the encoder one more with an earlier time: a frame a viewer of the interop runs is there for. */
const STALE_FRAME_AFTER = 45;

/** The bars a frame number is painted as, and read from. */
const NUMBER_BITS = 8;

/** Paints `number` modulo 256 as 8 bars, the highest bit first: white for a 1, black for a 0. */
function paintNumber(
	context: OffscreenCanvasRenderingContext2D,
	width: number,
	height: number,
	number: number,
): void {
	const bar = width / NUMBER_BITS;
	for (let bit = 0; bit < NUMBER_BITS; bit++) {
		const one = ((number >> (NUMBER_BITS - 1 - bit)) & 1) === 1;
		context.fillStyle = one ? "#ffffff" : "#000000";
		context.fillRect(bit * bar, 0, bar, height);
	}
}

/** Pictures of random pixels, made once: every frame gets another one than the frame before it. */
function noiseTiles(width: number, height: number): OffscreenCanvas[] {
	return Array.from({ length: 4 }, () => {
		const tile = new OffscreenCanvas(width, height);
		const context = tile.getContext("2d");
		if (context === null) throw new Error("no 2d context");
		const image = context.createImageData(width, height);
		// `getRandomValues` fills 65,536 bytes at most in one call.
		for (let at = 0; at < image.data.length; at += 65_536) {
			crypto.getRandomValues(image.data.subarray(at, at + 65_536));
		}
		// Opaque: a transparent pixel would show the bars beneath.
		for (let at = 3; at < image.data.length; at += 4) image.data[at] = 255;
		context.putImageData(image, 0, 0);
		return tile;
	});
}

/**
 * The number the bars of a frame say, or null where one is neither dark
 * nor light. WHY black and white, read in the middle of a bar: the encoder
 * changes every colour a little and blurs the edges; the two that are
 * furthest apart, away from an edge, leave 64 of 255 on either side.
 */
function readNumber(
	context: OffscreenCanvasRenderingContext2D,
	frame: VideoFrame,
): number | null {
	const { displayWidth: width, displayHeight: height } = frame;
	context.canvas.width = width;
	context.canvas.height = height;
	context.drawImage(frame, 0, 0);
	const bar = width / NUMBER_BITS;
	let number = 0;
	for (let bit = 0; bit < NUMBER_BITS; bit++) {
		const [red] = context.getImageData(
			Math.floor(bit * bar + bar / 2),
			// The upper half: the lower one can be noise (`publishVideo`).
			Math.floor(height / 4),
			1,
			1,
		).data;
		if (red === undefined || (red >= 64 && red <= 192)) return null;
		number = number * 2 + (red > 192 ? 1 : 0);
	}
	return number;
}

/** What `keepVideoPublished` and `keepVideoPlayed` saw. */
const kept: PageKeptReport = { publishing: [], playing: [], shown: [] };

/** A connection of a page that makes its own, one for every session. */
const connectAnew = (
	relayUrl: string,
	certSha256Hex: string,
): Promise<MoqtConnection> =>
	connect(relayUrl, {
		implementation: "xmoq-moqt/0.0.0",
		serverCertificateHashes: [
			{ algorithm: "sha-256", value: fromHex(certSha256Hex) },
		],
	});

/** What the handler of `publishDynamicTrack` was called with. */
const newGroupsAsked: string[] = [];

/** The participant of `joinChat`, and what it had. */
const chat: {
	participant: ChatParticipant | undefined;
	/** Stops the participant: its `signal`. */
	stop: AbortController;
	lines: PageChatReport["lines"];
	malformed: string[];
	gaps: string[];
} = {
	participant: undefined,
	stop: new AbortController(),
	lines: [],
	malformed: [],
	gaps: [],
};

window.xmoqOwn = {
	connectWithToken: withPlainErrors(
		async (relayUrl: string, certSha256Hex: string, tokenHex: string) => {
			connection = await connect(relayUrl, {
				implementation: "xmoq-moqt/0.0.0",
				authorizationToken: fromHex(tokenHex),
				serverCertificateHashes: [
					{ algorithm: "sha-256", value: fromHex(certSha256Hex) },
				],
				diagnostics,
			});
			console.log(
				`connected: peer_setup_options=${describe(connection.peerSetupOptions)}`,
			);
		},
	),

	publishDynamicTrack: withPlainErrors(
		async (namespace: string, trackName: string) => {
			const moq = connected();
			const trackNamespace = namespaceFromPath(namespace);
			await moq.publishNamespace(trackNamespace);
			published = await moq.publish({
				trackNamespace,
				trackName: utf8.encode(trackName),
				trackAlias: PUBLISH_TRACK_ALIAS,
				parameters: { forward: true },
				// DYNAMIC_GROUPS (transport §12.6).
				trackProperties: [{ type: 0x30n, value: 1n }],
			});
			published.onNewGroupRequest((group) => {
				newGroupsAsked.push(group.toString());
				console.log(`new group requested: ${group}`);
			});
		},
	),

	newGroupRequests: () => Promise.resolve([...newGroupsAsked]),

	trackStatus: withPlainErrors(async (namespace: string, trackName: string) => {
		try {
			const { largestObject, trackProperties } = await connected().trackStatus({
				trackNamespace: namespaceFromPath(namespace),
				trackName: utf8.encode(trackName),
			});
			const largest =
				largestObject === undefined
					? "none"
					: `${largestObject.group},${largestObject.object}`;
			return `TRACK_STATUS_OK largest=${largest} track_properties=${trackProperties.length}`;
		} catch (error) {
			if (!(error instanceof RequestFailedError)) throw error;
			return `TRACK_STATUS_REFUSED error_code=0x${error.errorCode.toString(16)}`;
		}
	}),

	watchNamespaces: withPlainErrors(async (prefix: string) => {
		const subscription = await connected().subscribeNamespace({
			trackNamespacePrefix: namespaceFromPath(prefix),
		});
		watchedNamespaces = subscription;
		namespaceLines.push("NAMESPACES_SUBSCRIBED");
		void (async () => {
			for await (const { trackNamespace, done } of subscription.changes) {
				const path = trackNamespace
					.map((field) => utf8Decoder.decode(field))
					.join("/");
				namespaceLines.push(
					`${done ? "NAMESPACE_DONE" : "NAMESPACE"} namespace=${path}`,
				);
			}
			namespaceLines.push(
				`NAMESPACES_ENDED ${(await subscription.ended).type}`,
			);
		})();
	}),

	namespaceLines: () => Promise.resolve([...namespaceLines]),

	stopWatchingNamespaces: withPlainErrors(async () => {
		await watchedNamespaces?.cancel();
	}),

	keepVideoPublished: withPlainErrors(
		async (
			relayUrl: string,
			certSha256Hex: string,
			namespace: string,
			setup: PageVideoSetup,
		) => {
			const trackNamespace = namespaceTuple(namespace);
			const encoder = await createVideoTrackEncoder({
				trackNamespace,
				trackName: utf8.encode(setup.trackName),
				firstGroup: BigInt(Date.now()),
				codec: setup.codec,
				width: setup.width,
				height: setup.height,
				framerate: setup.framerate,
				bitrate: setup.bitrate,
				keyFrameIntervalMs: setup.keyFrameIntervalMs,
				mapping: setup.mapping,
			});
			const canvas = new OffscreenCanvas(setup.width, setup.height);
			const context = canvas.getContext("2d");
			if (context === null) throw new Error("no 2d context");
			let painted = 0;
			// The encoder goes on while no session is there.
			setInterval(() => {
				paintNumber(context, setup.width, setup.height, painted);
				encoder.encode(
					new VideoFrame(canvas, {
						timestamp:
							setup.firstTimestamp +
							Math.round((painted * 1_000_000) / setup.framerate),
					}),
				);
				painted += 1;
			}, 1000 / setup.framerate);
			void keepPublishing({
				// As `keepVideoPlayed`: to the URI of a GOAWAY where it named one.
				async connect(goAway) {
					if (goAway !== undefined && setup.moveDelayMs !== undefined) {
						await new Promise((resolve) =>
							setTimeout(resolve, setup.moveDelayMs),
						);
					}
					return connectAnew(goAway?.newSessionUri || relayUrl, certSha256Hex);
				},
				trackNamespace,
				tracks: [encoder.publisher],
				delayMs: KEPT_DELAY_MS,
				onState(state) {
					const said =
						state.type === "publishing"
							? `publishing ${state.session}`
							: state.type === "moved"
								? `moved ${state.goAway.newSessionUri}`
								: state.type;
					kept.publishing.push(said);
					console.log(
						`kept publishing: ${said}${state.type === "failed" ? ` ${errorText(state.error)}` : ""}`,
					);
				},
			});
		},
	),

	keepVideoPlayed: withPlainErrors(
		async (
			relayUrl: string,
			certSha256Hex: string,
			namespace: string,
			trackName: string,
			codec: string,
		) => {
			const reading = new OffscreenCanvas(1, 1).getContext("2d", {
				willReadFrequently: true,
			});
			if (reading === null) throw new Error("no 2d context");
			let session = 0;
			const playout = createPlayout({
				// No sound is played here.
				sampleRate: 48_000,
				draw(frame) {
					kept.shown.push({ number: readNumber(reading, frame), session });
				},
			});
			void keepPlaying({
				// §10.4: "The client MUST use this URI for the new session if
				// provided"; the certificate is the one of the run.
				connect: (goAway) =>
					connectAnew(goAway?.newSessionUri || relayUrl, certSha256Hex),
				async play(connection) {
					const playback = await playVideoTrack(connection, {
						trackNamespace: namespaceTuple(namespace),
						trackName: utf8.encode(trackName),
						codec,
						onFrame: playout.onFrame,
					});
					return { ended: playback.ended };
				},
				movedBy: (end) => (end.type === "moved" ? end.goAway : undefined),
				delayMs: KEPT_DELAY_MS,
				onState(state) {
					if (state.type === "playing") session = state.session;
					const said =
						state.type === "playing"
							? `playing ${state.session}`
							: state.type === "moved"
								? `moved ${state.goAway.newSessionUri}`
								: state.type;
					kept.playing.push(said);
					console.log(
						`kept playing: ${said}${state.type === "failed" ? ` ${errorText(state.error)}` : ""}`,
					);
				},
			});
		},
	),

	keptReport: withPlainErrors(async () => ({
		publishing: [...kept.publishing],
		playing: [...kept.playing],
		shown: [...kept.shown],
	})),

	joinChat: withPlainErrors(
		async (
			room: string,
			name: string,
			catchUp?: PageChatCatchUp,
			publishNamespaceTokenHex?: string,
		) => {
			// A page that left joins again with a participant of its own.
			if (chat.stop.signal.aborted) chat.stop = new AbortController();
			chat.participant = await joinChat(connected(), {
				room,
				name,
				signal: chat.stop.signal,
				...(publishNamespaceTokenHex === undefined
					? {}
					: { publishNamespaceToken: fromHex(publishNamespaceTokenHex) }),
				...(catchUp?.catchUpLines === undefined
					? {}
					: { catchUpLines: catchUp.catchUpLines }),
				...(catchUp?.lastHad === undefined
					? {}
					: { lastHad: BigInt(catchUp.lastHad) }),
				onGap(gap) {
					console.log(`CHAT_GAP after=${gap.after} before=${gap.before}`);
					chat.gaps.push(`${gap.after}-${gap.before}`);
				},
				onLine(line) {
					console.log(
						`CHAT_LINE number=${line.number} name=${line.name} how=${line.how}`,
					);
					// WHY a bound: a run sends a few lines; this keeps a page from
					// holding a room's worth.
					if (chat.lines.length < 1_000) {
						chat.lines.push({
							number: line.number.toString(),
							name: line.name,
							text: line.text,
							how: line.how,
						});
					}
				},
				onMalformed(number, why) {
					chat.malformed.push(`${number} ${why}`);
				},
				onStatus(status) {
					console.log(
						`CHAT_STATUS heard=${status.heard} following=${status.following} last_end=${status.lastEnd ?? "none"}`,
					);
				},
			});
		},
	),

	chatSay: withPlainErrors(async (text: string) => {
		if (chat.participant === undefined) throw new Error("joinChat() first");
		const outcome = await chat.participant.say(text);
		return JSON.stringify(outcome, (_key, value: unknown) =>
			typeof value === "bigint" ? value.toString() : value,
		);
	}),

	chatLeave: withPlainErrors(async () => {
		chat.stop.abort();
	}),

	chatReport: withPlainErrors(async () => {
		const status = chat.participant?.status();
		return {
			lines: [...chat.lines],
			malformed: [...chat.malformed],
			gaps: [...chat.gaps],
			heard: status?.heard ?? false,
			following: status?.following ?? false,
			lastEnd: status?.lastEnd ?? null,
		};
	}),

	followCatalog: withPlainErrors(
		async (namespace: string, rendezvousTimeoutMs?: number) => {
			const follower = await followCatalog(connected(), {
				namespace: namespaceTuple(namespace),
				...(rendezvousTimeoutMs === undefined ? {} : { rendezvousTimeoutMs }),
			});
			const reading: CatalogReading = {
				states: [],
				joiningFetch: undefined,
				ended: undefined,
			};
			catalogReading = reading;
			void follower.ended.then((end) => {
				reading.ended =
					end.type === "publish-done"
						? `publish-done ${end.statusCode}`
						: end.type;
			});
			void follower.joiningFetch.then((fetched) => {
				reading.joiningFetch =
					fetched.type === "not-sent"
						? "not-sent"
						: fetched.type === "served"
							? `served ${fetched.objects}`
							: `failed ${fetched.objects}: ${errorText(fetched.error)}`;
			});
			void (async () => {
				const reader = follower.states.getReader();
				for (;;) {
					const next = await reader.read();
					if (next.done) return;
					const { current, outdated } = next.value;
					reading.states.push({
						catalog: current?.catalog ?? null,
						at:
							current === undefined
								? null
								: [current.at.group.toString(), current.at.object.toString()],
						outdated:
							outdated === undefined
								? null
								: `${outdated.because} at {${outdated.at.group}, ${outdated.at.object}}: ${outdated.detail}`,
					});
				}
			})().catch((error: unknown) => {
				console.log(`the catalog could not be read on: ${errorText(error)}`);
			});
		},
	),

	publishAudio: withPlainErrors(
		async (namespace: string, trackName: string) => {
			const moq = connected();
			const trackNamespace = namespaceTuple(namespace);
			const sampleRate = 48_000;
			const frames = 960;
			const encoder = await createAudioTrackEncoder({
				trackNamespace,
				trackName: utf8.encode(trackName),
				firstGroup: BigInt(Date.now()),
				codec: "opus",
				sampleRate,
				numberOfChannels: 1,
			});
			moq.serve({
				subscribe(request) {
					if (encoder.publisher.subscribe(request)) return;
					request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				},
				fetch(request) {
					if (encoder.publisher.fetch(request)) return;
					request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
				},
			});
			const publishing = {
				encoder,
				made: 0,
				timer: setInterval(() => {
					const piece = publishing.made;
					publishing.made += 1;
					const samples = new Float32Array(frames);
					for (let index = 0; index < frames; index++) {
						// The phase goes on from piece to piece.
						samples[index] =
							0.5 *
							Math.sin(
								(2 * Math.PI * 440 * (piece * frames + index)) / sampleRate,
							);
					}
					encoder.encode(
						new AudioData({
							format: "f32",
							sampleRate,
							numberOfFrames: frames,
							numberOfChannels: 1,
							timestamp: 7_000_000 + piece * 20_000,
							data: samples,
						}),
					);
				}, 20),
			};
			audioPublishing = publishing;
			await moq.publishNamespace(trackNamespace);
			console.log(`audio published: ${trackName}`);
		},
	),

	audioPublishReport: withPlainErrors(async () => {
		if (audioPublishing === undefined) {
			throw new Error("publishAudio() has not completed");
		}
		const stats = audioPublishing.encoder.stats();
		const sent = audioPublishing.encoder.publisher.stats();
		return {
			made: audioPublishing.made,
			submitted: stats.submitted,
			encoded: stats.encoded,
			encodedBytes: stats.encodedBytes,
			descriptionLength: stats.descriptionLength ?? null,
			encoderError: stats.encoderError ?? null,
			written: sent.written,
			skipped: { ...sent.skipped },
		};
	}),

	stopAudio: withPlainErrors(async () => {
		if (audioPublishing === undefined) {
			throw new Error("publishAudio() has not completed");
		}
		clearInterval(audioPublishing.timer);
		await audioPublishing.encoder.end();
	}),

	playAudio: withPlainErrors(async (namespace: string, trackName: string) => {
		const pieces: PageAudioReport["pieces"] = [];
		const playback = await playAudioTrack(connected(), {
			trackNamespace: namespaceTuple(namespace),
			trackName: utf8.encode(trackName),
			codec: "opus",
			sampleRate: 48_000,
			numberOfChannels: 1,
			onData(data) {
				const samples = new Float32Array(data.numberOfFrames);
				data.copyTo(samples, { planeIndex: 0, format: "f32-planar" });
				pieces.push({
					timestamp: data.timestamp,
					frames: data.numberOfFrames,
					sampleRate: data.sampleRate,
					channels: data.numberOfChannels,
					peak: samples.reduce(
						(peak, sample) => Math.max(peak, Math.abs(sample)),
						0,
					),
				});
				data.close();
			},
		});
		audioPlaying = { playback, pieces };
	}),

	audioReport: withPlainErrors(async (pieces: number, timeoutMs: number) => {
		const until = performance.now() + timeoutMs;
		for (;;) {
			const playing = audioPlaying;
			if (
				playing !== undefined &&
				(playing.pieces.length >= pieces || performance.now() >= until)
			) {
				const stats = playing.playback.stats();
				return {
					pieces: [...playing.pieces],
					received: stats.received,
					dropped: { ...stats.dropped },
					decoderError: stats.decoderError ?? null,
				};
			}
			await new Promise((resolve) => setTimeout(resolve, 20));
		}
	}),

	publishVideo: withPlainErrors(
		async (namespace: string, setup: PageVideoSetup) => {
			const moq = connected();
			const trackNamespace = namespaceTuple(namespace);
			const keyChunksNotSent: PageVideoPublishReport["keyChunksNotSent"] = [];
			const encoder = await createVideoTrackEncoder({
				...(setup.maxPayloadLength === undefined
					? {}
					: { maxPayloadLength: setup.maxPayloadLength }),
				onKeyChunksNotSent(event) {
					keyChunksNotSent.push(event);
					console.log(`key chunks not sent: ${JSON.stringify(event)}`);
				},
				trackNamespace,
				trackName: utf8.encode(setup.trackName),
				// MSF §6.1 has the time in milliseconds as one way to begin.
				firstGroup: BigInt(Date.now()),
				codec: setup.codec,
				width: setup.width,
				height: setup.height,
				framerate: setup.framerate,
				bitrate: setup.bitrate,
				keyFrameIntervalMs: setup.keyFrameIntervalMs,
				mapping: setup.mapping,
				...(setup.shortestGroupMs === undefined
					? {}
					: { dynamicGroups: { shortestGroupMs: setup.shortestGroupMs } }),
			});
			const catalog = createCatalogPublisher({
				namespace: trackNamespace,
				catalog: {
					version: "draft-01",
					tracks: [
						{
							name: setup.trackName,
							packaging: "loc",
							isLive: true,
							role: "video",
							codec: setup.catalogCodec,
							bitrate: setup.bitrate,
							width: setup.width,
							height: setup.height,
							framerate: setup.framerate,
						},
					],
				},
			});
			moq.serve({
				subscribe(request) {
					console.log(
						`asked: request_id=${request.requestId} track=${utf8Decoder.decode(request.trackName)}`,
					);
					if (catalog.subscribe(request)) return;
					if (encoder.publisher.subscribe(request)) return;
					request
						.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
						.catch(() => {});
				},
				fetch(request) {
					if (catalog.fetch(request)) return;
					if (encoder.publisher.fetch(request)) return;
					request
						.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
						.catch(() => {});
				},
			});
			const canvas = new OffscreenCanvas(setup.width, setup.height);
			const context = canvas.getContext("2d");
			if (context === null) throw new Error("no 2d context");
			const noise = setup.noise
				? noiseTiles(setup.width, Math.floor(setup.height / 2))
				: [];
			const publishing: VideoPublishing = {
				encoder,
				keyChunksNotSent,
				painted: 0,
				timer: setInterval(() => {
					const number = publishing.painted;
					publishing.painted += 1;
					paintNumber(context, setup.width, setup.height, number);
					const tile = noise[number % Math.max(noise.length, 1)];
					if (tile !== undefined) {
						context.drawImage(tile, 0, setup.height - tile.height);
					}
					const timeOf = (frame: number): number =>
						setup.firstTimestamp +
						Math.round((frame * 1_000_000) / setup.framerate);
					encoder.encode(new VideoFrame(canvas, { timestamp: timeOf(number) }));
					// Once, a frame that is not after the one before it: the
					// picture of this frame with the time of the last one. It is
					// not to be encoded.
					if (number === STALE_FRAME_AFTER) {
						encoder.encode(
							new VideoFrame(canvas, { timestamp: timeOf(number - 1) }),
						);
					}
				}, 1000 / setup.framerate),
			};
			videoPublishing = publishing;
			void encoder.failed.then((error) => {
				console.log(`video encoder failed: ${errorText(error)}`);
			});
			await moq.publishNamespace(trackNamespace);
			console.log(`video published: ${setup.trackName} ${setup.mapping}`);
		},
	),

	videoPublishReport: withPlainErrors(async () => {
		if (videoPublishing === undefined) {
			throw new Error("publishVideo() has not completed");
		}
		const stats = videoPublishing.encoder.stats();
		const sent = videoPublishing.encoder.publisher.stats();
		return {
			painted: videoPublishing.painted,
			submitted: stats.submitted,
			encoded: stats.encoded,
			keyChunks: stats.keyChunks,
			notEncoded: { ...stats.notEncoded },
			notSent: { ...stats.notSent },
			keyFramesAskedFor: stats.keyFramesAskedFor,
			encodedBytes: stats.encodedBytes,
			largestChunk: stats.largestChunk,
			encoderError: stats.encoderError ?? null,
			keyChunksNotSent: [...videoPublishing.keyChunksNotSent],
			written: sent.written,
			writeFailures: sent.writeFailures,
			newGroupsAskedFor: sent.newGroupsAskedFor,
			newGroupsBegun: sent.newGroupsBegun,
		};
	}),

	stopVideo: withPlainErrors(async () => {
		if (videoPublishing === undefined) {
			throw new Error("publishVideo() has not completed");
		}
		clearInterval(videoPublishing.timer);
		await videoPublishing.encoder.end();
		console.log("video stopped");
	}),

	publishBroadcast: withPlainErrors(async (namespace: string) => {
		const moq = connected();
		const trackNamespace = namespaceTuple(namespace);
		const sampleRate = 48_000;
		const video = await createVideoTrackEncoder({
			trackNamespace,
			trackName: utf8.encode(BROADCAST.video),
			firstGroup: BigInt(Date.now()),
			codec: BROADCAST.codec,
			width: BROADCAST.width,
			height: BROADCAST.height,
			framerate: BROADCAST.framerate,
			bitrate: BROADCAST.bitrate,
			keyFrameIntervalMs: 1000,
			mapping: "stream-per-group",
		});
		// The encoder counts from about here (`epoch` of `@xmoq/media`): a
		// frame's time is the page's clock since then, so that the Timestamp
		// of LOC is the moment it was painted.
		const videoBegan = Date.now();
		const audio = await createAudioTrackEncoder({
			trackNamespace,
			trackName: utf8.encode(BROADCAST.audio),
			firstGroup: BigInt(Date.now()),
			codec: "opus",
			sampleRate,
			numberOfChannels: 1,
		});
		const audioBegan = Date.now();
		const catalog = createCatalogPublisher({
			namespace: trackNamespace,
			catalog: {
				version: "draft-01",
				tracks: [
					{
						name: BROADCAST.video,
						packaging: "loc",
						isLive: true,
						role: "video",
						codec: BROADCAST.catalogCodec,
						bitrate: BROADCAST.bitrate,
						width: BROADCAST.width,
						height: BROADCAST.height,
						framerate: BROADCAST.framerate,
					},
					{
						name: BROADCAST.audio,
						packaging: "loc",
						isLive: true,
						role: "audio",
						codec: "opus",
						bitrate: 67_000,
						samplerate: sampleRate,
						channelConfig: "1",
					},
				],
			},
		});
		moq.serve({
			subscribe(request) {
				if (catalog.subscribe(request)) return;
				if (video.publisher.subscribe(request)) return;
				if (audio.publisher.subscribe(request)) return;
				request
					.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
					.catch(() => {});
			},
			fetch(request) {
				if (catalog.fetch(request)) return;
				if (video.publisher.fetch(request)) return;
				if (audio.publisher.fetch(request)) return;
				request
					.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
					.catch(() => {});
			},
		});
		/** The mark of the demo: the first 200 ms of every 2 seconds. */
		const marked = (wallClockMs: number): boolean => wallClockMs % 2000 < 200;
		const canvas = new OffscreenCanvas(BROADCAST.width, BROADCAST.height);
		const context = canvas.getContext("2d");
		if (context === null) throw new Error("no 2d context");
		const timers: ReturnType<typeof setInterval>[] = [];
		const publishing = { video, audio, catalog, timers, painted: 0, made: 0 };
		broadcastPublishing = publishing;
		timers[0] = setInterval(() => {
			const now = Date.now();
			publishing.painted += 1;
			context.fillStyle = "#444";
			context.fillRect(0, 0, BROADCAST.width, BROADCAST.height);
			// Something that moves, so that the frames differ.
			context.fillStyle = "#3a7";
			context.fillRect((publishing.painted * 4) % BROADCAST.width, 300, 40, 30);
			if (marked(now)) {
				// `MARK` of the demo: 80 pixels from (540, 20) of 640x360.
				context.fillStyle = "#fff";
				context.fillRect(540, 20, 80, 80);
			}
			video.encode(
				new VideoFrame(canvas, { timestamp: (now - videoBegan) * 1000 }),
			);
		}, 1000 / BROADCAST.framerate);
		const frames = 960;
		let phase = 0;
		let level = 0.25;
		// Every piece is 20 ms after the one before it, whenever its timer
		// came: a decoder wants the sound without gaps.
		const firstPiece = audioBegan;
		timers[1] = setInterval(() => {
			const piece = publishing.made;
			publishing.made += 1;
			const loud = marked(firstPiece + piece * 20);
			const samples = new Float32Array(frames);
			for (let index = 0; index < frames; index++) {
				phase += (2 * Math.PI * (loud ? 1000 : 440)) / sampleRate;
				// Towards its value and not at once: a step is a click.
				level += ((loud ? 0.6 : 0.25) - level) * 0.02;
				samples[index] = level * Math.sin(phase);
			}
			audio.encode(
				new AudioData({
					format: "f32",
					sampleRate,
					numberOfFrames: frames,
					numberOfChannels: 1,
					timestamp: piece * 20_000,
					data: samples,
				}),
			);
		}, 20);
		void video.failed.then((error) => {
			console.log(`video encoder failed: ${errorText(error)}`);
		});
		await moq.publishNamespace(trackNamespace);
		console.log("broadcast published");
	}),

	endBroadcast: withPlainErrors(async () => {
		if (broadcastPublishing === undefined) {
			throw new Error("publishBroadcast() has not completed");
		}
		const { video, audio, catalog, timers } = broadcastPublishing;
		for (const timer of timers) clearInterval(timer);
		// Each ends its track with PUBLISH_DONE (TRACK_ENDED). The session
		// and the namespace it announced stay.
		await Promise.all([video.end(), audio.end(), catalog.end()]);
		console.log("broadcast ended");
	}),

	keepBroadcastPlayed: withPlainErrors(
		async (
			relayUrl: string,
			certSha256Hex: string,
			namespace: string,
			waitMs: number,
		) => {
			const reading = new OffscreenCanvas(1, 1).getContext("2d", {
				willReadFrequently: true,
			});
			if (reading === null) throw new Error("no 2d context");
			let session = 0;
			const playout = createPlayout({
				// No sound is played here.
				sampleRate: 48_000,
				draw(frame) {
					kept.shown.push({ number: readNumber(reading, frame), session });
				},
			});
			const trackNamespace = namespaceTuple(namespace);
			void keepPlaying({
				connect: () => connectAnew(relayUrl, certSha256Hex),
				async play(connection) {
					// What this call subscribed to is let go when the video has
					// ended, so that the next call is no second subscription.
					const over = new AbortController();
					const follower = await followCatalog(connection, {
						namespace: trackNamespace,
						signal: over.signal,
						rendezvousTimeoutMs: waitMs,
					});
					const catalogs = follower.states.getReader();
					let video: string | undefined;
					while (video === undefined) {
						const next = await catalogs.read();
						if (next.done) {
							throw new Error("the catalog track ended without a catalog");
						}
						video = next.value.current?.catalog.tracks.find(
							(track) => track.role === "video",
						)?.name;
					}
					const playback = await playVideoTrack(connection, {
						trackNamespace,
						trackName: utf8.encode(video),
						codec: BROADCAST.catalogCodec,
						onFrame: playout.onFrame,
					});
					return {
						ended: playback.ended.then((end) => {
							over.abort();
							return end.type;
						}),
					};
				},
				// As the demo's `/watch?wait=1`: "not yet" is the relay's TIMEOUT
				// for a SUBSCRIBE it held, and DOES_NOT_EXIST.
				stay: {
					notYet: (error: unknown) =>
						error instanceof RequestFailedError &&
						(error.errorCode === RequestErrorCode.TIMEOUT ||
							error.errorCode === RequestErrorCode.DOES_NOT_EXIST),
				},
				delayMs: KEPT_DELAY_MS,
				onState(state) {
					if (state.type === "playing") session = state.session;
					const said =
						state.type === "playing"
							? `playing ${state.session}`
							: state.type === "not-yet"
								? `not-yet ${errorText(state.error)}`
								: state.type;
					kept.playing.push(said);
					console.log(
						`kept playing: ${said}${state.type === "failed" ? ` ${errorText(state.error)}` : ""}`,
					);
				},
			});
		},
	),

	publishVoice: withPlainErrors(async (namespace: string, voice: string) => {
		const moq = connected();
		const trackNamespace = namespaceTuple(namespace);
		const sampleRate = 48_000;
		const audio = await createAudioTrackEncoder({
			trackNamespace,
			trackName: utf8.encode(VOICE.audio),
			firstGroup: BigInt(Date.now()),
			codec: "opus",
			sampleRate,
			numberOfChannels: 1,
		});
		const catalog = createCatalogPublisher({
			namespace: trackNamespace,
			catalog: {
				version: "draft-01",
				tracks: [
					{
						name: VOICE.audio,
						packaging: "loc",
						isLive: true,
						role: "audio",
						codec: "opus",
						bitrate: 32_000,
						samplerate: sampleRate,
						channelConfig: "1",
					},
				],
			},
		});
		moq.serve({
			subscribe(request) {
				if (catalog.subscribe(request)) return;
				if (audio.publisher.subscribe(request)) return;
				request
					.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
					.catch(() => {});
			},
			fetch(request) {
				if (catalog.fetch(request)) return;
				if (audio.publisher.fetch(request)) return;
				request
					.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
					.catch(() => {});
			},
		});
		const spoken = voiceAt48kHz(voice);
		const round = new Float32Array(
			spoken.length + (VOICE.pauseMs * sampleRate) / 1000,
		);
		round.set(spoken);
		const frames = 960;
		const counted = { made: 0 };
		// Every piece is 20 ms after the one before it, whenever its timer
		// came, as in `publishBroadcast`.
		setInterval(() => {
			const piece = counted.made;
			counted.made += 1;
			const samples = new Float32Array(frames);
			for (let index = 0; index < frames; index++) {
				samples[index] = round[(piece * frames + index) % round.length] ?? 0;
			}
			audio.encode(
				new AudioData({
					format: "f32",
					sampleRate,
					numberOfFrames: frames,
					numberOfChannels: 1,
					timestamp: piece * 20_000,
					data: samples,
				}),
			);
		}, 20);
		voicePublishing = { audio, round: round.length, counted };
		await moq.publishNamespace(trackNamespace);
		console.log("voice published");
	}),

	followTranscript: withPlainErrors(
		async (listened: string, latest: number) => {
			followTranscript(connected(), {
				listened: listened.split("/"),
				latest,
				onSpeech(speech) {
					transcriptPieces.push({
						number: speech.number.toString(),
						from: speech.from,
						text: speech.text,
						how: speech.how,
					});
				},
			});
		},
	),

	transcriptReport: withPlainErrors(
		async (): Promise<PageTranscriptPiece[]> => [...transcriptPieces],
	),

	voiceReport: withPlainErrors(async (): Promise<PageVoiceReport> => {
		if (voicePublishing === undefined) {
			throw new Error("publishVoice() has not completed");
		}
		const { audio, counted, round } = voicePublishing;
		const { made } = counted;
		return {
			made,
			voices: Math.ceil((made * 960) / round),
			audioEncoderError: audio.stats().encoderError ?? null,
			audioWritten: audio.publisher.stats().written,
		};
	}),

	broadcastReport: withPlainErrors(async (): Promise<PageBroadcastReport> => {
		if (broadcastPublishing === undefined) {
			throw new Error("publishBroadcast() has not completed");
		}
		const { video, audio, painted, made } = broadcastPublishing;
		return {
			painted,
			made,
			videoEncoderError: video.stats().encoderError ?? null,
			audioEncoderError: audio.stats().encoderError ?? null,
			videoWriteFailures: video.publisher.stats().writeFailures,
			audioWritten: audio.publisher.stats().written,
			audioSkipped: { ...audio.publisher.stats().skipped },
		};
	}),

	playVideo: withPlainErrors(
		async (
			namespace: string,
			trackName: string,
			codec: string,
			readNumbers = false,
			joiningFetch = false,
			deliveryTimeoutMs?: number,
			newGroupRequest = false,
		) => {
			const asked = performance.now();
			const reading = readNumbers
				? new OffscreenCanvas(1, 1).getContext("2d", {
						willReadFrequently: true,
					})
				: null;
			const frames: PageVideoReport["frames"] = [];
			let firstFrameAt: number | undefined;
			const playback = await playVideoTrack(connected(), {
				trackNamespace: namespaceTuple(namespace),
				trackName: utf8.encode(trackName),
				codec,
				joiningFetch,
				newGroupRequest,
				...(deliveryTimeoutMs === undefined
					? {}
					: {
							deliveryTimeouts: {
								subgroupMs: deliveryTimeoutMs,
								objectMs: deliveryTimeoutMs,
							},
						}),
				onFrame(frame) {
					firstFrameAt ??= performance.now();
					frames.push({
						timestamp: frame.timestamp,
						width: frame.displayWidth,
						height: frame.displayHeight,
						...(reading === null ? {} : { number: readNumber(reading, frame) }),
					});
					// The frame is this page's from here on.
					frame.close();
				},
			});
			const playing: VideoPlaying = {
				playback,
				frames,
				ended: undefined,
				firstFrameAfterMs: () =>
					firstFrameAt === undefined ? null : Math.round(firstFrameAt - asked),
			};
			videoPlaying = playing;
			void playback.ended.then((end) => {
				switch (end.type) {
					case "subscription-ended":
						playing.ended = `subscription-ended ${end.end.type}`;
						break;
					case "moved":
						playing.ended = `moved ${end.goAway.newSessionUri}`;
						break;
					case "decoder-failed":
					case "failed":
						playing.ended = `${end.type} ${errorText(end.error)}`;
						break;
				}
				console.log(`video ended: ${playing.ended}`);
			});
		},
	),

	videoDecoderSupport: withPlainErrors(async (codec: string) => {
		if (typeof VideoDecoder !== "function") return "no VideoDecoder";
		const { supported } = await VideoDecoder.isConfigSupported({
			codec,
			optimizeForLatency: true,
		});
		return supported === true ? "supported" : "not supported";
	}),

	updateVideoDeliveryTimeout: withPlainErrors(async (subgroupMs: number) => {
		if (videoPlaying === undefined) return "nothing is played";
		try {
			await videoPlaying.playback.setDeliveryTimeouts({ subgroupMs });
			return "accepted";
		} catch (error) {
			return `not accepted: ${errorText(error)}`;
		}
	}),

	videoReport: withPlainErrors(async (frames: number, timeoutMs: number) => {
		const until = performance.now() + timeoutMs;
		for (;;) {
			const playing = videoPlaying;
			if (
				playing !== undefined &&
				(playing.frames.length >= frames ||
					playing.ended !== undefined ||
					performance.now() >= until)
			) {
				const stats = playing.playback.stats();
				return {
					frames: [...playing.frames],
					stats: {
						decoded: stats.decoded,
						submitted: stats.submitted,
						dropped: { ...stats.dropped },
						decoderFailures: stats.decoderFailures,
						lastDecoderError: stats.lastDecoderError ?? null,
						endOfTrack: stats.endOfTrack,
						joiningFetch: stats.joiningFetch,
					},
					ended: playing.ended ?? null,
					firstFrameAfterMs: playing.firstFrameAfterMs(),
				};
			}
			await new Promise((resolve) => setTimeout(resolve, 20));
		}
	}),

	catalogEnded: withPlainErrors(async () => {
		for (;;) {
			const ended = catalogReading?.ended;
			if (ended !== undefined) return ended;
			await new Promise((resolve) => setTimeout(resolve, 20));
		}
	}),

	catalogReport: withPlainErrors(async (states: number) => {
		for (;;) {
			const reading = catalogReading;
			if (
				reading !== undefined &&
				reading.joiningFetch !== undefined &&
				reading.states.length >= states
			) {
				return {
					states: [...reading.states],
					joiningFetch: reading.joiningFetch,
				};
			}
			await new Promise((resolve) => setTimeout(resolve, 20));
		}
	}),

	publishCatalog: withPlainErrors(
		async (namespace: string, catalog: Catalog) => {
			const moq = connected();
			const trackNamespace = namespaceTuple(namespace);
			const publisher = createCatalogPublisher({
				namespace: trackNamespace,
				catalog,
			});
			catalogPublisher = publisher;
			// Before the namespace is announced: a relay asks as soon as it has
			// a subscriber.
			moq.serve({
				subscribe(request) {
					console.log(
						`asked: request_id=${request.requestId} track=${utf8Decoder.decode(request.trackName)}`,
					);
					if (publisher.subscribe(request)) return;
					request
						.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
						.catch(() => {});
				},
				fetch(request) {
					console.log(
						`fetch asked: request_id=${request.requestId} start={${request.startLocation.group}, ${request.startLocation.object}} end={${request.endLocation.group}, ${request.endLocation.object}}`,
					);
					if (publisher.fetch(request)) return;
					request
						.reject(RequestErrorCode.DOES_NOT_EXIST, "not a track of this page")
						.catch(() => {});
				},
			});
			await moq.publishNamespace(trackNamespace);
			const { at } = publisher.current;
			return [at.group.toString(), at.object.toString()];
		},
	),

	updateCatalog: withPlainErrors(async (catalog: Catalog) => {
		if (catalogPublisher === undefined) {
			throw new Error("publishCatalog() has not completed");
		}
		catalogPublisher.update(catalog);
		const { at } = catalogPublisher.current;
		return [at.group.toString(), at.object.toString()];
	}),

	endCatalog: withPlainErrors(async (catalog: Catalog) => {
		if (catalogPublisher === undefined) {
			throw new Error("publishCatalog() has not completed");
		}
		// Nothing between the two: the last catalog is to reach a subscriber
		// all the same.
		catalogPublisher.update(catalog);
		const ending = catalogPublisher.end();
		const { at } = catalogPublisher.current;
		await ending;
		return [at.group.toString(), at.object.toString()];
	}),

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
				// Object 0 of three Groups it published before anyone subscribed,
				// which no relay has seen. What it sends comes after them.
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

	sendLargeGroup: withPlainErrors(
		async (group: string, objects: number, bytes: number) => {
			const subgroup = await publishedTrack().openSubgroup({
				groupId: BigInt(group),
				subgroupId: { mode: "Explicit", value: 0n },
				publisherPriority: 0,
				properties: false,
				endOfGroup: true,
			});
			for (let object = 0; object < objects; object++) {
				// The payload of each Object is its own.
				await subgroup.write({
					objectId: BigInt(object),
					payload: new Uint8Array(bytes).fill(object + 1),
				});
			}
			await subgroup.close();
			console.log(
				`large group sent: group=${group} objects=${objects} payload_len=${bytes}`,
			);
		},
	),

	objectsOfGroup: withPlainErrors(
		async (group: string) =>
			reading?.objects.filter((object) => object.group === group).length ?? 0,
	),

	receivedOfGroup: withPlainErrors(async (group: string) =>
		JSON.stringify(
			reading?.objects.filter((object) => object.group === group) ?? [],
		),
	),

	writeToSubgroup: withPlainErrors(
		async (
			group: string,
			subgroup: string,
			publisherPriority: number,
			object: string,
			payloadHex: string,
			properties: PageProperty[],
		) => {
			const key = `${group}/${subgroup}`;
			let writer = openSubgroups.get(key);
			if (writer === undefined) {
				writer = await publishedTrack().openSubgroup({
					groupId: BigInt(group),
					subgroupId: { mode: "Explicit", value: BigInt(subgroup) },
					publisherPriority,
					// §11.4.2: the bit is of the stream.
					properties: properties.length > 0,
					endOfGroup: false,
				});
				openSubgroups.set(key, writer);
			}
			await writer.write({
				objectId: BigInt(object),
				payload: fromHex(payloadHex),
				properties: properties.map(toKeyValuePair),
			});
			console.log(
				`object sent: group=${group} subgroup=${subgroup} object=${object} priority=${publisherPriority} properties=${properties.length}`,
			);
		},
	),

	closeSubgroups: withPlainErrors(async (group: string) => {
		for (const [key, writer] of openSubgroups) {
			if (!key.startsWith(`${group}/`)) continue;
			openSubgroups.delete(key);
			await writer.close();
		}
		console.log(`subgroups closed: group=${group}`);
	}),

	fetchAndLeaveUnread: withPlainErrors(
		async (asked: PageFetch, cancelAfterMs: number) => {
			const fetch = await connected().fetch({
				range: fetchRangeOf(asked),
				parameters: { subscriberPriority: 128 },
			});
			console.log(
				`fetch sent: request_id=${fetch.requestId} type=${asked.type}`,
			);
			const ok = await fetch.response;
			// `fetch.body` is not read: the library takes from the transport
			// only as fast as it is read, so the publisher's writes stand
			// still when the windows are full.
			await new Promise((resolve) => setTimeout(resolve, cancelAfterMs));
			await fetch.cancel();
			const end = await fetch.ended;
			const reported: { endLocation: [string, string]; ended: string } = {
				endLocation: [
					ok.endLocation.group.toString(),
					ok.endLocation.object.toString(),
				],
				ended: end.type,
			};
			console.log(`fetch left unread: ${JSON.stringify(reported)}`);
			return reported;
		},
	),

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
