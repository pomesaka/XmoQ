// A participant of a meeting in a page: it learns who is there from one
// SUBSCRIBE_NAMESPACE for `xmoq/meet/<room>` (draft 18 §6.1), reads each
// one's catalog, and receives the sound of all of them and the small video
// of the first who came (decided on 2026-10-25, `docs/scale-and-meetings.md`).
// What it receives is handed to the caller, who plays it; publishing its
// own broadcast is the caller's as well (`@xmoq/media`, `@xmoq/msf`).

import {
	keepTrackOnSession,
	playAudioTrack,
	playVideoTrack,
} from "@xmoq/media";
import {
	createAskAgain,
	type MoqtConnection,
	type NamespaceSubscriptionEnd,
	RequestFailedError,
	waitFor,
} from "@xmoq/moqt";
import { type CatalogTrack, followCatalog } from "@xmoq/msf";
import { meetingPrefix, participantOf } from "../core/names.ts";
import {
	createPresence,
	type PresenceChange,
	SETTLE_MS,
	watchedOf,
} from "../core/presence.ts";
import { type ChosenTracks, chooseTracks } from "../core/tracks.ts";

/** Whose video is received at once: decided on 2026-10-25, the small version of 8 as they came. */
export const DEFAULT_MOST_WATCHED = 8;

/**
 * The others kept. Decided on 2026-10-25: the room counts the participants
 * and gives no token past 49, and a page keeps 48 others besides, so that a
 * relay that tells of more is not followed past them.
 */
export const DEFAULT_MOST_OTHERS = 48;

/**
 * How long the SUBSCRIBE for a participant's catalog asks the relay to wait
 * for it (RENDEZVOUS_TIMEOUT, §10.2.6): a participant announces its
 * namespace before its catalog is published. **Provisional**, as long as
 * the transcript's.
 */
const CATALOG_RENDEZVOUS_MS = 10_000;

/** What of a participant is received. */
export type ReceivedKind = "video" | "audio";

/** What became of the meeting, for the caller to show or log. */
export type MeetingEvent =
	| PresenceChange
	/** The track of `id` plays, on its `times`-th subscription of the session. */
	| {
			type: "receiving";
			id: string;
			kind: ReceivedKind;
			track: string;
			times: number;
	  }
	/** The meeting let go of the track of `id`: it left, lost its place among the watched, or its catalog changed. */
	| { type: "stopped"; id: string; kind: ReceivedKind }
	/** Something of `id` could not be received; the catalog is followed again, a track is not until its catalog changes. */
	| {
			type: "failed";
			id: string;
			what: ReceivedKind | "catalog";
			error: unknown;
	  };

export interface MeetingOptions {
	/** The meeting's room: its broadcasts are under `xmoq/meet/<room>`. */
	room: string;
	/** This participant's id, whose own broadcast is not received. */
	self: string;
	/** `DEFAULT_MOST_WATCHED` when left out; 0 receives no video. */
	mostWatched?: number;
	/** `DEFAULT_MOST_OTHERS` when left out. */
	mostOthers?: number;
	/** Each decoded frame of `id`; **the callee closes it**, as `PlayVideoTrackOptions.onFrame`. */
	onFrame(id: string, frame: VideoFrame): void;
	/** Each decoded piece of the sound of `id`; **the callee closes it**, as `PlayAudioTrackOptions.onData`. */
	onAudio(id: string, data: AudioData): void;
	onEvent?(event: MeetingEvent): void;
}

export interface Meeting {
	/**
	 * Takes part on one session until it ends or `signal` aborts, and
	 * resolves with how the SUBSCRIBE_NAMESPACE ended. Rejects as
	 * `MoqtConnection.subscribeNamespace` does where it is refused. Called
	 * again with the next session, it keeps who was there: those the relay
	 * does not tell of again within `SETTLE_MS` have left.
	 */
	attend(
		connection: MoqtConnection,
		signal?: AbortSignal,
	): Promise<NamespaceSubscriptionEnd>;
	/** The others there, in the order they came. */
	ids(): string[];
	/** Whose video is received, or would be once its catalog is read. */
	watched(): string[];
}

const utf8 = new TextEncoder();
const fromUtf8 = new TextDecoder();

/** A participant there on this session. */
interface Attending {
	/** Aborted when it leaves or the session ends. */
	stop: AbortController;
	tracks: ChosenTracks<CatalogTrack>;
	video?: AbortController;
	audio?: AbortController;
}

function wholeFrom(name: string, value: number, least: number): number {
	if (!Number.isInteger(value) || value < least) {
		throw new RangeError(
			`${name} is a whole number from ${least}, not ${value}`,
		);
	}
	return value;
}

/**
 * A meeting nobody is in yet. Throws a `RangeError` for a `mostWatched` that
 * is no whole number from 0, or a `mostOthers` from 1.
 */
export function createMeeting(options: MeetingOptions): Meeting {
	const mostWatched = wholeFrom(
		"mostWatched",
		options.mostWatched ?? DEFAULT_MOST_WATCHED,
		0,
	);
	const mostOthers = wholeFrom(
		"mostOthers",
		options.mostOthers ?? DEFAULT_MOST_OTHERS,
		1,
	);
	const presence = createPresence(mostOthers);
	const tell = (event: MeetingEvent): void => options.onEvent?.(event);

	const attend = async (
		connection: MoqtConnection,
		signal: AbortSignal = new AbortController().signal,
	): Promise<NamespaceSubscriptionEnd> => {
		const prefix = meetingPrefix(options.room);
		const subscription = await connection.subscribeNamespace(
			{ trackNamespacePrefix: prefix.map((field) => utf8.encode(field)) },
			{ signal },
		);
		const attending = new Map<string, Attending>();

		const receive = (
			id: string,
			kind: ReceivedKind,
			participant: Attending,
			track: CatalogTrack,
		): AbortController => {
			const over = new AbortController();
			const playing = AbortSignal.any([participant.stop.signal, over.signal]);
			const trackNamespace = [...prefix, id].map((field) => utf8.encode(field));
			const trackName = utf8.encode(track.name);
			const codec = track.codec;
			void keepTrackOnSession({
				connection,
				async play() {
					if (codec === undefined) {
						throw new Error(`the catalog has no codec for ${track.name}`);
					}
					if (kind === "video") {
						const playback = await playVideoTrack(connection, {
							trackNamespace,
							trackName,
							codec,
							onFrame: (frame) => options.onFrame(id, frame),
							signal: playing,
						});
						return { ended: playback.ended.then((end) => end.type) };
					}
					const playback = await playAudioTrack(connection, {
						trackNamespace,
						trackName,
						codec,
						sampleRate: track.samplerate ?? 48_000,
						numberOfChannels: Number(track.channelConfig ?? 1),
						onData: (data) => options.onAudio(id, data),
						signal: playing,
					});
					return { ended: playback.ended.then((end) => end.type) };
				},
				signal: playing,
				onState(state) {
					if (state.type === "playing") {
						tell({
							type: "receiving",
							id,
							kind,
							track: track.name,
							times: state.times,
						});
					} else if (state.type === "failed" && !playing.aborted) {
						tell({ type: "failed", id, what: kind, error: state.error });
					}
				},
			});
			return over;
		};

		/** Receives what each one there should have, and lets go of what not. */
		const sync = (): void => {
			const watched = watchedOf(presence.ids(), mostWatched);
			for (const [id, participant] of attending) {
				const { video, audio } = participant.tracks;
				if (audio !== undefined && participant.audio === undefined) {
					participant.audio = receive(id, "audio", participant, audio);
				}
				const wanted = watched.includes(id);
				if (wanted && video !== undefined && participant.video === undefined) {
					participant.video = receive(id, "video", participant, video);
				} else if (!wanted && participant.video !== undefined) {
					participant.video.abort();
					participant.video = undefined;
					tell({ type: "stopped", id, kind: "video" });
				}
			}
		};

		/** Lets go of the tracks of `participant`, for a catalog that changed. */
		const letGo = (id: string, participant: Attending): void => {
			for (const kind of ["video", "audio"] as const) {
				const playing = participant[kind];
				if (playing === undefined) continue;
				playing.abort();
				participant[kind] = undefined;
				tell({ type: "stopped", id, kind });
			}
		};

		const follow = async (
			id: string,
			participant: Attending,
		): Promise<void> => {
			const { signal: stopped } = participant.stop;
			const namespace = [...prefix, id].map((field) => utf8.encode(field));
			// The pause before the catalog is followed again: the rule every
			// client asks by (`createAskAgain` of @xmoq/moqt).
			const again = createAskAgain();
			while (!stopped.aborted) {
				let pauseMs: number;
				try {
					const follower = await followCatalog(connection, {
						namespace,
						signal: stopped,
						rendezvousTimeoutMs: CATALOG_RENDEZVOUS_MS,
					});
					again.subscribed();
					const states = follower.states.getReader();
					for (;;) {
						const next = await states.read();
						if (next.done) break;
						const catalog = next.value.current?.catalog;
						if (catalog === undefined) continue;
						const chosen = chooseTracks(catalog.tracks);
						const same = (kind: ReceivedKind): boolean =>
							chosen[kind]?.name === participant.tracks[kind]?.name &&
							chosen[kind]?.codec === participant.tracks[kind]?.codec;
						if (same("video") && same("audio")) continue;
						letGo(id, participant);
						participant.tracks = chosen;
						sync();
					}
					pauseMs = again.afterEnd();
				} catch (error) {
					if (!stopped.aborted)
						tell({ type: "failed", id, what: "catalog", error });
					pauseMs = again.afterRefusal(
						error instanceof RequestFailedError ? error.retryWaitMs : undefined,
					);
				}
				await waitFor(pauseMs, stopped);
			}
		};

		const begin = (id: string): void => {
			const participant: Attending = {
				stop: new AbortController(),
				tracks: {},
			};
			attending.set(id, participant);
			void follow(id, participant);
		};
		const end = (id: string): void => {
			const participant = attending.get(id);
			if (participant === undefined) return;
			attending.delete(id);
			participant.stop.abort();
			for (const kind of ["video", "audio"] as const) {
				if (participant[kind] !== undefined)
					tell({ type: "stopped", id, kind });
			}
		};
		const changed = (change: PresenceChange | undefined): void => {
			if (change === undefined) return;
			if (change.type === "came") begin(change.id);
			if (change.type === "left") end(change.id);
			tell(change);
		};

		// Those kept from the session before are followed on this one at once.
		for (const id of presence.ids()) begin(id);
		const settle = setTimeout(() => {
			for (const change of presence.settled()) changed(change);
			sync();
		}, SETTLE_MS);
		try {
			const changes = subscription.changes.getReader();
			for (;;) {
				const next = await changes.read();
				if (next.done) break;
				const fields = next.value.trackNamespace.map((field) =>
					fromUtf8.decode(field),
				);
				const id = participantOf(options.room, fields);
				if (id === undefined || id === options.self) continue;
				changed(next.value.done ? presence.done(id) : presence.named(id));
				sync();
			}
		} catch {
			// The request ended; `ended` says how.
		} finally {
			clearTimeout(settle);
			for (const participant of attending.values()) participant.stop.abort();
			presence.sessionEnded();
		}
		return subscription.ended;
	};

	return {
		attend,
		ids: () => presence.ids(),
		watched: () => watchedOf(presence.ids(), mostWatched),
	};
}
