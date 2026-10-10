// Contract between the driver (`run.ts`) and every browser page it drives.
// Each page assigns an implementation to `window.xmoq`, so the driver can swap
// one client for another without knowing which library is behind it.

/** The first Object a page received, in strings that survive `page.evaluate`. */
export type PageReceivedObject = {
	group: string;
	object: string;
	payloadHex: string;
};

export type XmoqPage = {
	/**
	 * `certSha256Hex` is null when the browser trusts the relay by other means.
	 * The driver always passes a hash; null and `disconnect` are part of the
	 * contract because the moqtail page, copied as it was, has them.
	 */
	connect(relayUrl: string, certSha256Hex: string | null): Promise<void>;
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
	): Promise<void>;
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
	/** Cancels the subscription and keeps the session. */
	cancelSubscription(): Promise<void>;
	/** Resolves once the subscription has ended and its Objects were read; JSON of `SubscriptionReport`. */
	subscriptionReport(): Promise<string>;
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

declare global {
	interface Window {
		xmoq: XmoqPage;
		xmoqOwn?: OwnPageExtras;
	}
}
