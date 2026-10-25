// Who is there, by the names the relay tells of with NAMESPACE and
// NAMESPACE_DONE to a SUBSCRIBE_NAMESPACE (draft 18 §6.1), across the
// sessions of a participant: on a new session the relay tells of everybody
// again, and those it does not tell of are taken to have left.
// `Presence` of `crates/viewer/src/presence.rs` keeps the same, and
// `fixtures/meeting-presence/who-is-there.tsv` pins both.

/**
 * How long after the SUBSCRIBE_NAMESPACE of a new session was answered those
 * not told of again are taken to have left. **Provisional**, as `SETTLE_MS`
 * of `crates/viewer/src/presence.rs`.
 *
 * WHY: the relay tells of every namespace under the prefix when it is asked
 * (§6.1), and nobody is told to have left while the participant was away.
 * WHY a wait and not the answer: draft 18 has no message that says the
 * namespaces there were are all told.
 */
export const SETTLE_MS = 2_000;

/** Somebody came or left. */
export type PresenceChange =
	/** Told of, and not there before. */
	| { type: "came"; id: string }
	/** Told to be gone, or not told of again on a new session. */
	| { type: "left"; id: string }
	/** Told of while as many are there as are kept: left out. */
	| { type: "too-many"; id: string };

export interface Presence {
	/** Who is there, in the order they came. */
	ids(): string[];
	has(id: string): boolean;
	/**
	 * The relay told of `id` (NAMESPACE). WHY one more than is kept is told
	 * of and not kept: the relay says who announces, and how many is theirs
	 * to choose; the participant keeps what it can, and says what it left out.
	 */
	named(id: string): PresenceChange | undefined;
	/** The relay told that `id` is gone (NAMESPACE_DONE). */
	done(id: string): PresenceChange | undefined;
	/** The session ended: whoever is told of on the next one is still there, and nobody comes or leaves for it. */
	sessionEnded(): void;
	/** `SETTLE_MS` after the SUBSCRIBE_NAMESPACE of a session was answered: those not told of on it have left. */
	settled(): PresenceChange[];
}

/**
 * Nobody yet, with room for `most` (taken for 1 where it is less).
 *
 * WHY in the order they came, where the Rust one keeps them by name: the
 * videos of a meeting go to those who came first (`watchedOf`), and the
 * reader for agents has no such choice.
 */
export function createPresence(most: number): Presence {
	const room = Math.max(1, most);
	/** Each one there, and whether the relay told of it on this session; a Map keeps the order they came in. */
	const here = new Map<string, boolean>();
	return {
		ids: () => [...here.keys()],
		has: (id) => here.has(id),
		named(id) {
			if (here.has(id)) {
				here.set(id, true);
				return undefined;
			}
			if (here.size >= room) return { type: "too-many", id };
			here.set(id, true);
			return { type: "came", id };
		},
		done(id) {
			return here.delete(id) ? { type: "left", id } : undefined;
		},
		sessionEnded() {
			for (const id of here.keys()) here.set(id, false);
		},
		settled() {
			const gone = [...here].filter(([, told]) => !told).map(([id]) => id);
			for (const id of gone) here.delete(id);
			return gone.map((id) => ({ type: "left", id }));
		},
	};
}

/**
 * Whose video is received: the first `most` of those there, in the order
 * they came (decided on 2026-10-25: the small version of up to 8, as they
 * came). One who leaves gives the place to the next who came.
 */
export function watchedOf(ids: readonly string[], most: number): string[] {
	return ids.slice(0, Math.max(0, most));
}
