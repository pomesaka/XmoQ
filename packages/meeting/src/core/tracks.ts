// Which tracks of a participant's catalog are received.

/** What of a track of an MSF catalog this reads (`CatalogTrack` of `@xmoq/msf`). */
export interface OfferedTrack {
	readonly name: string;
	readonly namespace?: string;
	readonly role?: string;
	readonly width?: number;
	readonly height?: number;
}

/** The tracks of a participant that are received, where it offers them. */
export interface ChosenTracks<T extends OfferedTrack> {
	/** The video with the fewest pixels. */
	video?: T;
	/** The first sound. */
	audio?: T;
}

/**
 * The smallest video and the first sound of a catalog (decided on
 * 2026-10-25: the small version of the video, and the sound of everybody).
 * A video without a width or a height comes after every one with both.
 *
 * WHY only tracks in the catalog's own namespace (no `namespace`, MSF
 * §5.2.2): a participant publishes under `xmoq/meet/<room>/<id>`, which is
 * what its token allows; a track elsewhere would be somebody else's.
 */
export function chooseTracks<T extends OfferedTrack>(
	tracks: readonly T[],
): ChosenTracks<T> {
	const own = tracks.filter((track) => track.namespace === undefined);
	const pixels = (track: T): number =>
		track.width === undefined || track.height === undefined
			? Number.POSITIVE_INFINITY
			: track.width * track.height;
	let video: T | undefined;
	for (const track of own) {
		if (track.role !== "video") continue;
		if (video === undefined || pixels(track) < pixels(video)) video = track;
	}
	const audio = own.find((track) => track.role === "audio");
	return {
		...(video === undefined ? {} : { video }),
		...(audio === undefined ? {} : { audio }),
	};
}
