// Spike: the page of run.ts. One page publishes Groups as fast as it can, another
// subscribes with @xmoq/moqt and never reads the Objects.
import { connect, type MoqtConnection, type Subscription } from "../../packages/moqt/src/index.ts";

const utf8 = new TextEncoder();
const NAMESPACE = [utf8.encode("xmoq"), utf8.encode("backlog")];
const TRACK = utf8.encode("object");
const SECOND_TRACK = utf8.encode("second");
let moq: MoqtConnection | undefined;
let subscription: Subscription | undefined;
let handedOver = 0;
let ended: string | undefined;
let sent = { groups: 0, objects: 0, stopped: "" };

const api = {
	/** `maxPeerUniStreams` 0 keeps the default of the library. */
	async connect(url: string, hashHex: string, maxPeerUniStreams = 0) {
		const hash = new Uint8Array((hashHex.match(/../g) ?? []).map((h) => Number.parseInt(h, 16)));
		moq = await connect(url, {
			serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
			...(maxPeerUniStreams > 0 ? { maxPeerUniStreams } : {}),
			diagnostics: { onEvent(event) { if (event.type === "object") handedOver += 1; } },
		});
	},
	async subscribeAndNeverRead() {
		if (moq === undefined) throw new Error("not connected");
		subscription = await moq.subscribe({
			trackNamespace: NAMESPACE,
			trackName: TRACK,
			parameters: { forward: true, subscriberPriority: 128, subscriptionFilter: { filterType: "LargestObject" }, groupOrder: "Ascending" },
		});
		void subscription.ended.then((end) => {
			ended = JSON.stringify(end, (_k, v) => (typeof v === "bigint" ? v.toString() : v));
		});
	},
	/**
	 * Subscribes to the second track, which the publisher offers and never
	 * writes to, while the first subscription is not read. Resolves with how
	 * the request went and how long it took.
	 */
	async subscribeToSecond(limitMs: number) {
		if (moq === undefined) throw new Error("not connected");
		const began = performance.now();
		const outcome = await Promise.race([
			moq
				.subscribe({
					trackNamespace: NAMESPACE,
					trackName: SECOND_TRACK,
					parameters: { forward: true, subscriberPriority: 128, subscriptionFilter: { filterType: "LargestObject" }, groupOrder: "Ascending" },
				})
				.then(() => "answered", (error: unknown) => `failed: ${String(error)}`),
			new Promise<string>((resolve) => setTimeout(() => resolve("no answer"), limitMs)),
		]);
		return `${outcome} after ${Math.round(performance.now() - began)} ms`;
	},
	/** Starts reading the Objects after all; resolves with how many were read when the stream ends. */
	async readAll() {
		if (subscription === undefined) throw new Error("not subscribed");
		const reader = subscription.objects.getReader();
		let read = 0;
		try {
			for (;;) {
				const next = await reader.read();
				if (next.done) return `closed after ${read}`;
				read += 1;
			}
		} catch (error) {
			return `failed after ${read}: ${String(error)}`;
		}
	},
	stats() {
		return { handedOver, ended: ended ?? null };
	},
	/** Publishes Groups of `objectsPerGroup` Objects of `objectBytes` bytes, each Group on one stream, until told to stop. */
	async publish(objectsPerGroup: number, objectBytes: number) {
		if (moq === undefined) throw new Error("not connected");
		await moq.publishNamespace(NAMESPACE);
		const track = await moq.publish({ trackNamespace: NAMESPACE, trackName: TRACK, trackAlias: 1n, parameters: { forward: true } });
		// A second track that never carries an Object, for `subscribeToSecond`.
		await moq.publish({ trackNamespace: NAMESPACE, trackName: SECOND_TRACK, trackAlias: 2n, parameters: { forward: true } });
		const payload = new Uint8Array(objectBytes).fill(0x61);
		void (async () => {
			try {
				for (let group = 0; sent.stopped === ""; group++) {
					const subgroup = await track.openSubgroup({ groupId: BigInt(group), subgroupId: { mode: "Explicit", value: 0n }, publisherPriority: 0, properties: false, endOfGroup: true });
					for (let object = 0; object < objectsPerGroup; object++) {
						await subgroup.write({ objectId: BigInt(object), payload });
						sent.objects += 1;
					}
					await subgroup.close();
					sent.groups += 1;
				}
			} catch (error) {
				sent.stopped = `failed: ${String(error)}`;
			}
		})();
	},
	sent() {
		return sent;
	},
	stop() {
		sent = { ...sent, stopped: "stopped" };
	},
};
Object.assign(globalThis, { backlog: api });
