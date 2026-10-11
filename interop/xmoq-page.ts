// Contract between the driver (`run.ts`) and every browser page it drives.
// Each page assigns an implementation to `window.xmoq`, so the driver can swap
// one client for another without knowing which library is behind it.

// A type alone, of the part of the package that needs no browser.
import type { Catalog } from "@xmoq/msf/catalog";

/**
 * One Object Property (draft 18 §11.2.1.2), in strings that survive
 * `page.evaluate`. `type` is decimal. An even type has a variable-length
 * integer as its value, in decimal; an odd type has bytes, in hexadecimal
 * (§1.4.3).
 */
export type PageProperty =
	| { type: string; varint: string }
	| { type: string; bytesHex: string };

/** The first Object a page received, in strings that survive `page.evaluate`. */
export type PageReceivedObject = {
	group: string;
	object: string;
	payloadHex: string;
	/** Absent on the moqtail page, which does not report them. */
	properties?: PageProperty[];
	/** The Subgroup ID of the stream it came on. Absent on the moqtail page. */
	subgroup?: string;
	/** The Publisher Priority of its Subgroup Header, or null where that left it out. Absent on the moqtail page. */
	publisherPriority?: number | null;
};

export type XmoqPage = {
	/**
	 * `certSha256Hex` is null when the browser trusts the relay by other means.
	 * The driver always passes a hash; null and `disconnect` are part of the
	 * contract because the moqtail page, copied as it was, has them.
	 */
	connect(
		relayUrl: string,
		certSha256Hex: string | null,
		/** Own page: the longest payload of an Object it takes (`limits.maxObjectPayloadLength` of `@xmoq/moqt`); the library's when left out. The moqtail page has no such limit to set. */
		maxObjectPayloadLength?: number,
	): Promise<void>;
	publishTrack(namespace: string, trackName: string): Promise<string>;
	sendObject(payloadHex: string): Promise<void> | void;
	subscribe(namespace: string, trackName: string): Promise<string>;
	firstObject(): Promise<PageReceivedObject>;
	disconnect(): Promise<void>;
};

/**
 * What only the own page offers. The moqtail page is a copy that is kept as
 * it came, so these are not part of `XmoqPage`.
 */
export type OwnPageExtras = {
	/**
	 * Sends PUBLISH_NAMESPACE and no PUBLISH. A page that does not say how it
	 * serves the requests of the peer refuses them with REQUEST_ERROR
	 * NOT_SUPPORTED, so this page refuses the SUBSCRIBE a relay sends it for
	 * a track.
	 */
	announceNamespace(namespace: string): Promise<void>;
	/**
	 * Sends PUBLISH_NAMESPACE and no PUBLISH, and accepts the first SUBSCRIBE
	 * the peer sends for the track (`MoqtConnection.serve`); one for another
	 * track is refused with DOES_NOT_EXIST. The page then sends on that
	 * subscription as it does on one made with `publishTrack`.
	 */
	serveTrack(
		namespace: string,
		trackName: string,
		fetches: PageFetchServing,
		/** With "serve-unsent": what the three Objects the page never sent carry. */
		unsentPayloadHex: string,
	): Promise<void>;
	/** How each FETCH the page accepted ended: the library's `ServedFetchEnd` as JSON, in the order they ended. */
	servedFetchEnds(): Promise<string[]>;
	/** Resolves with the Track Alias once the SUBSCRIBE was accepted. To be called after `serveTrack`. */
	servingTrack(): Promise<string>;
	/** Ends the published Track with PUBLISH_DONE, Status Code TRACK_ENDED. */
	endTrack(): Promise<void>;
	/**
	 * Sends one Object as Object 0 of the Group, on a subgroup stream of its
	 * own. With `finish` false the stream is left open, as it is while more
	 * Objects of the Subgroup are to come.
	 */
	sendObjectIn(
		group: string,
		payloadHex: string,
		finish: boolean,
		properties: PageProperty[],
		/** The Publisher Priority in the Subgroup Header; null leaves it out (§11.4.2). */
		publisherPriority: number | null,
	): Promise<void>;
	/**
	 * Sends a Group of `objects` Objects, Object 0 on, each with `bytes`
	 * bytes of payload, on one subgroup stream that is finished after them
	 * and marked as the end of its Group. Resolves when the stream is
	 * finished.
	 */
	sendLargeGroup(group: string, objects: number, bytes: number): Promise<void>;
	/** How many Objects of the Group the subscription of this page has delivered so far. */
	objectsOfGroup(group: string): Promise<number>;
	/** The Objects of the Group the subscription of this page has delivered so far, as JSON of `PageReceivedObject[]`, in the order they came. */
	receivedOfGroup(group: string): Promise<string>;
	/**
	 * Writes one Object to the subgroup stream of the Group and Subgroup,
	 * which is opened with the Publisher Priority at its first Object and
	 * left open. Every Object of a stream is to have properties, or none.
	 */
	writeToSubgroup(
		group: string,
		subgroup: string,
		publisherPriority: number,
		object: string,
		payloadHex: string,
		properties: PageProperty[],
	): Promise<void>;
	/** Finishes the streams `writeToSubgroup` opened for the Group. */
	closeSubgroups(group: string): Promise<void>;
	/**
	 * Sends a FETCH, waits for its FETCH_OK, **reads nothing of the fetch
	 * stream**, and gives the FETCH up after `cancelAfterMs`. Resolves with
	 * the End Location of the FETCH_OK as [group, object] in decimal and with
	 * how the FETCH ended; rejects when the FETCH is refused.
	 */
	fetchAndLeaveUnread(
		fetch: PageFetch,
		cancelAfterMs: number,
	): Promise<{ endLocation: [string, string]; ended: string }>;
	/**
	 * Sends one Object like `sendObjectIn` with `finish`, on a stream the
	 * subscriber may stop before the Object is written. Resolves with "sent",
	 * or with "stopped: " and the error of the write; rejects when the write
	 * fails for another reason.
	 */
	sendObjectInWhatMayBeStopped(
		group: string,
		payloadHex: string,
		publisherPriority: number | null,
	): Promise<string>;
	/**
	 * Sends one Object like `sendObjectIn` with `finish`, and closes the
	 * session. With `publishDone` the Track is ended with PUBLISH_DONE first,
	 * and the session is closed when `done()` has settled, as an application
	 * of the library does; without, nothing is waited for in between.
	 */
	sendLastObjectAndClose(
		group: string,
		payloadHex: string,
		publishDone: boolean,
	): Promise<void>;
	/** How the subscription made with PUBLISH ended, as JSON of the library's `PublishEnd`. */
	publishEnd(): Promise<string>;
	/**
	 * What `done()` resolved with, as JSON of the library's `PeerStreamEnd`:
	 * how the subscriber ended its direction of the request stream after
	 * PUBLISH_DONE. "null" when no PUBLISH_DONE was sent.
	 */
	peerStreamEnd(): Promise<string>;
	/**
	 * Subscribes like `subscribe` and never reads the Objects, as an
	 * application that has stopped taking them.
	 */
	subscribeWithoutReading(
		namespace: string,
		trackName: string,
	): Promise<string>;
	/** The payload of Object 0 of the Group, once it has arrived. */
	objectOfGroup(group: string): Promise<string>;
	/**
	 * The payloads of every Object of the Group that has arrived so far, as a
	 * JSON array of hexadecimal strings: more than one where the Group came
	 * twice.
	 */
	payloadsOfGroup(group: string): Promise<string>;
	/**
	 * The Object Properties of Object 0 of the Group once it arrived, in the
	 * order they were on the wire, as a JSON array of `PageProperty`.
	 */
	propertiesOfGroup(group: string): Promise<string>;
	/**
	 * For the message of a run that failed: JSON of how the session ended, or
	 * null while it is open, how many Objects were read, and how reading ended.
	 */
	state(): Promise<string>;
	/**
	 * Starts reading the catalog track of the namespace with `followCatalog`
	 * of `@xmoq/msf`: a SUBSCRIBE and a Joining FETCH. Resolves when the
	 * SUBSCRIBE was accepted.
	 */
	followCatalog(namespace: string): Promise<void>;
	/**
	 * What the reading started with `followCatalog` has given so far.
	 * Resolves once it has `states` states at least and the Joining FETCH is
	 * over.
	 */
	catalogReport(states: number): Promise<PageCatalogReport>;
	/**
	 * Announces the namespace and publishes its catalog track with
	 * `createCatalogPublisher` of `@xmoq/msf`: the page answers the SUBSCRIBE
	 * and the FETCH of the peer for that track, and refuses every other
	 * request with DOES_NOT_EXIST. Resolves with the Location of the catalog,
	 * as [group, object] in decimal.
	 */
	publishCatalog(
		namespace: string,
		catalog: Catalog,
	): Promise<[string, string]>;
	/** Publishes a catalog that replaces the one before; resolves with its Location. */
	updateCatalog(catalog: Catalog): Promise<[string, string]>;
	/**
	 * Publishes a last catalog and ends the catalog track right after, with
	 * nothing awaited between. Resolves with the Location of that catalog
	 * once the subscriptions were told of the end.
	 */
	endCatalog(catalog: Catalog): Promise<[string, string]>;
	/**
	 * Resolves once the subscription started with `followCatalog` has ended:
	 * "publish-done <status code>", or the type of another end.
	 */
	catalogEnded(): Promise<string>;
	/**
	 * Starts playing the video track with `playVideoTrack` of `@xmoq/media`:
	 * a SUBSCRIBE, and a decoder of WebCodecs configured with `codec`. The
	 * frames are not drawn: their time and size are noted, and each is
	 * closed. Resolves when the SUBSCRIBE was accepted.
	 */
	playVideo(
		namespace: string,
		trackName: string,
		codec: string,
		/** Reads the number `publishVideo` paints into every frame. */
		readNumbers?: boolean,
	): Promise<void>;
	/**
	 * Publishes a catalog with one video track and the track itself, with
	 * `createCatalogPublisher` of `@xmoq/msf` and `createVideoTrackEncoder`
	 * of `@xmoq/media`, and announces the namespace. From then on the page
	 * paints a frame every `1000 / framerate` milliseconds and encodes it:
	 * frame `n` has the time `firstTimestamp + n * 1000000 / framerate`
	 * microseconds and shows `n` modulo 256 as 8 bars from left to right,
	 * the highest bit first, a white bar for a 1 and a black one for a 0.
	 * Resolves when the namespace is announced.
	 */
	publishVideo(namespace: string, setup: PageVideoSetup): Promise<void>;
	/** What the encoder and the publisher started with `publishVideo` have counted so far. */
	videoPublishReport(): Promise<PageVideoPublishReport>;
	/** Stops painting and ends the video track (`VideoTrackEncoder.end`). */
	stopVideo(): Promise<void>;
	/**
	 * Whether the browser has a decoder for the codec, for Annex B without a
	 * description: "supported", "not supported", or "no VideoDecoder".
	 */
	videoDecoderSupport(codec: string): Promise<string>;
	/**
	 * What the playing started with `playVideo` has given so far. Resolves
	 * once `frames` frames at least were decoded, the playing has ended, or
	 * `timeoutMs` are over, whichever comes first: a run that gets too few
	 * frames still has a report to show.
	 */
	videoReport(frames: number, timeoutMs: number): Promise<PageVideoReport>;
	/** Cancels the subscription and keeps the session. */
	cancelSubscription(): Promise<void>;
	/** Resolves once the subscription has ended and its Objects were read; JSON of `SubscriptionReport`. */
	subscriptionReport(): Promise<string>;
	/**
	 * Sends a FETCH and resolves once it is over, with JSON of
	 * `PageFetchReport`. The FETCH is cancelled when it is not over after
	 * `timeoutMs`. A Joining FETCH joins the page's subscription.
	 */
	fetchRange(fetch: PageFetch, timeoutMs: number): Promise<string>;
};

/**
 * What a page that serves a track does with a Standalone FETCH for it:
 * "serve", it answers from the Objects it has sent; "serve-unsent", the
 * same, and it has Object 0 of Groups 100, 101 and 102 as well, which it
 * never sent, so that no relay has them, and the driver has it send from
 * Group 103 on; "stall", it accepts and writes
 * nothing; "refuse", DOES_NOT_EXIST.
 */
export type PageFetchServing = "serve" | "serve-unsent" | "stall" | "refuse";

/** A FETCH for an own page to send (draft 18 §10.12). Numbers are decimal strings. */
export type PageFetch =
	| {
			type: "Standalone";
			namespace: string;
			trackName: string;
			/** Group and Object of the Start Location. */
			start: [string, string];
			/** Group and Object of the End Location: the last Object plus 1, or Object 0 for a whole Group. */
			end: [string, string];
			/** FILL_TIMEOUT in milliseconds (§10.2.5); left out of the FETCH when absent. */
			fillTimeout?: string;
	  }
	| { type: "RelativeJoining" | "AbsoluteJoining"; joiningStart: string };

/** What an own page saw of one FETCH. Numbers are decimal strings. */
export type PageFetchReport = {
	/** The FETCH_OK, or what `response` rejected with: a REQUEST_ERROR has its code. */
	response:
		| { endOfTrack: boolean; endLocation: [string, string] }
		| { error: string; errorCode?: string };
	/** What the fetch stream decoded to, in order. */
	items: (
		| {
				type: "object";
				group: string;
				object: string;
				payloadHex: string;
				/** Null for an Object without a Subgroup ID (§11.4.4.1). */
				subgroup: string | null;
				publisherPriority: number;
				properties: PageProperty[];
		  }
		| { type: "end-of-range"; kind: string; group: string; object: string }
	)[];
	/** How reading the fetch stream ended: "closed", or the error it failed with. */
	bodyEnd: string;
	/** The library's `FetchEnd`, with its `type` first. */
	end: { type: string; code?: number; error?: { errorCode: string } };
};

/** What an own page saw of one subscription. Numbers are decimal strings. */
export type SubscriptionReport = {
	/** The library's `SubscriptionEnd`, with its `type` first. */
	end: { type: string; statusCode?: string; errorReason?: string };
	/** The Group ID of every Object read, in the order read. */
	groups: string[];
	/** Stream Count of the PUBLISH_DONE, or null when none arrived. */
	streamCount: string | null;
	/** How reading the Objects ended: "closed", or the error `objects` failed with. */
	objectsEnd: string;
};

/** What `playVideoTrack` of `@xmoq/media` gave a page, in values that survive `page.evaluate`. */
export type PageVideoReport = {
	/**
	 * The frames the decoder gave, in the order it gave them: the time in
	 * microseconds, and the size the frame is shown at. With `readNumbers`,
	 * `number` is what the 8 bars of the picture say, or null where a bar
	 * is neither dark (below 64 of 255) nor light (above 192).
	 */
	frames: {
		timestamp: number;
		width: number;
		height: number;
		number?: number | null;
	}[];
	stats: {
		decoded: number;
		submitted: number;
		/** Objects that did not become a frame for the decoder, by the reason. */
		dropped: Record<string, number>;
		decoderFailures: number;
		lastDecoderError: string | null;
		endOfTrack: boolean;
	};
	/** How the playing ended, or null while it goes on: "subscription-ended <type>", "decoder-failed <error>" or "failed <error>". */
	ended: string | null;
};

/** What `publishVideo` is started with. */
export type PageVideoSetup = {
	trackName: string;
	/** What the encoder is configured with. */
	codec: string;
	/** What the catalog says of the track. */
	catalogCodec: string;
	width: number;
	height: number;
	framerate: number;
	/** Bits a second: for the encoder, and in the catalog, which wants it of a video track. */
	bitrate: number;
	keyFrameIntervalMs: number;
	mapping: "stream-per-group" | "stream-per-object";
	/** The time of frame 0, in microseconds. */
	firstTimestamp: number;
	/**
	 * Fills the lower half of every frame with random pixels, another set
	 * for every frame: a picture an encoder cannot make small. The bars of
	 * the number stay in the upper half.
	 */
	noise: boolean;
};

/** What `createVideoTrackEncoder` of `@xmoq/media` counted on a page. */
export type PageVideoPublishReport = {
	/** Frames the page painted. */
	painted: number;
	submitted: number;
	encoded: number;
	keyChunks: number;
	notEncoded: Record<string, number>;
	notSent: Record<string, number>;
	keyFramesAskedFor: number;
	encodedBytes: number;
	largestChunk: number;
	encoderError: string | null;
	/** Of the publisher: Objects handed to the transport, and writes that failed. */
	written: number;
	writeFailures: number;
};

/** What `followCatalog` of `@xmoq/msf` gave a page, in values that survive `page.evaluate`. */
export type PageCatalogReport = {
	states: {
		/** The catalog that is held, or null. */
		catalog: Catalog | null;
		/** Where it was, as [group, object] in decimal, or null. */
		at: [string, string] | null;
		/** Why the catalog may be outdated, or null. */
		outdated: string | null;
	}[];
	/** What became of the Joining FETCH: "not-sent", "served <objects>", or "failed <objects>: <error>". */
	joiningFetch: string;
};

declare global {
	interface Window {
		xmoq: XmoqPage;
		xmoqOwn?: OwnPageExtras;
	}
}
