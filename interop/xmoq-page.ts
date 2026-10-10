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
	/** How the subscription ended, as JSON of the library's `SubscriptionEnd`. */
	subscriptionEnd(): Promise<string>;
};

declare global {
	interface Window {
		xmoq: XmoqPage;
		xmoqOwn?: OwnPageExtras;
	}
}
