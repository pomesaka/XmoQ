// An actor of a room's garden over a connection of @xmoq/moqt, for a page:
// it says what it does to the garden on its own track `acted`, and reads
// the room's `garden` and the pet's `state`, its body. Section numbers are
// of draft-ietf-moq-transport-18.

import type { MoqtConnection, PublishedTrack } from "@xmoq/moqt";
import {
	type FollowStatus,
	followTrack,
	publishTrack,
	serveTracks,
} from "@xmoq/tracks";
import {
	ACTED,
	actorNamespace,
	GARDEN,
	PET_NAME,
	roomNamespace,
	STATE,
} from "../core/names.ts";
import {
	type Action,
	actedPayload,
	checkAction,
	type Garden,
	type Malformed,
	type Read,
	readGarden,
	readState,
	type State,
} from "../core/payloads.ts";

/**
 * The longest payload of a state read from the Joining FETCH or the
 * subscription to `state`: 64 KiB. **Provisional**. WHY: a state holds the
 * name of whom the pet reacts to, which a Full Track Name bounds to 4,096
 * bytes (§2.4.1), and JSON may write a byte of it as an escape of 6; the
 * rest of a state is a few hundred bytes at the most.
 */
export const MAX_STATE_PAYLOAD = 64 * 1024;

/**
 * The longest payload of a garden read from the Joining FETCH or the
 * subscription to `garden`: 1 MiB. **Provisional**. WHY: a garden holds up
 * to 8 things and 16 events, each with the name of an actor, which a Full
 * Track Name bounds to 4,096 bytes (§2.4.1) and JSON may write as escapes
 * of 6 a byte: 24 × 24 KiB is 576 KiB, and the texts and the rest are a
 * few KiB more.
 */
export const MAX_GARDEN_PAYLOAD = 1024 * 1024;

/**
 * How many Group numbers of `garden` or of `state` are remembered, so that
 * one is handed on once. **Provisional**. WHY few: they come live, and the
 * newest alone by the Joining FETCH, so a number comes twice only where
 * the FETCH and the subscription meet.
 */
const SEEN = 16;

/**
 * Actions being written on `acted` at once. One more is not sent. WHY a
 * bound: each action opens a stream, which waits for the relay to allow
 * it, and a page that acts faster than they go out would hold them all.
 * **Provisional**.
 */
const MAX_WRITING = 16;

/** What became of an action. */
export type ActOutcome =
	/** Written on `acted` in Group `group`, with `sentAt`; the garden may still pass it over. */
	| { type: "sent"; group: bigint; sentAt: number }
	/** Not sent: the member no reader takes (`checkAction`). */
	| { type: "kept-back"; why: "to" | "text" | "thing" }
	/**
	 * Not sent: nobody has subscribed to `acted` yet (the garden subscribes
	 * from the newest Object on, so an action before it would never reach
	 * it), too many actions are being written, or the writing failed.
	 */
	| {
			type: "not-sent";
			why: "nobody-subscribed" | "busy" | "failed";
			error?: unknown;
	  };

/** How far the actor is, for whoever shows it. */
export interface GardenStatus {
	/** Whether the garden, through the relay, has subscribed to `acted`. */
	heard: boolean;
	/** How the room's `garden` is followed. */
	garden: FollowStatus;
	/** How the pet's `state` is followed. */
	state: FollowStatus;
}

/** A track `joinGarden` follows. */
export type FollowedGardenTrack = typeof GARDEN | typeof STATE;

export interface JoinGardenOptions {
	/** The room's name: its namespace is `xmoq/pet/<room>`. */
	room: string;
	/** The page's name, not `PET_NAME`, which is the pet's. */
	name: string;
	/**
	 * An AUTHORIZATION TOKEN (§10.2.2, the bytes of the Token structure) for
	 * the PUBLISH_NAMESPACE of `xmoq/pet/<room>/<name>` alone. Left out, the
	 * PUBLISH_NAMESPACE carries none of its own.
	 *
	 * The token for reading the room goes on the connection instead
	 * (`authorizationToken` of `connect`). WHY a token of the request beside
	 * the connection's: the two name different namespaces and operations,
	 * and §10.2.2 has the parameter "authorize the sender to perform the
	 * operation carrying the parameter", so each goes where it is needed.
	 * WHY NOT this one on the SUBSCRIBE and the FETCH as well: it allows
	 * nothing there.
	 */
	publishNamespaceToken?: Uint8Array;
	/**
	 * Called with each garden of the room: the latest when the subscription
	 * is made, and each one after it. A garden older (by `at`) than the last
	 * given is not given.
	 */
	onGarden(garden: Garden): void;
	/**
	 * Called with each state of the pet, its body: the latest when the
	 * subscription is made, and each one after it. A state older (by `at`)
	 * than the last given is not given.
	 */
	onState(state: State): void;
	/** Called for an Object of `garden` or `state` that is none, with the track, its Group ID and why. */
	onMalformed?(
		track: FollowedGardenTrack,
		number: bigint,
		why: Malformed,
	): void;
	/** Called when `status()` changes. */
	onStatus?(status: GardenStatus): void;
	/**
	 * Stops the actor: the namespace is withdrawn, the subscriptions to
	 * `garden` and `state` are cancelled, and no SUBSCRIBE for `acted` is
	 * accepted after. Before the namespace is announced, `joinGarden` then
	 * rejects with `RequestAbortedError` of `cancelled`.
	 */
	signal?: AbortSignal;
	/** The wall clock, in milliseconds since the Unix epoch: `sentAt`. `Date.now` when left out. */
	now?: () => number;
}

export interface GardenActor {
	/** Does one thing to the garden. Never rejects. */
	act(action: Action): Promise<ActOutcome>;
	status(): GardenStatus;
}

/**
 * Joins the garden of `room` as the actor `name`: announces
 * `xmoq/pet/<room>/<name>` (PUBLISH_NAMESPACE), serves the relay's
 * SUBSCRIBE for `acted` on it, and follows the room's `garden` and the
 * pet's `state` under `xmoq/pet/<room>/pet` (`followTrack` of
 * `@xmoq/tracks`): the newest of each by a Joining FETCH on coming, so that
 * a page that comes shows the garden and the pet at once, and each one
 * after it, for as long as the session lasts.
 *
 * **It names the connection's `serve` handlers** (`serveTracks` of
 * `@xmoq/tracks`): a request for another track is refused with
 * DOES_NOT_EXIST, and a FETCH for `acted` with NOT_SUPPORTED. Give it a
 * connection of its own.
 *
 * Resolves once the namespace is announced; rejects as `publishNamespace`
 * does, and with a `RangeError`, before anything is sent, where `name` is
 * `PET_NAME`. WHY refused here: the garden does not take a page named so
 * for an actor (`fixtures/pet-tracks/README.md`), and the pet's namespace
 * announced by a page would stand in the pet's way at the relay.
 */
export async function joinGarden(
	connection: Pick<
		MoqtConnection,
		"serve" | "publishNamespace" | "subscribe" | "fetch" | "close" | "closed"
	>,
	options: JoinGardenOptions,
): Promise<GardenActor> {
	if (options.name === PET_NAME) {
		throw new RangeError(`the name ${PET_NAME} is the pet's`);
	}
	const now = options.now ?? Date.now;
	const stop = new AbortController();
	options.signal?.addEventListener("abort", () => stop.abort(), { once: true });
	void connection.closed.then(() => stop.abort());
	const ours = actorNamespace(options.room, options.name);
	const utf8 = new TextEncoder();

	const status: GardenStatus = {
		heard: false,
		garden: { following: false, lastEnd: undefined },
		state: { following: false, lastEnd: undefined },
	};
	const changed = (change: Partial<GardenStatus>): void => {
		Object.assign(status, change);
		options.onStatus?.({ ...status });
	};

	/** The relay's subscription to `acted`; one at a time (`publishTrack` of `one`). */
	let listener: PublishedTrack | undefined;
	/**
	 * The Group of the next action. WHY from the time in milliseconds: a
	 * page that comes again on the same name is a new publisher of the same
	 * track, and §2.4 has one that "intends to resume publishing on the same
	 * Track" "set the initial Group ID to a unique value guaranteed to be
	 * larger than all previously used groups ... based on the current time".
	 */
	let nextGroup = BigInt(now());
	/** The Group of the last action written, for the LARGEST_OBJECT of a later SUBSCRIBE_OK (§10.2.11). */
	let lastGroup: bigint | undefined;
	/** Actions being written: one stream each. */
	let writing = 0;

	const acted = publishTrack({
		trackNamespace: ours,
		trackName: utf8.encode(ACTED),
		subscribers: "one",
		// §10.2.11: "If Objects have been published on this Track the
		// Publisher MUST include this parameter".
		answer: () =>
			lastGroup === undefined
				? {}
				: { parameters: { largestObject: { group: lastGroup, object: 0n } } },
		subscribed(published) {
			listener = published;
			changed({ heard: true });
			void published.ended.then(() => {
				if (listener !== published) return;
				listener = undefined;
				changed({ heard: false });
			});
		},
	});
	// After the stop, a SUBSCRIBE for `acted` is refused with DOES_NOT_EXIST.
	stop.signal.addEventListener("abort", () => void acted.end(), {
		once: true,
	});
	if (stop.signal.aborted) void acted.end();
	serveTracks(connection, [acted]);
	// Withdrawn when the actor stops (§6.2): the relay tells the garden
	// NAMESPACE_DONE, and the garden ends its SUBSCRIBE for `acted`, which
	// ends `listener`.
	await connection.publishNamespace(
		ours,
		options.publishNamespaceToken === undefined
			? undefined
			: { authorizationToken: [options.publishNamespaceToken] },
		{ signal: stop.signal },
	);

	followNewest(connection, {
		track: GARDEN,
		trackNamespace: roomNamespace(options.room),
		maxPayloadLength: MAX_GARDEN_PAYLOAD,
		read: readGarden,
		give: (garden) => options.onGarden(garden),
		onMalformed: options.onMalformed,
		onStatus: (garden) => changed({ garden }),
		signal: stop.signal,
	});
	followNewest(connection, {
		track: STATE,
		trackNamespace: actorNamespace(options.room, PET_NAME),
		maxPayloadLength: MAX_STATE_PAYLOAD,
		read: readState,
		give: (state) => options.onState(state),
		onMalformed: options.onMalformed,
		onStatus: (state) => changed({ state }),
		signal: stop.signal,
	});

	return {
		status: () => ({ ...status }),
		async act(action) {
			const why = checkAction(action);
			if (why !== undefined) return { type: "kept-back", why };
			const published = listener;
			if (published === undefined) {
				return { type: "not-sent", why: "nobody-subscribed" };
			}
			if (writing >= MAX_WRITING) return { type: "not-sent", why: "busy" };
			const group = nextGroup;
			nextGroup += 1n;
			writing += 1;
			try {
				const stream = await published.openSubgroup({
					groupId: group,
					subgroupId: { mode: "Zero" },
					properties: false,
					// One action is the whole of its Group.
					endOfGroup: true,
				});
				const sentAt = now();
				try {
					await stream.write({
						objectId: 0n,
						payload: actedPayload({ ...action, sentAt }),
					});
					await stream.close();
				} catch (error) {
					await stream.reset().catch(() => {});
					throw error;
				}
				if (lastGroup === undefined || group > lastGroup) lastGroup = group;
				return { type: "sent", group, sentAt };
			} catch (error) {
				return { type: "not-sent", why: "failed", error };
			} finally {
				writing -= 1;
			}
		},
	};
}

/**
 * Follows a track of one value a Group, of which the newest alone counts:
 * the newest by a Joining FETCH on coming, and each one after it that is
 * not older (by `at`) than the last given.
 *
 * WHY no value older than the one given: a FETCH and the live Objects can
 * come in either order, and the garden is what its newest garden says, the
 * pet where its newest state says.
 */
function followNewest<T extends { at: number }>(
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "closed" | "close">,
	options: {
		track: FollowedGardenTrack;
		trackNamespace: Uint8Array[];
		maxPayloadLength: number;
		read(payload: Uint8Array): Read<T>;
		give(value: T): void;
		onMalformed: JoinGardenOptions["onMalformed"];
		onStatus(status: FollowStatus): void;
		signal: AbortSignal;
	},
): void {
	let newest = -1;
	followTrack(connection, {
		trackNamespace: options.trackNamespace,
		trackName: new TextEncoder().encode(options.track),
		recent: 1,
		catchUp: 1,
		remembered: SEEN,
		maxPayloadLength: options.maxPayloadLength,
		take(number, payload) {
			const read = options.read(payload);
			if (!read.ok) {
				options.onMalformed?.(options.track, number, read.why);
				return;
			}
			if (read.value.at < newest) return;
			newest = read.value.at;
			options.give(read.value);
		},
		onStatus: ({ following, lastEnd }) =>
			options.onStatus({ following, lastEnd }),
		signal: options.signal,
	});
}
