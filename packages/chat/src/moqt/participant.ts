// A participant of a room of the chat over a connection of @xmoq/moqt:
// it says its lines on its own track `said` and its reactions on `reacted`,
// and reads the room's `lines`, and where it is asked to, its `reactions`
// and `present`, which the gatherer (`crates/chat`) publishes. Section numbers are of
// draft-ietf-moq-transport-18.

import {
	createAskAgain,
	decodeFetchBody,
	type MoqtConnection,
	type PeerSubscribe,
	type PublishedTrack,
	RequestErrorCode,
	RequestFailedError,
	type Subscription,
	type SubscriptionEnd,
	sameFullTrackName,
	waitFor,
} from "@xmoq/moqt";
import {
	catchUpLines,
	historyToFetch,
	type LinesGap,
} from "../core/history.ts";
import {
	type ChatLimits,
	chatLimits,
	type KeptBack,
	keepBack,
	keepBackReaction,
	RateWindow,
} from "../core/limits.ts";
import {
	type ChatRoom,
	LINES_TRACK,
	PRESENT_TRACK,
	personNamespace,
	REACTED_TRACK,
	REACTIONS_TRACK,
	roomNamespace,
	SAID_TRACK,
} from "../core/names.ts";
import {
	type Line,
	type Malformed,
	type Present,
	type Reactions,
	reactedPayload,
	readLine,
	readPresent,
	readReactions,
	saidPayload,
} from "../core/payloads.ts";
import { SeenLines } from "../core/seen-lines.ts";

/**
 * How many line numbers are remembered, so that a line is handed on once
 * (`SeenLines`), where `catchUpLines` is no more. WHY 200: what the demo's
 * page keeps of the chat. **Provisional**. WHY as many as `catchUpLines`
 * where that is more: the lines of a FETCH that catches up come with older
 * numbers than live lines that came meanwhile, and a number below all those
 * remembered is passed over as had.
 */
const SEEN_LINES = 200;

/**
 * The longest payload of a line read from the Joining FETCH or the
 * subscription to `lines`: 64 KiB, what
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
 * again after the pause of `createAskAgain` (@xmoq/moqt).
 */
const RENDEZVOUS_MS = 10_000;

/** How long the Joining FETCH has, as `followCatalog` of @xmoq/msf gives its own, and for the same reason. **Provisional**. */
const FETCH_TIMEOUT_MS = 15_000;

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
	room: ChatRoom;
	name: string;
	/**
	 * An AUTHORIZATION TOKEN (§10.2.2, the bytes of the Token structure) for
	 * the PUBLISH_NAMESPACE of `xmoq/chat/<kind>/<room>/<name>` alone: in the demo's
	 * relay, one that allows publishing in that namespace. Left out, the
	 * PUBLISH_NAMESPACE carries none of its own.
	 *
	 * The token for reading the room goes on the connection instead
	 * (`authorizationToken` of `connect`): its SETUP, the SUBSCRIBE for
	 * `lines` and the Joining FETCH carry it, and the PUBLISH_NAMESPACE as
	 * well, before this one. WHY a token of the request beside the
	 * connection's: the two name different namespaces and operations, and
	 * §10.2.2 has the parameter "authorize the sender to perform the
	 * operation carrying the parameter", so each goes where it is needed.
	 * WHY NOT this one on the SUBSCRIBE and the FETCH as well: it allows
	 * nothing there.
	 */
	publishNamespaceToken?: Uint8Array;
	/** The limits of a line; `DEFAULT_LIMITS` for what is left out. */
	limits?: Partial<ChatLimits>;
	/**
	 * Called once for each line of the room, by its number. Lines can come
	 * out of their order: a line said now can come before the history.
	 */
	onLine(line: ReceivedLine): void;
	/**
	 * Called where lines of the room were not fetched, before the lines
	 * after them: a subscription made again missed more lines than
	 * `catchUpLines`, and only the newest of them are fetched.
	 */
	onGap?(gap: LinesGap): void;
	/**
	 * How many of the lines it missed the participant fetches at the most,
	 * when its subscription to `lines` is made again (or when it comes with
	 * `lastHad`): `CATCH_UP_LINES` (200, what the demo's page keeps) when
	 * left out, from 1 to `CATCH_UP_LINES_MOST` (1,000). Where it missed more,
	 * the newest are fetched and `onGap` tells of the rest.
	 */
	catchUpLines?: number;
	/**
	 * The number of the newest line the caller already has, from an earlier
	 * participant (a page that connected again): the first FETCH then catches
	 * up from the line after it, as a subscription made again does. Left
	 * out, the first FETCH brings the newest 16 lines.
	 */
	lastHad?: bigint;
	/** Called for an Object of `lines` that is no line, with its Group ID and why. */
	onMalformed?(number: bigint, why: Malformed): void;
	/**
	 * Called with the reactions of the room over each while, as the gatherer
	 * counts them on `reactions` (decided on 2026-10-26). Left out, the
	 * track is not subscribed to. They come live alone: nothing of them is
	 * kept.
	 */
	onReactions?(reactions: Reactions): void;
	/**
	 * Called with how many the gatherer follows in the room, from `present`:
	 * the latest when the subscription is made, and each change after it.
	 * Left out, the track is not subscribed to.
	 */
	onPresent?(present: Present): void;
	/** Called when `status()` changes. */
	onStatus?(status: ChatStatus): void;
	/**
	 * Stops the participant: the namespace is withdrawn, the subscription to
	 * `lines` is cancelled, and no SUBSCRIBE for `said` is accepted after.
	 * Before the namespace is announced, `joinChat` then rejects with
	 * `RequestAbortedError` of `cancelled`.
	 */
	signal?: AbortSignal;
	/** The wall clock, in milliseconds since the Unix epoch: `sentAt`, and the rate. `Date.now` when left out. */
	now?: () => number;
}

export interface ChatParticipant {
	/** Says one line, if the limits let it. Never rejects. */
	say(text: string): Promise<SayOutcome>;
	/** Says one reaction on `reacted`, if the limits let it (`reactionsPerSecond`). Never rejects. */
	react(emoji: string): Promise<SayOutcome>;
	status(): ChatStatus;
}

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
 * Joins the room `room` as `name`: announces `xmoq/chat/<kind>/<room>/<name>`
 * (PUBLISH_NAMESPACE), serves the relay's SUBSCRIBE for `said` on it, and
 * subscribes to the room's `lines`, with a Joining FETCH for the newest
 * `HISTORY_LINES` where the room has lines. When the subscription ends it
 * is made again after the pause of `createAskAgain` (@xmoq/moqt): 1 s,
 * or the Retry Interval of a refusal, as the terminal viewer asks for the
 * same track. For as long as the session lasts, and
 * its Joining FETCH asks for the lines after the last one had
 * (`historyToFetch`).
 *
 * **It names the connection's `serve` handlers**: a SUBSCRIBE for another
 * track, and every FETCH, are refused. Give it a connection of its own.
 *
 * Resolves once the namespace is announced; rejects as
 * `publishNamespace` does, or with a `RangeError` for limits or
 * `catchUpLines` out of range, or a `lastHad` below 0.
 */
export async function joinChat(
	connection: Pick<
		MoqtConnection,
		"serve" | "publishNamespace" | "subscribe" | "fetch" | "close" | "closed"
	>,
	options: JoinChatOptions,
): Promise<ChatParticipant> {
	const limits = chatLimits(options.limits);
	const catchUp = catchUpLines(options.catchUpLines);
	if (options.lastHad !== undefined && options.lastHad < 0n) {
		throw new RangeError(`lastHad is a line number, not ${options.lastHad}`);
	}
	const now = options.now ?? Date.now;
	const stop = new AbortController();
	options.signal?.addEventListener("abort", () => stop.abort(), { once: true });
	void connection.closed.then(() => stop.abort());
	const ours = personNamespace(options.room, options.name);
	const utf8 = new TextEncoder();

	const status: ChatStatus = {
		heard: false,
		following: false,
		lastEnd: undefined,
	};
	const changed = (change: Partial<ChatStatus>): void => {
		Object.assign(status, change);
		options.onStatus?.({ ...status });
	};

	/** A track of this participant: `said` or `reacted`, one Object a Group. */
	interface Ours {
		name: Uint8Array;
		/** The relay's subscription to it; one at a time, as the library refuses a second (§5.1). */
		listener: PublishedTrack | undefined;
		/**
		 * The Group of the next Object. WHY from the time in milliseconds: a
		 * page that comes again on the same name is a new publisher of the
		 * same track, and §2.4 has one that "intends to resume publishing on
		 * the same Track" "set the initial Group ID to a unique value
		 * guaranteed to be larger than all previously used groups ... based
		 * on the current time", as the demo's other tracks do.
		 */
		nextGroup: bigint;
		/** The Group of the last Object written, for the LARGEST_OBJECT of a later SUBSCRIBE_OK (§10.2.11). */
		lastGroup: bigint | undefined;
	}
	const ourTrack = (name: string): Ours => ({
		name: utf8.encode(name),
		listener: undefined,
		nextGroup: BigInt(now()),
		lastGroup: undefined,
	});
	const said = ourTrack(SAID_TRACK);
	const reacted = ourTrack(REACTED_TRACK);
	/** Lines and reactions being written: one stream each. */
	let writing = 0;

	const answer = (request: PeerSubscribe): void => {
		const track = [said, reacted].find((each) =>
			sameFullTrackName(request, {
				trackNamespace: ours,
				trackName: each.name,
			}),
		);
		if (track === undefined || stop.signal.aborted) {
			void request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
			return;
		}
		void request
			.accept({
				// §10.2.11: "If Objects have been published on this Track the
				// Publisher MUST include this parameter".
				parameters:
					track.lastGroup === undefined
						? {}
						: { largestObject: { group: track.lastGroup, object: 0n } },
			})
			.then((published) => {
				track.listener = published;
				// The gatherer is heard by its subscription to `said`.
				if (track === said) changed({ heard: true });
				void published.ended.then(() => {
					if (track.listener !== published) return;
					track.listener = undefined;
					if (track === said) changed({ heard: false });
				});
			})
			.catch(() => {});
	};
	connection.serve({ subscribe: answer });
	// Withdrawn when the participant stops (§6.2): the relay tells the
	// gatherer NAMESPACE_DONE, and the gatherer ends its SUBSCRIBE for
	// `said`, which ends `listener`.
	await connection.publishNamespace(
		ours,
		options.publishNamespaceToken === undefined
			? undefined
			: { authorizationToken: [options.publishNamespaceToken] },
		{ signal: stop.signal },
	);

	const remembered = Math.max(SEEN_LINES, catchUp);
	let seen = new SeenLines(remembered);
	/**
	 * The largest line number that came, a line or not: the next
	 * subscription's FETCH catches up from the one after it. WHY not only
	 * the lines handed on: an Object that is no line was had all the same,
	 * and fetching it again would bring it again.
	 */
	let had = options.lastHad;
	const take = (
		number: bigint,
		payload: Uint8Array,
		how: ReceivedLine["how"],
	): void => {
		if (had === undefined || number > had) had = number;
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
		// without lines has nothing to fetch (`historyToFetch`).
		const largest = subscription.parameters.largestObject?.group;
		// WHY decided here, before the first `await`: the live lines that
		// come next would move `had` past what this FETCH is to bring.
		const plan = historyToFetch(had, largest, catchUp);
		if (plan.forget) {
			seen = new SeenLines(remembered);
			had = undefined;
		}
		if (plan.fetch === undefined || largest === undefined) return;
		const { start, gap } = plan.fetch;
		if (gap !== undefined) options.onGap?.(gap);
		const most = largest - start + 1n;
		const fetch = await connection.fetch(
			{
				range: {
					// An Absolute Joining FETCH from the Group that
					// `historyToFetch` gives (§10.12.2.1), to the Joining
					// Location: why Absolute is at `historyStart`.
					fetchType: "AbsoluteJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: start,
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
		let objects = 0n;
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			if (next.value.type !== "object") continue;
			const { groupId, objectId, payload } = next.value.object;
			if (objectId !== 0n) continue;
			take(groupId, payload, "history");
			objects += 1n;
			// A relay or a gatherer that sends more than was asked for is not
			// read further.
			if (objects >= most) {
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
		const again = createAskAgain();
		while (!stop.signal.aborted) {
			let subscription: Subscription;
			try {
				subscription = await connection.subscribe({
					trackNamespace: roomNamespace(options.room),
					trackName: utf8.encode(LINES_TRACK),
					parameters: { rendezvousTimeout: BigInt(RENDEZVOUS_MS) },
					// A longer Object ends its stream before it is gathered,
					// and the subscription goes on.
					maxObjectPayloadLength: MAX_LINE_PAYLOAD,
				});
			} catch (error) {
				changed({ lastEnd: `refused: ${String(error).slice(0, 120)}` });
				await waitFor(
					again.afterRefusal(
						error instanceof RequestFailedError ? error.retryWaitMs : undefined,
					),
					stop.signal,
				);
				continue;
			}
			const cancel = (): void => {
				void subscription.cancel().catch(() => {});
			};
			// The signal can have aborted while the SUBSCRIBE waited for its
			// answer: the subscription made after it is cancelled at once.
			if (stop.signal.aborted) {
				cancel();
				return;
			}
			stop.signal.addEventListener("abort", cancel, { once: true });
			again.subscribed();
			changed({ following: true });
			// WHY a failed FETCH is no failure of the participant: the lines
			// that come live are there all the same; the history is missing.
			const fetched = history(subscription).catch(() => {});
			await live(subscription).catch(() => {});
			const end = await subscription.ended;
			await fetched;
			stop.signal.removeEventListener("abort", cancel);
			changed({ following: false, lastEnd: described(end) });
			await waitFor(again.afterEnd(), stop.signal);
		}
	};
	void follow();

	/**
	 * Follows a track of the room that is live alone (`reactions`,
	 * `present`): each Object 0 of a Group is handed to `take`, and where
	 * `latest`, the largest Group when the subscription is made, by a
	 * Relative Joining FETCH of none before it (§10.12.2), so that a
	 * participant that comes has the count at once. Subscribed again after
	 * it ends, as `lines` is.
	 */
	const followLive = async (
		trackName: string,
		latest: boolean,
		take: (payload: Uint8Array) => void,
	): Promise<void> => {
		const again = createAskAgain();
		while (!stop.signal.aborted) {
			let subscription: Subscription;
			try {
				subscription = await connection.subscribe({
					trackNamespace: roomNamespace(options.room),
					trackName: utf8.encode(trackName),
					parameters: { rendezvousTimeout: BigInt(RENDEZVOUS_MS) },
					maxObjectPayloadLength: MAX_LINE_PAYLOAD,
				});
			} catch (error) {
				await waitFor(
					again.afterRefusal(
						error instanceof RequestFailedError ? error.retryWaitMs : undefined,
					),
					stop.signal,
				);
				continue;
			}
			const cancel = (): void => {
				void subscription.cancel().catch(() => {});
			};
			if (stop.signal.aborted) {
				cancel();
				return;
			}
			stop.signal.addEventListener("abort", cancel, { once: true });
			again.subscribed();
			const fetched =
				latest && subscription.parameters.largestObject !== undefined
					? latestOf(subscription, take).catch(() => {})
					: undefined;
			const reader = subscription.objects.getReader();
			for (;;) {
				const next = await reader.read().catch(() => undefined);
				if (next === undefined || next.done) break;
				const { object } = next.value;
				if (object.status !== "Normal" || object.objectId !== 0n) continue;
				take(object.payload);
			}
			await subscription.ended;
			await fetched;
			stop.signal.removeEventListener("abort", cancel);
			await waitFor(again.afterEnd(), stop.signal);
		}
	};

	/** The Object 0 of the largest Group of `subscription`, by a Relative Joining FETCH of none before it. */
	const latestOf = async (
		subscription: Subscription,
		take: (payload: Uint8Array) => void,
	): Promise<void> => {
		const fetch = await connection.fetch(
			{
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: subscription.requestId,
					joiningStart: 0n,
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
		for (;;) {
			const next = await reader.read();
			if (next.done) return;
			if (next.value.type !== "object" || next.value.object.objectId !== 0n) {
				continue;
			}
			take(next.value.object.payload);
			// One Group was asked for: what comes after it is not read.
			await reader.cancel();
			return;
		}
	};

	const { onReactions, onPresent } = options;
	if (onReactions !== undefined) {
		void followLive(REACTIONS_TRACK, false, (payload) => {
			const read = readReactions(payload);
			if (read.ok) onReactions(read.value);
		});
	}
	if (onPresent !== undefined) {
		// WHY the newest alone: a FETCH and the live Objects can come in
		// either order, and a count older than the one shown is no news.
		let newest = -1;
		void followLive(PRESENT_TRACK, true, (payload) => {
			const read = readPresent(payload);
			if (!read.ok || read.value.at < newest) return;
			newest = read.value.at;
			onPresent(read.value);
		});
	}

	/** Writes `payload` as the Object 0 of the next Group of `track`, the whole of that Group. */
	const write = async (
		track: Ours,
		payload: () => Uint8Array,
	): Promise<SayOutcome> => {
		const published = track.listener;
		if (published === undefined)
			return { type: "not-sent", why: "nobody-subscribed" };
		if (writing >= MAX_WRITING) return { type: "not-sent", why: "busy" };
		const group = track.nextGroup;
		track.nextGroup += 1n;
		writing += 1;
		try {
			const stream = await published.openSubgroup({
				groupId: group,
				subgroupId: { mode: "Zero" },
				properties: false,
				// One line, or one reaction, is the whole of its Group.
				endOfGroup: true,
			});
			try {
				await stream.write({ objectId: 0n, payload: payload() });
				await stream.close();
			} catch (error) {
				await stream.reset().catch(() => {});
				throw error;
			}
			if (track.lastGroup === undefined || group > track.lastGroup) {
				track.lastGroup = group;
			}
			return { type: "sent", group };
		} catch (error) {
			return { type: "not-sent", why: "failed", error };
		} finally {
			writing -= 1;
		}
	};

	const lines = new RateWindow(limits.linesPerSecond);
	const reactions = new RateWindow(limits.reactionsPerSecond);
	return {
		status: () => ({ ...status }),
		async say(text) {
			const why = keepBack(text, limits, lines, now());
			if (why !== undefined) return { type: "kept-back", why };
			return write(said, () => saidPayload({ text, sentAt: now() }));
		},
		async react(emoji) {
			const why = keepBackReaction(emoji, reactions, now());
			if (why !== undefined) return { type: "kept-back", why };
			return write(reacted, () => reactedPayload({ emoji, sentAt: now() }));
		},
	};
}
