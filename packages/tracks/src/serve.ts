// The tracks of a namespace that one connection serves: each request of the
// peer goes to the track it is for, and one for none of them is refused.

import {
	type MoqtConnection,
	type PeerFetch,
	type PeerSubscribe,
	RequestErrorCode,
} from "@xmoq/moqt";

/** A track that answers the requests for it: a publisher of `@xmoq/media` or of `@xmoq/msf`. */
export interface ServedTrack {
	/** Whether the SUBSCRIBE was for this track, and is answered. */
	subscribe(request: PeerSubscribe): boolean;
	fetch(request: PeerFetch): boolean;
}

/**
 * Has `connection` answer the SUBSCRIBE and FETCH requests of its peer with
 * `tracks`: the first that takes a request answers it, and one that none
 * takes is refused with DOES_NOT_EXIST (§10.6.2). It names the `serve`
 * handlers of the connection.
 */
export function serveTracks(
	connection: Pick<MoqtConnection, "serve">,
	tracks: readonly ServedTrack[],
): void {
	connection.serve({
		subscribe(request) {
			if (tracks.some((track) => track.subscribe(request))) return;
			request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
		},
		fetch(request) {
			if (tracks.some((track) => track.fetch(request))) return;
			request.reject(RequestErrorCode.DOES_NOT_EXIST).catch(() => {});
		},
	});
}
