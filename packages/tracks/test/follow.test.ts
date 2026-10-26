import { expect, test } from "bun:test";
import type {
	ReceivedObject,
	SessionClose,
	Subscription,
	SubscriptionEnd,
} from "@xmoq/moqt";
import { followTrack } from "../src/follow.ts";

/**
 * A SUBSCRIBE whose answer comes when the test says: `accept()` resolves it
 * with a subscription that ends, as `cancelled`, when it is cancelled, and
 * whose `objects` closes then, as `@xmoq/moqt`'s does.
 */
function answeredLate(): {
	subscribe(): Promise<Subscription>;
	asked: Promise<void>;
	accept(): void;
	cancelled(): number;
} {
	let cancelled = 0;
	const { promise: asked, resolve: ask } = Promise.withResolvers<void>();
	const { promise: answer, resolve } = Promise.withResolvers<Subscription>();
	const { promise: ended, resolve: end } =
		Promise.withResolvers<SubscriptionEnd>();
	let close: () => void = () => {};
	const objects = new ReadableStream<ReceivedObject>({
		start(controller) {
			close = () => controller.close();
		},
	});
	return {
		subscribe() {
			ask();
			return answer;
		},
		asked,
		accept() {
			resolve({
				requestId: 0n,
				trackAlias: 0n,
				parameters: {},
				trackProperties: [],
				objects,
				ended,
				goAway: new Promise(() => {}),
				async cancel() {
					cancelled += 1;
					close();
					end({ type: "cancelled" });
				},
				update: () => Promise.reject(new Error("no update in this test")),
			});
		},
		cancelled: () => cancelled,
	};
}

const options = {
	trackNamespace: [new TextEncoder().encode("test")],
	trackName: new TextEncoder().encode("items"),
	recent: 16,
	catchUp: 16,
	remembered: 16,
	maxPayloadLength: 1024,
	take() {},
};

const turns = async (): Promise<void> => {
	for (let turn = 0; turn < 10; turn++) await Promise.resolve();
};

test.each([
	["the signal", "signal"],
	["the connection closing", "closed"],
])(
	"a subscription made after %s ended the following is cancelled",
	async (_case, how) => {
		const late = answeredLate();
		const stop = new AbortController();
		const { promise: closed, resolve: close } =
			Promise.withResolvers<SessionClose>();
		followTrack(
			{
				subscribe: () => late.subscribe(),
				fetch: () => Promise.reject(new Error("no FETCH in this test")),
				close() {},
				closed,
			},
			{ ...options, signal: stop.signal },
		);
		await late.asked;
		if (how === "signal") stop.abort();
		else close({ type: "local", code: 0, reason: "" });
		await turns();
		late.accept();
		await turns();
		expect(late.cancelled()).toBe(1);
	},
);

// WHY: a SUBSCRIBE on a closed connection is refused at once, and asked for
// again after each refusal it would go on for as long as the reader is kept.
test("no SUBSCRIBE is made after the connection closed", async () => {
	let subscribes = 0;
	const { promise: closed, resolve: close } =
		Promise.withResolvers<SessionClose>();
	followTrack(
		{
			async subscribe() {
				subscribes += 1;
				throw new Error("refused by the test");
			},
			fetch: () => Promise.reject(new Error("no FETCH in this test")),
			close() {},
			closed,
		},
		options,
	);
	await turns();
	close({ type: "local", code: 0, reason: "" });
	await new Promise((resolve) => setTimeout(resolve, 1_200));
	expect(subscribes).toBe(1);
});
