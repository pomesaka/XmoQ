// A participant of a room of the chat over a connection of @xmoq/moqt:
// it says its lines on its own track `said` and its reactions on `reacted`,
// and reads the room's `lines`, and where it is asked to, its `reactions`
// and `present`, which the gatherer (`crates/chat`) publishes. Section numbers are of
// draft-ietf-moq-transport-18.

import type { MoqtConnection, PublishedTrack } from "@xmoq/moqt";
import {
	type FollowGap,
	followTrack,
	type PublishingTrack,
	publishTrack,
	serveTracks,
} from "@xmoq/tracks";
import { catchUpLines, HISTORY_LINES } from "../core/history.ts";
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

/**
 * How many line numbers are remembered, so that a line is handed on once
 * (`SeenNumbers` of `@xmoq/tracks`), where `catchUpLines` is no more. WHY 200: what the demo's
 * page keeps of the chat. **Provisional**. WHY as many as `catchUpLines`
 * where that is more: the lines of a FETCH that catches up come with older
 * numbers than live lines that came meanwhile, and a number below all those
 * remembered is passed over as had.
 */
export const SEEN_LINES = 200;

/**
 * The longest payload of a line read from the Joining FETCH or the
 * subscription to `lines`: 64 KiB, what
 * the gatherer reads of one stream of `said` (`PAYLOAD_MOST`), which holds
 * the longest line it takes, 8,192 bytes of text, however it is escaped.
 */
export const MAX_LINE_PAYLOAD = 64 * 1024;

/**
 * How many Group numbers of `reactions` and `present` are remembered, so
 * that an Object is handed on once. **Provisional**. WHY few: their Objects
 * come live, and of `present` the newest alone by the Joining FETCH, so a
 * number comes twice only where the FETCH and the subscription meet.
 */
const SEEN_LIVE = 16;

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
	/**
	 * Written on `said` in Group `group`, with `sentAt`; the gatherer may
	 * still drop it (its rate is counted where the line arrives). A line
	 * that comes back from the gatherer has the same `sentAt`.
	 */
	| { type: "sent"; group: bigint; sentAt: number }
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
	onGap?(gap: FollowGap): void;
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

/**
 * Joins the room `room` as `name`: announces `xmoq/chat/<kind>/<room>/<name>`
 * (PUBLISH_NAMESPACE), serves the relay's SUBSCRIBE for `said` on it, and
 * follows the room's `lines` (`followTrack` of `@xmoq/tracks`): the newest
 * `HISTORY_LINES` on coming, and the lines after the last one had each time
 * its subscription is made again, for as long as the session lasts. WHY a
 * RENDEZVOUS_TIMEOUT on the SUBSCRIBE: the gatherer publishes a room only
 * once it follows somebody in it (`crates/chat/src/bin/xmoq-chat/session.rs`), so the first
 * participant's SUBSCRIBE would otherwise come before the room's PUBLISH
 * and be refused.
 *
 * **It names the connection's `serve` handlers** (`serveTracks` of
 * `@xmoq/tracks`): a request for another track is refused with
 * DOES_NOT_EXIST, and a FETCH for `said` or `reacted` with NOT_SUPPORTED.
 * Give it a connection of its own.
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
		/** The relay's subscription to it; one at a time (`publishTrack` of `one`). */
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

	/** `track` as the relay asks for it: the one subscription of the gatherer. */
	const publishing = (track: Ours): PublishingTrack =>
		publishTrack({
			trackNamespace: ours,
			trackName: track.name,
			subscribers: "one",
			// §10.2.11: "If Objects have been published on this Track the
			// Publisher MUST include this parameter".
			answer: () =>
				track.lastGroup === undefined
					? {}
					: {
							parameters: {
								largestObject: { group: track.lastGroup, object: 0n },
							},
						},
			subscribed(published) {
				track.listener = published;
				// The gatherer is heard by its subscription to `said`.
				if (track === said) changed({ heard: true });
				void published.ended.then(() => {
					if (track.listener !== published) return;
					track.listener = undefined;
					if (track === said) changed({ heard: false });
				});
			},
		});
	const served = [publishing(said), publishing(reacted)];
	// After the stop, a SUBSCRIBE for either is refused with DOES_NOT_EXIST.
	stop.signal.addEventListener(
		"abort",
		() => {
			for (const track of served) void track.end();
		},
		{ once: true },
	);
	if (stop.signal.aborted) for (const track of served) void track.end();
	serveTracks(connection, served);
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

	followTrack(connection, {
		trackNamespace: roomNamespace(options.room),
		trackName: utf8.encode(LINES_TRACK),
		recent: HISTORY_LINES,
		catchUp,
		...(options.lastHad === undefined ? {} : { lastHad: options.lastHad }),
		remembered: Math.max(SEEN_LINES, catchUp),
		maxPayloadLength: MAX_LINE_PAYLOAD,
		take(number, payload, how) {
			const read = readLine(number, payload);
			if (!read.ok) {
				options.onMalformed?.(number, read.why);
				return;
			}
			options.onLine({ ...read.value, how });
		},
		...(options.onGap === undefined ? {} : { onGap: options.onGap }),
		onStatus: ({ following, lastEnd }) => changed({ following, lastEnd }),
		signal: stop.signal,
	});

	/**
	 * Follows a track of the room whose Objects matter as they come
	 * (`reactions`, `present`): nothing of what came before, or with
	 * `latest`, the newest alone, so that a participant that comes has the
	 * count at once.
	 */
	const followLive = (
		trackName: string,
		latest: boolean,
		take: (payload: Uint8Array) => void,
	): void => {
		const recent = latest ? 1 : 0;
		followTrack(connection, {
			trackNamespace: roomNamespace(options.room),
			trackName: utf8.encode(trackName),
			recent,
			catchUp: recent,
			remembered: SEEN_LIVE,
			maxPayloadLength: MAX_LINE_PAYLOAD,
			take: (_number, payload) => take(payload),
			signal: stop.signal,
		});
	};

	const { onReactions, onPresent } = options;
	if (onReactions !== undefined) {
		followLive(REACTIONS_TRACK, false, (payload) => {
			const read = readReactions(payload);
			if (read.ok) onReactions(read.value);
		});
	}
	if (onPresent !== undefined) {
		// WHY the newest alone: a FETCH and the live Objects can come in
		// either order, and a count older than the one shown is no news.
		let newest = -1;
		followLive(PRESENT_TRACK, true, (payload) => {
			const read = readPresent(payload);
			if (!read.ok || read.value.at < newest) return;
			newest = read.value.at;
			onPresent(read.value);
		});
	}

	/** Writes `payload` of the clock now as the Object 0 of the next Group of `track`, the whole of that Group. */
	const write = async (
		track: Ours,
		payload: (sentAt: number) => Uint8Array,
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
			const sentAt = now();
			try {
				await stream.write({ objectId: 0n, payload: payload(sentAt) });
				await stream.close();
			} catch (error) {
				await stream.reset().catch(() => {});
				throw error;
			}
			if (track.lastGroup === undefined || group > track.lastGroup) {
				track.lastGroup = group;
			}
			return { type: "sent", group, sentAt };
		} catch (error) {
			return { type: "not-sent", why: "failed", error };
		} finally {
			writing -= 1;
		}
	};

	const lineRate = new RateWindow(limits.linesPerSecond);
	const reactionRate = new RateWindow(limits.reactionsPerSecond);
	return {
		status: () => ({ ...status }),
		async say(text) {
			const why = keepBack(text, limits, lineRate, now());
			if (why !== undefined) return { type: "kept-back", why };
			return write(said, (sentAt) => saidPayload({ text, sentAt }));
		},
		async react(emoji) {
			const why = keepBackReaction(emoji, reactionRate, now());
			if (why !== undefined) return { type: "kept-back", why };
			return write(reacted, (sentAt) => reactedPayload({ emoji, sentAt }));
		},
	};
}
