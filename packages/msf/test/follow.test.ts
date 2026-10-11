import { describe, expect, test } from "bun:test";
import type {
	Fetch,
	FetchRequest,
	MoqtConnection,
	ReceivedObject,
	SubscribeRequest,
	Subscription,
	SubscriptionEnd,
} from "@xmoq/moqt";
import type { CatalogState } from "../src/core/catalog-track.ts";
import { followCatalog } from "../src/moqt/follow.ts";

// The connection is a fake that has the three calls `followCatalog` makes.
// The fetch stream is real bytes, which `decodeFetchBody` of @xmoq/moqt
// decodes; the run against another publisher is in interop/.

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);
const whole = (name: string): Uint8Array =>
	utf8(
		`{"version":"draft-01","tracks":[{"name":"${name}","packaging":"x","isLive":true}]}`,
	);
const DELTA = utf8('{"deltaUpdate":[{"op":"remove","tracks":[{"name":"a"}]}]}');

/**
 * An Object of a fetch stream after its FETCH_HEADER (draft-ietf-moq-
 * transport-18 §11.4.4.1), as in fixtures/moqt-18-by-hand: Serialization
 * Flags 0x1C (Group ID, Object ID and priority present, Subgroup 0), each
 * number in one byte (§1.4.1: a first bit of 0 and seven bits of value), so
 * for numbers and payloads below 128.
 */
function fetchObject(
	group: number,
	object: number,
	payload: Uint8Array,
): Uint8Array {
	if (group > 127 || object > 127 || payload.length > 127) {
		throw new Error("does not fit one byte each");
	}
	return new Uint8Array([0x1c, group, object, 7, payload.length, ...payload]);
}

/** A stream that the test writes to and ends. */
function hand<T>(): {
	stream: ReadableStream<T>;
	put(value: T): void;
	close(): void;
	fail(error: Error): void;
} {
	let controller: ReadableStreamDefaultController<T> | undefined;
	const stream = new ReadableStream<T>({
		start(started) {
			controller = started;
		},
	});
	return {
		stream,
		put: (value) => controller?.enqueue(value),
		close: () => {
			try {
				controller?.close();
			} catch {
				// Closed or failed already.
			}
		},
		fail: (error) => controller?.error(error),
	};
}

function received(
	group: bigint,
	object: bigint,
	payload: Uint8Array,
): ReceivedObject {
	return {
		groupId: group,
		subgroupId: 0n,
		publisherPriority: 128,
		object: { objectId: object, properties: [], status: "Normal", payload },
	};
}

interface Fake {
	connection: Pick<MoqtConnection, "subscribe" | "fetch" | "close">;
	subscribed: SubscribeRequest[];
	fetched: FetchRequest[];
	objects: ReturnType<typeof hand<ReceivedObject>>;
	/** Ends the subscription as its publisher would. */
	end(how: SubscriptionEnd): void;
	cancelled: { subscription: number; fetch: number };
	/** The signal the FETCH was sent with. */
	fetchSignal(): AbortSignal | undefined;
	/** The fetch stream of a FETCH answered with "held", which the test writes. */
	fetchBody: ReturnType<typeof hand<Uint8Array>>;
}

/**
 * `largest` is the LARGEST_OBJECT of the SUBSCRIBE_OK. `fetch` is how the
 * FETCH is answered: the bytes of its fetch stream, a refusal, or nothing
 * at all.
 */
function fake(options: {
	largest?: { group: bigint; object: bigint };
	fetch?: Uint8Array[] | "refused" | "silent" | "held" | "refused-with-objects";
	/** The SUBSCRIBE is answered when this resolves. */
	subscribeAfter?: Promise<void>;
	/** `Subscription.cancel` rejects, as when the reset cannot be written. */
	cancelFails?: boolean;
}): Fake {
	const objects = hand<ReceivedObject>();
	const cancelled = { subscription: 0, fetch: 0 };
	const subscribed: SubscribeRequest[] = [];
	const fetched: FetchRequest[] = [];
	let fetchSignal: AbortSignal | undefined;
	let end: (how: SubscriptionEnd) => void = () => {};
	const ended = new Promise<SubscriptionEnd>((resolve) => {
		end = resolve;
	});
	const subscription: Subscription = {
		requestId: 4n,
		trackAlias: 0n,
		parameters:
			options.largest === undefined ? {} : { largestObject: options.largest },
		trackProperties: [],
		objects: objects.stream,
		ended,
		cancel: async () => {
			cancelled.subscription += 1;
			objects.close();
			end({ type: "cancelled" });
			if (options.cancelFails === true) throw new Error("the reset failed");
		},
	};
	const heldBody = hand<Uint8Array>();
	const connection: Fake["connection"] = {
		subscribe: async (request) => {
			subscribed.push(request);
			await options.subscribeAfter;
			return subscription;
		},
		fetch: async (request, fetchOptions) => {
			fetched.push(request);
			fetchSignal = fetchOptions?.signal;
			const body = hand<Uint8Array>();
			const answer = options.fetch ?? [];
			let response: Fetch["response"];
			if (answer === "refused") {
				response = Promise.reject(new Error("REQUEST_ERROR"));
				body.fail(new Error("refused"));
			} else if (answer === "refused-with-objects") {
				// §5.2 of the transport: Objects can come for a FETCH that
				// ends in an error.
				response = Promise.reject(new Error("REQUEST_ERROR"));
				body.put(fetchObject(5, 0, whole("a")));
				body.close();
			} else if (answer === "held") {
				response = Promise.resolve({
					endOfTrack: false,
					endLocation: { group: 0n, object: 0n },
					parameters: {},
					trackProperties: [],
				});
				return {
					requestId: 6n,
					groupOrder: "Ascending",
					response,
					body: heldBody.stream,
					ended: new Promise(() => {}),
					cancel: async () => {
						cancelled.fetch += 1;
					},
				};
			} else if (answer === "silent") {
				response = new Promise((_resolve, reject) => {
					fetchSignal?.addEventListener("abort", () => {
						reject(new Error("cancelled"));
						body.fail(new Error("cancelled"));
					});
				});
			} else {
				response = Promise.resolve({
					endOfTrack: false,
					endLocation: { group: 0n, object: 0n },
					parameters: {},
					trackProperties: [],
				});
				for (const bytes of answer) body.put(bytes);
				body.close();
			}
			response.catch(() => {});
			return {
				requestId: 6n,
				groupOrder: "Ascending",
				response,
				body: body.stream,
				ended: new Promise(() => {}),
				cancel: async () => {
					cancelled.fetch += 1;
				},
			};
		},
		close: () => {},
	};
	return {
		connection,
		subscribed,
		fetched,
		objects,
		end: (how) => {
			objects.close();
			end(how);
		},
		cancelled,
		fetchSignal: () => fetchSignal,
		fetchBody: heldBody,
	};
}

/** The state in a line. */
function line(state: CatalogState): string {
	const { current, outdated } = state;
	const names =
		current === undefined
			? "none"
			: `${current.catalog.tracks.map((track) => track.name).join(",")} at {${current.at.group}, ${current.at.object}}`;
	return `${names}${outdated === undefined ? "" : `; outdated: ${outdated.because}`}`;
}

const NAMESPACE = [utf8("xmoq"), utf8("demo")];
const settle = (): Promise<void> =>
	new Promise((resolve) => setTimeout(resolve, 0));

describe("followCatalog", () => {
	test('subscribes to the track "catalog" of the namespace and sends a Relative Joining FETCH with offset 0 for that subscription', async () => {
		const peer = fake({
			largest: { group: 5n, object: 0n },
			fetch: [fetchObject(5, 0, whole("a"))],
		});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe("a at {5, 0}");
		expect(peer.subscribed).toEqual([
			{ trackNamespace: NAMESPACE, trackName: utf8("catalog") },
		]);
		expect(peer.fetched).toEqual([
			{
				range: {
					fetchType: "RelativeJoining",
					joiningRequestId: 4n,
					joiningStart: 0n,
				},
			},
		]);
	});

	test("the Object the FETCH brought is not taken a second time from the subscription, and what follows it is", async () => {
		const peer = fake({
			largest: { group: 5n, object: 0n },
			fetch: [fetchObject(5, 0, whole("a"))],
		});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe("a at {5, 0}");
		// As moqlivemock's publisher does: the newest Object on the
		// subscription as well.
		peer.objects.put(received(5n, 0n, whole("a")));
		peer.objects.put(received(6n, 0n, whole("b")));
		expect(line((await reader.read()).value ?? {})).toBe("b at {6, 0}");
	});

	test("without a LARGEST_OBJECT no FETCH is sent, and the catalog is the first one the subscription brings", async () => {
		const peer = fake({});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		const reader = follower.states.getReader();
		peer.objects.put(received(7n, 0n, whole("a")));
		expect(line((await reader.read()).value ?? {})).toBe("a at {7, 0}");
		expect(peer.fetched).toEqual([]);
	});

	test.each([
		{ name: "is refused", fetch: "refused" as const, fetchTimeoutMs: 15_000 },
		{
			name: "gets no answer in its time",
			fetch: "silent" as const,
			fetchTimeoutMs: 20,
		},
	])(
		"a FETCH that $name is given up, and the reading goes on with the subscription",
		async ({ fetch, fetchTimeoutMs }) => {
			const peer = fake({ largest: { group: 5n, object: 0n }, fetch });
			const follower = await followCatalog(peer.connection, {
				namespace: NAMESPACE,
				fetchTimeoutMs,
			});
			const reader = follower.states.getReader();
			peer.objects.put(received(6n, 0n, whole("b")));
			expect(line((await reader.read()).value ?? {})).toBe("b at {6, 0}");
			expect(peer.cancelled.subscription).toBe(0);
		},
	);

	test("a state nobody read is replaced by the next: a slow reader gets the newest, once", async () => {
		const peer = fake({});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		peer.objects.put(received(5n, 0n, whole("a")));
		peer.objects.put(received(6n, 0n, whole("b")));
		peer.objects.put(received(6n, 1n, DELTA));
		await settle();
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe(
			"b at {6, 0}; outdated: delta-update",
		);
		peer.end({ type: "publish-done", statusCode: 2n, errorReason: "" });
		expect(await reader.read()).toEqual({ done: true, value: undefined });
	});

	test("an Object that only has a status is no catalog", async () => {
		const peer = fake({});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		peer.objects.put({
			...received(5n, 0n, new Uint8Array(0)),
			object: {
				objectId: 0n,
				properties: [],
				status: "EndOfGroup",
				payload: new Uint8Array(0),
			},
		});
		peer.objects.put(received(6n, 0n, whole("b")));
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe("b at {6, 0}");
	});

	test.each([
		{
			name: "the publisher ends the subscription",
			end: (peer: Fake) =>
				peer.end({ type: "publish-done", statusCode: 2n, errorReason: "" }),
			how: { type: "publish-done", statusCode: 2n, errorReason: "" },
		},
		{
			name: "the subscription is cut off",
			end: (peer: Fake) => {
				peer.objects.fail(new Error("cut off"));
				peer.end({ type: "peer-reset", code: 1 });
			},
			how: { type: "peer-reset", code: 1 },
		},
	])(
		"when $name, the states close and `ended` says how",
		async ({ end, how }) => {
			const peer = fake({});
			const follower = await followCatalog(peer.connection, {
				namespace: NAMESPACE,
			});
			const reader = follower.states.getReader();
			end(peer);
			expect(await reader.read()).toEqual({ done: true, value: undefined });
			expect(await follower.ended).toEqual(how);
		},
	);

	test("the signal cancels the subscription, and the FETCH was sent with a signal that aborts with it", async () => {
		const peer = fake({ largest: { group: 5n, object: 0n }, fetch: "silent" });
		const aborting = new AbortController();
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
			signal: aborting.signal,
		});
		await settle();
		expect(peer.fetchSignal()?.aborted).toBe(false);
		aborting.abort();
		expect(peer.fetchSignal()?.aborted).toBe(true);
		expect(peer.cancelled.subscription).toBe(1);
		expect(await follower.states.getReader().read()).toEqual({
			done: true,
			value: undefined,
		});
		expect(await follower.ended).toEqual({ type: "cancelled" });
	});

	test("with a signal that has aborted nothing is sent", async () => {
		const peer = fake({});
		const reason = new Error("not now");
		const outcome = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
			signal: AbortSignal.abort(reason),
		}).then(
			() => "resolved",
			(error: unknown) => error,
		);
		expect(outcome).toBe(reason);
		expect(peer.subscribed).toEqual([]);
	});

	test("with a `maxLength` that is no length it rejects with a RangeError, and nothing is sent", async () => {
		const peer = fake({});
		const outcome = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
			maxLength: Number.NaN,
		}).then(
			() => "resolved",
			(error: unknown) => error,
		);
		expect(outcome).toBeInstanceOf(RangeError);
		expect(peer.subscribed).toEqual([]);
	});

	test("cancelling the states cancels the subscription", async () => {
		const peer = fake({});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		await follower.states.cancel();
		expect(peer.cancelled.subscription).toBe(1);
	});

	test("what the FETCH brings is read to its end before what the subscription brings, also when that came first", async () => {
		const peer = fake({ largest: { group: 5n, object: 0n }, fetch: "held" });
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		// The subscription has the catalog of a later Group already.
		peer.objects.put(received(6n, 0n, whole("b")));
		await settle();
		peer.fetchBody.put(fetchObject(5, 0, whole("a")));
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe("a at {5, 0}");
		peer.fetchBody.close();
		expect(line((await reader.read()).value ?? {})).toBe("b at {6, 0}");
	});

	test("an End of Range indicator on the fetch stream is passed over, and the Object after it is read", async () => {
		const peer = fake({
			largest: { group: 5n, object: 0n },
			fetch: [
				// End of Unknown Range at {3, 1}, as in
				// fixtures/moqt-18-by-hand; then an Object with Group ID Delta 1,
				// which counts from the indicator: Group 3 + 1 + 1.
				new Uint8Array([0x81, 0x0c, 3, 1]),
				new Uint8Array([0x1c, 1, 0, 7, whole("a").length, ...whole("a")]),
			],
		});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe("a at {5, 0}");
		expect(await follower.joiningFetch).toEqual({ type: "served", objects: 1 });
	});

	test("the Objects of a FETCH that was refused are not taken", async () => {
		const peer = fake({
			largest: { group: 5n, object: 0n },
			fetch: "refused-with-objects",
		});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		const outcome = await follower.joiningFetch;
		expect(outcome.type).toBe("failed");
		expect(outcome).toMatchObject({ objects: 0 });
		peer.objects.put(received(6n, 0n, whole("b")));
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe("b at {6, 0}");
	});

	test("`joiningFetch` says that no FETCH was sent where the SUBSCRIBE_OK had no LARGEST_OBJECT", async () => {
		const follower = await followCatalog(fake({}).connection, {
			namespace: NAMESPACE,
		});
		expect(await follower.joiningFetch).toEqual({ type: "not-sent" });
	});

	test("an Object of the fetch stream over the length that is read ends the FETCH, which is said", async () => {
		const peer = fake({
			largest: { group: 5n, object: 0n },
			fetch: [fetchObject(5, 0, whole("a"))],
		});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
			maxLength: 10,
		});
		const outcome = await follower.joiningFetch;
		expect(outcome.type).toBe("failed");
		expect(peer.cancelled.fetch).toBe(1);
	});

	test("cancelling the states gives the FETCH up as well", async () => {
		const peer = fake({ largest: { group: 5n, object: 0n }, fetch: "silent" });
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		await settle();
		expect(peer.fetchSignal()?.aborted).toBe(false);
		await follower.states.cancel();
		expect(peer.fetchSignal()?.aborted).toBe(true);
		expect((await follower.joiningFetch).type).toBe("failed");
	});

	test("when the subscription ends while the FETCH has no answer, the FETCH is given up and the states close", async () => {
		const peer = fake({ largest: { group: 5n, object: 0n }, fetch: "silent" });
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
		});
		await settle();
		const reader = follower.states.getReader();
		peer.end({ type: "publish-done", statusCode: 2n, errorReason: "" });
		expect(await reader.read()).toEqual({ done: true, value: undefined });
		expect(peer.fetchSignal()?.aborted).toBe(true);
	});

	test("a signal that aborts while the SUBSCRIBE is under way cancels the subscription once it is there", async () => {
		let answer: () => void = () => {};
		const peer = fake({
			subscribeAfter: new Promise((resolve) => {
				answer = resolve;
			}),
		});
		const aborting = new AbortController();
		const following = followCatalog(peer.connection, {
			namespace: NAMESPACE,
			signal: aborting.signal,
		});
		await settle();
		aborting.abort();
		answer();
		const follower = await following;
		expect(peer.cancelled.subscription).toBe(1);
		expect(await follower.ended).toEqual({ type: "cancelled" });
	});

	test("a cancel that fails is nobody's unhandled rejection", async () => {
		const peer = fake({ cancelFails: true });
		const unhandled: unknown[] = [];
		const note = (reason: unknown): void => {
			unhandled.push(reason);
		};
		process.on("unhandledRejection", note);
		try {
			const follower = await followCatalog(peer.connection, {
				namespace: NAMESPACE,
			});
			await follower.states.cancel();
			await new Promise((resolve) => setTimeout(resolve, 10));
			expect(peer.cancelled.subscription).toBe(1);
			expect(unhandled).toEqual([]);
		} finally {
			process.off("unhandledRejection", note);
		}
	});

	test("a catalog Object over the length that is read leaves the state outdated", async () => {
		const peer = fake({});
		const follower = await followCatalog(peer.connection, {
			namespace: NAMESPACE,
			maxLength: 10,
		});
		peer.objects.put(received(5n, 0n, whole("a")));
		const reader = follower.states.getReader();
		expect(line((await reader.read()).value ?? {})).toBe(
			"none; outdated: unreadable",
		);
	});
});
