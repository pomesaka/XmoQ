// A participant of a room of the chat over a connection of @xmoq/moqt:
// it says its lines on its own track `said` and reads the room's `lines`,
// which the gatherer (`crates/chat`) publishes. Section numbers are of
// draft-ietf-moq-transport-18.

import {
	decodeFetchBody,
	type MoqtConnection,
	type PeerSubscribe,
	type PublishedTrack,
	RequestErrorCode,
	type Subscription,
	type SubscriptionEnd,
} from "@xmoq/moqt";
import { HISTORY_LINES, historyStart } from "../core/history.ts";
import {
	type ChatLimits,
	chatLimits,
	type KeptBack,
	keepBack,
	RateWindow,
} from "../core/limits.ts";
import {
	LINES_TRACK,
	personNamespace,
	roomNamespace,
	SAID_TRACK,
} from "../core/names.ts";
import {
	type Line,
	type Malformed,
	readLine,
	saidPayload,
} from "../core/payloads.ts";
import { SeenLines } from "../core/seen-lines.ts";

/**
 * How many line numbers are remembered, so that a line is handed on once
 * (`SeenLines`). WHY 200: what the demo's page keeps of the chat. Not a
 * setting. **Provisional**.
 */
const SEEN_LINES = 200;

/**
 * The longest payload of a line read from the Joining FETCH: 64 KiB, what
 * the gatherer reads of one stream of `said` (`PAYLOAD_MOST`), which holds
 * the longest line it takes, 8,192 bytes of text, however it is escaped.
 */
const MAX_LINE_PAYLOAD = 64 * 1024;

/**
 * How long the SUBSCRIBE for `lines` asks the relay to wait for the room to
 * be published (RENDEZVOUS_TIMEOUT, §10.2.6). WHY: the gatherer publishes a
 * room only once it follows somebody in it (`crates/chat/README.md`), so the
 * first participant's SUBSCRIBE would otherwise come before the room's
 * PUBLISH and be refused. **Provisional**: meant to be longer than the
 * gatherer takes to be told of a participant and to publish, which was not
 * measured (the interop run `chat-pages.ts` had it answered within its step
 * of 20 s); the own relay holds 30 seconds at the most. A relay that does
 * not hold it (moqtail's 0.15.0) refuses at once, and the SUBSCRIBE is made
 * again `AGAIN_MS` later.
 */
const RENDEZVOUS_MS = 10_000;

/** How long the Joining FETCH has, as `followCatalog` of @xmoq/msf gives its own, and for the same reason. **Provisional**. */
const FETCH_TIMEOUT_MS = 15_000;

/**
 * How long to wait before the room is subscribed to again after its
 * subscription ended or was refused: the gatherer started again, or the
 * room was not published in time. **Provisional**, as the 2 seconds the
 * demo's pages wait before they connect again.
 */
const AGAIN_MS = 2_000;

/**
 * Lines being written on `said` at once. One more is not sent. WHY a bound:
 * each line opens a stream, which waits for the relay to allow it, and a
 * page that says lines faster than they go out would hold them all.
 * **Provisional**.
 */
const MAX_WRITING = 16;

/** A line of the room, and how it came. */
export interface ReceivedLine extends Line {
	/** By the subscription as it was said, or by the Joining FETCH, from before. */
	how: "live" | "history";
}

/** What became of a line the participant said. */
export type SayOutcome =
	/** Written on `said` in Group `group`; the gatherer may still drop it (its rate is counted where the line arrives). */
	| { type: "sent"; group: bigint }
	/** Not sent, for a limit of the chat or a text no reader takes. */
	| { type: "kept-back"; why: KeptBack }
	/**
	 * Not sent: nobody has subscribed to `said` yet (the gatherer subscribes
	 * from the newest Object on, so a line before it would never reach it),
	 * too many lines are being written, or the writing failed.
	 */
	| {
			type: "not-sent";
			why: "nobody-subscribed" | "busy" | "failed";
			error?: unknown;
	  };

/** How far the participant is, for whoever shows it. */
export interface ChatStatus {
	/** Whether the gatherer, through the relay, has subscribed to `said`. */
	heard: boolean;
	/** Whether the room's `lines` are subscribed to. */
	following: boolean;
	/** How the last subscription to `lines` ended, or why it was refused; undefined before. */
	lastEnd: string | undefined;
}

export interface JoinChatOptions {
	room: string;
	name: string;
	/** The limits of a line; `DEFAULT_LIMITS` for what is left out. */
	limits?: Partial<ChatLimits>;
	/**
	 * Called once for each line of the room, by its number. Lines can come
	 * out of their order: a line said now can come before the history.
	 */
	onLine(line: ReceivedLine): void;
	/** Called for an Object of `lines` that is no line, with its Group ID and why. */
	onMalformed?(number: bigint, why: Malformed): void;
	/** Called when `status()` changes. */
	onStatus?(status: ChatStatus): void;
	/** Stops the participant: the subscription to `lines` is cancelled, and no SUBSCRIBE for `said` is accepted after. */
	signal?: AbortSignal;
	/** The wall clock, in milliseconds since the Unix epoch: `sentAt`, and the rate. `Date.now` when left out. */
	now?: () => number;
}

export interface ChatParticipant {
	/** Says one line, if the limits let it. Never rejects. */
	say(text: string): Promise<SayOutcome>;
	status(): ChatStatus;
}

const delay = (ms: number, signal: AbortSignal): Promise<void> =>
	new Promise((resolve) => {
		const timer = setTimeout(resolve, ms);
		signal.addEventListener(
			"abort",
			() => {
				clearTimeout(timer);
				resolve();
			},
			{ once: true },
		);
	});

const same = (a: readonly Uint8Array[], b: readonly Uint8Array[]): boolean =>
	a.length === b.length &&
	a.every(
		(field, index) =>
			field.length === b[index]?.length &&
			field.every((byte, at) => byte === b[index]?.[at]),
	);

function described(end: SubscriptionEnd): string {
	switch (end.type) {
		case "publish-done":
			return `PUBLISH_DONE 0x${end.statusCode.toString(16)} ${end.errorReason}`;
		case "peer-reset":
			return `reset ${end.code ?? "without a code"}`;
		case "session-closed":
			return "the session closed";
		default:
			return end.type;
	}
}

/**
 * Joins the room `room` as `name`: announces `xmoq/chat/<room>/<name>`
 * (PUBLISH_NAMESPACE), serves the relay's SUBSCRIBE for `said` on it, and
 * subscribes to the room's `lines`, with a Joining FETCH for the newest
 * `HISTORY_LINES` where the room has lines. When the subscription ends it
 * is made again, `AGAIN_MS` later, for as long as the session lasts.
 *
 * **It names the connection's `serve` handlers**: a SUBSCRIBE for another
 * track, and every FETCH, are refused. Give it a connection of its own.
 *
 * Resolves once the namespace is announced; rejects as
 * `publishNamespace` does, or with a `RangeError` for limits out of range.
 */
export async function joinChat(
	connection: Pick<
		MoqtConnection,
		"serve" | "publishNamespace" | "subscribe" | "fetch" | "close" | "closed"
	>,
	options: JoinChatOptions,
): Promise<ChatParticipant> {
	const limits = chatLimits(options.limits);
	const now = options.now ?? Date.now;
	const stop = new AbortController();
	options.signal?.addEventListener("abort", () => stop.abort(), { once: true });
	void connection.closed.then(() => stop.abort());
	const ours = personNamespace(options.room, options.name);
	const said = new TextEncoder().encode(SAID_TRACK);

	const status: ChatStatus = {
		heard: false,
		following: false,
		lastEnd: undefined,
	};
	const changed = (change: Partial<ChatStatus>): void => {
		Object.assign(status, change);
		options.onStatus?.({ ...status });
	};

	/** The relay's subscription to `said`; one at a time, as the library refuses a second (§5.1). */
	let listener: PublishedTrack | undefined;
	/**
	 * The Group of the next line. WHY from the time in milliseconds: a page
	 * that comes again on the same name is a new publisher of the same
	 * track, and §2.4 has one that "intends to resume publishing on the
	 * same Track" "set the initial Group ID to a unique value guaranteed to
	 * be larger than all previously used groups ... based on the current
	 * time", as the demo's other tracks do.
	 */
	let nextGroup = BigInt(now());
	/** The Group of the last line written, for the LARGEST_OBJECT of a later SUBSCRIBE_OK (§10.2.11). */
	let lastGroup: bigint | undefined;
	let writing = 0;

	const answer = (request: PeerSubscribe): void => {
		const asked =
			same(request.trackNamespace, ours) &&
			same([request.trackName], [said]) &&
			!stop.signal.aborted;
		if (!asked) {
			void request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
			return;
		}
		void request
			.accept({
				// §10.2.11: "If Objects have been published on this Track the
				// Publisher MUST include this parameter".
				parameters:
					lastGroup === undefined
						? {}
						: { largestObject: { group: lastGroup, object: 0n } },
			})
			.then((track) => {
				listener = track;
				changed({ heard: true });
				void track.ended.then(() => {
					if (listener !== track) return;
					listener = undefined;
					changed({ heard: false });
				});
			})
			.catch(() => {});
	};
	connection.serve({ subscribe: answer });
	await connection.publishNamespace(ours);

	const seen = new SeenLines(SEEN_LINES);
	const take = (
		number: bigint,
		payload: Uint8Array,
		how: ReceivedLine["how"],
	): void => {
		const read = readLine(number, payload);
		if (!read.ok) {
			options.onMalformed?.(number, read.why);
			return;
		}
		if (seen.take(number)) options.onLine({ ...read.value, how });
	};

	const history = async (subscription: Subscription): Promise<void> => {
		// §10.12.2: a Joining FETCH for a track whose SUBSCRIBE_OK said that
		// nothing was published is refused with INVALID_RANGE; a room
		// without lines has nothing to fetch.
		const largest = subscription.parameters.largestObject;
		if (largest === undefined) return;
		const fetch = await connection.fetch(
			{
				range: {
					// An Absolute Joining FETCH from the Group `historyStart`
					// gives (§10.12.2.2): the reason is there.
					fetchType: "AbsoluteJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: historyStart(largest.group),
				},
			},
			{
				signal: AbortSignal.any([
					stop.signal,
					AbortSignal.timeout(FETCH_TIMEOUT_MS),
				]),
			},
		);
		await fetch.response;
		const reader = decodeFetchBody(connection, fetch, {
			limits: { maxObjectPayloadLength: MAX_LINE_PAYLOAD },
		}).getReader();
		let objects = 0;
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			if (next.value.type !== "object") continue;
			const { groupId, objectId, payload } = next.value.object;
			if (objectId !== 0n) continue;
			take(groupId, payload, "history");
			objects += 1;
			// A relay or a gatherer that sends more than was asked for is not
			// read further.
			if (objects >= HISTORY_LINES) {
				await reader.cancel();
				return;
			}
		}
	};

	const live = async (subscription: Subscription): Promise<void> => {
		const reader = subscription.objects.getReader();
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			const { groupId, object } = next.value;
			// A line is Object 0 of its Group, and a status says no line.
			if (object.status !== "Normal" || object.objectId !== 0n) continue;
			take(groupId, object.payload, "live");
		}
	};

	const follow = async (): Promise<void> => {
		while (!stop.signal.aborted) {
			let subscription: Subscription;
			try {
				subscription = await connection.subscribe({
					trackNamespace: roomNamespace(options.room),
					trackName: new TextEncoder().encode(LINES_TRACK),
					parameters: { rendezvousTimeout: BigInt(RENDEZVOUS_MS) },
				});
			} catch (error) {
				changed({ lastEnd: `refused: ${String(error).slice(0, 120)}` });
				await delay(AGAIN_MS, stop.signal);
				continue;
			}
			const cancel = (): void => {
				void subscription.cancel().catch(() => {});
			};
			stop.signal.addEventListener("abort", cancel, { once: true });
			changed({ following: true });
			// WHY a failed FETCH is no failure of the participant: the lines
			// that come live are there all the same; the history is missing.
			const fetched = history(subscription).catch(() => {});
			await live(subscription).catch(() => {});
			const end = await subscription.ended;
			await fetched;
			stop.signal.removeEventListener("abort", cancel);
			changed({ following: false, lastEnd: described(end) });
			await delay(AGAIN_MS, stop.signal);
		}
	};
	void follow();

	const window = new RateWindow(limits.linesPerSecond);
	return {
		status: () => ({ ...status }),
		async say(text) {
			const why = keepBack(text, limits, window, now());
			if (why !== undefined) return { type: "kept-back", why };
			const track = listener;
			if (track === undefined)
				return { type: "not-sent", why: "nobody-subscribed" };
			if (writing >= MAX_WRITING) return { type: "not-sent", why: "busy" };
			const group = nextGroup;
			nextGroup += 1n;
			writing += 1;
			try {
				const payload = saidPayload({ text, sentAt: now() });
				const stream = await track.openSubgroup({
					groupId: group,
					subgroupId: { mode: "Zero" },
					properties: false,
					// One line is the whole of its Group.
					endOfGroup: true,
				});
				try {
					await stream.write({ objectId: 0n, payload });
					await stream.close();
				} catch (error) {
					await stream.reset().catch(() => {});
					throw error;
				}
				if (lastGroup === undefined || group > lastGroup) lastGroup = group;
				return { type: "sent", group };
			} catch (error) {
				return { type: "not-sent", why: "failed", error };
			} finally {
				writing -= 1;
			}
		},
	};
}
