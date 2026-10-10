// Spike: the page of run.ts. One page publishes Objects of the size of a video frame
// at a fixed rate, either one a stream or a whole Group on one stream. Others subscribe
// with @xmoq/moqt and read everything, read slowly, or never read.
import { connect, type MoqtConnection, type Subscription } from "../../packages/moqt/src/index.ts";

const utf8 = new TextEncoder();
const NAMESPACE = [utf8.encode("xmoq"), utf8.encode("frames")];
const TRACK = utf8.encode("video");
const SECOND_TRACK = utf8.encode("second");
const FILTER = { forward: true, subscriberPriority: 128, subscriptionFilter: { filterType: "LargestObject" as const }, groupOrder: "Ascending" as const };
let moq: MoqtConnection | undefined;
let subscription: Subscription | undefined;
let handedOver = 0;
let read = 0;
let readEnd: string | undefined;
let ended: string | undefined;
let publishDoneAt: number | undefined;
let sent = { objects: 0, streams: 0, late: 0, stopped: "", startedAt: 0, lastAt: 0 };
const text = (value: unknown) => JSON.stringify(value, (_k, v) => (typeof v === "bigint" ? v.toString() : v));

const api = {
	async connect(url: string, hashHex: string) {
		const hash = new Uint8Array((hashHex.match(/../g) ?? []).map((h) => Number.parseInt(h, 16)));
		moq = await connect(url, {
			serverCertificateHashes: [{ algorithm: "sha-256", value: hash }],
			diagnostics: { onEvent(event) { if (event.type === "object") handedOver += 1; if (event.type === "publish-done") publishDoneAt = performance.now(); } },
		});
	},
	/** `readPerSecond`: 0 never reads; Infinity (given as -1) reads as fast as Objects come; another number reads that many a second. */
	async subscribe(readPerSecond: number) {
		if (moq === undefined) throw new Error("not connected");
		subscription = await moq.subscribe({ trackNamespace: NAMESPACE, trackName: TRACK, parameters: FILTER });
		void subscription.ended.then((end) => { ended = text(end); });
		if (readPerSecond === 0) return;
		const reader = subscription.objects.getReader();
		void (async () => {
			try {
				const start = performance.now();
				for (;;) {
					if (readPerSecond > 0) {
						const due = start + (read * 1000) / readPerSecond;
						const wait = due - performance.now();
						if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
					}
					const next = await reader.read();
					if (next.done) { readEnd = "closed"; return; }
					read += 1;
				}
			} catch (error) {
				readEnd = `failed: ${String(error)}`;
			}
		})();
	},
	/** Subscribes to a second track of the same publisher, which never carries an Object: does the answer arrive? */
	async subscribeToSecond(limitMs: number) {
		if (moq === undefined) throw new Error("not connected");
		const began = performance.now();
		const outcome = await Promise.race([
			moq.subscribe({ trackNamespace: NAMESPACE, trackName: SECOND_TRACK, parameters: FILTER }).then(() => "answered", (error: unknown) => `failed: ${String(error)}`),
			new Promise<string>((resolve) => setTimeout(() => resolve("no answer"), limitMs)),
		]);
		return `${outcome} after ${Math.round(performance.now() - began)} ms`;
	},
	stats() {
		return { handedOver, read, readEnd: readEnd ?? null, ended: ended ?? null, publishDoneSeenMs: publishDoneAt === undefined ? null : Math.round(publishDoneAt) , now: Math.round(performance.now()) };
	},
	/**
	 * Publishes Objects of `objectBytes` bytes, `perSecond` a second, in Groups of
	 * `objectsPerGroup`. `onePerStream`: every Object on a stream of its own (a Subgroup of
	 * its own, named by its Object ID); otherwise a Group on one stream.
	 */
	async publish(objectBytes: number, perSecond: number, objectsPerGroup: number, onePerStream: boolean) {
		if (moq === undefined) throw new Error("not connected");
		await moq.publishNamespace(NAMESPACE);
		const track = await moq.publish({ trackNamespace: NAMESPACE, trackName: TRACK, trackAlias: 1n, parameters: { forward: true } });
		await moq.publish({ trackNamespace: NAMESPACE, trackName: SECOND_TRACK, trackAlias: 2n, parameters: { forward: true } });
		const payload = new Uint8Array(objectBytes).fill(0x61);
		void (async () => {
			try {
				sent.startedAt = performance.now();
				let stream: Awaited<ReturnType<typeof track.openSubgroup>> | undefined;
				for (let index = 0; sent.stopped === ""; index++) {
					const due = sent.startedAt + (index * 1000) / perSecond;
					const wait = due - performance.now();
					if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
					else if (-wait > 1000 / perSecond) sent.late += 1;
					const group = Math.floor(index / objectsPerGroup);
					const object = index % objectsPerGroup;
					const last = object === objectsPerGroup - 1;
					if (onePerStream) {
						const own = await track.openSubgroup({ groupId: BigInt(group), subgroupId: { mode: "Explicit", value: BigInt(object) }, publisherPriority: 0, properties: false, endOfGroup: last });
						await own.write({ objectId: BigInt(object), payload });
						await own.close();
						sent.streams += 1;
					} else {
						if (object === 0) { stream = await track.openSubgroup({ groupId: BigInt(group), subgroupId: { mode: "Explicit", value: 0n }, publisherPriority: 0, properties: false, endOfGroup: true }); sent.streams += 1; }
						await stream?.write({ objectId: BigInt(object), payload });
						if (last) await stream?.close();
					}
					sent.objects += 1;
					sent.lastAt = performance.now();
				}
			} catch (error) {
				sent.stopped = `failed: ${String(error)}`;
			}
		})();
	},
	sent() {
		return { ...sent, elapsedMs: Math.round(sent.lastAt - sent.startedAt) };
	},
	stop() {
		sent = { ...sent, stopped: "stopped" };
	},
};
Object.assign(globalThis, { frames: api });
