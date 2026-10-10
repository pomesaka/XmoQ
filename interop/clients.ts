// The four clients of the interop runs behind one shape each for a publisher
// and a subscriber, so that a driver can put any of them in any role. What a
// client cannot do rejects when asked for.

import {
	type BrowserClient,
	type Client,
	captureGroup,
	isBrowserClient,
	type LoggedProcess,
	NAMESPACE,
	openConnectedPage,
	type RunContext,
	type RustClient,
	spawnRustClient,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	SubscriptionReport,
	XmoqPage,
} from "./xmoq-page.ts";

// The functions passed to `page.evaluate` run in the page, where this global
// exists. `xmoqOwn` is missing on the moqtail page; it is read only where the
// client is own-browser, and a mistake there fails the call.
declare const window: {
	readonly xmoq: XmoqPage;
	readonly xmoqOwn: OwnPageExtras;
};

/**
 * Bounds each Rust client. WHY not their default of 20 seconds: a run that
 * waits for the relay's idle timeout, 6 to 9 seconds, and has several steps
 * before and after must not end because a client gave up.
 */
const RUST_CLIENT_TIMEOUT_SECS = "100";

/** A started relay and the payload every Object of the run carries. */
export interface ClientRun {
	context: RunContext;
	payloadHex: string;
}

const LEAVE_WAYS = ["cancel", "close", "kill"] as const;
/**
 * cancel: the subscription is ended and the session kept.
 * close: the client closes its session in order.
 * kill: the process is killed, or the page closed, without a word.
 */
export type LeaveWay = (typeof LEAVE_WAYS)[number];

export function isLeaveWay(value: string): value is LeaveWay {
	return LEAVE_WAYS.some((way) => way === value);
}

/** Calls the own page and bounds the wait. */
function onPage<T>(
	label: string,
	what: string,
	call: Promise<T>,
	timeoutMs?: number,
): Promise<T> {
	return withTimeout(call, `${label} to ${what}`, timeoutMs);
}

export interface Publisher {
	kind: Client;
	label: string;
	/** The moqtail clients send Object 0 of Group 0 and nothing else. */
	sendsSeveral: boolean;
	/** Sends Object 0 of the next Group on a stream of its own, finishes the stream, and resolves with the Group. */
	send(): Promise<number>;
	/** Sends `count` Objects like `send`, one after the other, and resolves with their Groups. */
	sendBatch(count: number): Promise<number[]>;
	/** Like `send`, and leaves the stream open. Only own-browser can. */
	sendUnfinished(): Promise<number>;
	/**
	 * How many of the streams it wrote are not finished. The relay has to end
	 * each of them with a reset towards the subscribers when the publisher is
	 * gone (§11.4.3).
	 */
	streamsLeftOpen(): number;
	/**
	 * Leaves in order and resolves with what the client says about it. The
	 * own clients end their Track with PUBLISH_DONE; the moqtail clients have
	 * no way to and end the session.
	 */
	end(): Promise<string>;
	/** Kills the process, or closes the page, without a word to the relay. */
	kill(): Promise<void>;
	/** Sends the next Object and closes the session at once; resolves with the Group. */
	sendLastAndClose(publishDone: boolean): Promise<number>;
}

/** What `sendLastAndClose` can do in each client. */
export const LAST_OBJECT_WITH_PUBLISH_DONE: Record<Client, readonly boolean[]> =
	{
		"own-browser": [true, false],
		// Closing stdin is its one way to leave, and it sends PUBLISH_DONE then.
		"own-rust": [true],
		"moqtail-browser": [false],
		"moqtail-rust": [false],
	};

async function exitedCleanly(
	client: LoggedProcess,
	label: string,
): Promise<string> {
	const code = await withTimeout(client.exited, `${label} to exit`);
	if (code !== 0) throw new Error(`${label} exited with code ${code}`);
	return "exit 0";
}

async function startRustPublisher(
	kind: RustClient,
	label: string,
	run: ClientRun,
): Promise<Publisher> {
	const client = spawnRustClient(run.context, label, kind, "publish", TRACK, [
		"--payload-hex",
		run.payloadHex,
		"--timeout-secs",
		RUST_CLIENT_TIMEOUT_SECS,
	]);
	await client.waitForLine(/^PUBLISHED /);
	let nextGroup = 0;
	const sent = async (): Promise<number> => {
		const group = nextGroup;
		nextGroup += 1;
		await client.waitForLine(new RegExp(`^SENT group=${group} `));
		return group;
	};
	return {
		kind,
		label,
		sendsSeveral: kind === "own-rust",
		send() {
			client.child.stdin.write("send\n");
			return sent();
		},
		async sendBatch(count) {
			// One write: the client sends them as fast as the relay takes them.
			client.child.stdin.write("send\n".repeat(count));
			const groups: number[] = [];
			for (let index = 0; index < count; index++) groups.push(await sent());
			return groups;
		},
		sendUnfinished: () =>
			Promise.reject(new Error(`${kind} finishes every stream it opens`)),
		streamsLeftOpen: () => 0,
		end() {
			client.child.stdin.end();
			return exitedCleanly(client, label);
		},
		async kill() {
			client.child.kill("SIGKILL");
			await client.exited;
		},
		async sendLastAndClose() {
			// One write: the client reads the line and the end of stdin together.
			client.child.stdin.end("send\n");
			const group = await sent();
			await exitedCleanly(client, label);
			return group;
		},
	};
}

async function startBrowserPublisher(
	kind: BrowserClient,
	label: string,
	run: ClientRun,
): Promise<Publisher> {
	const page = await openConnectedPage(run.context, label, kind);
	await onPage(
		label,
		"publish the track",
		page.evaluate(([ns, track]) => window.xmoq.publishTrack(ns, track), [
			NAMESPACE,
			TRACK,
		] satisfies [string, string]),
	);
	const payloadHex = run.payloadHex;
	const kill = (): Promise<void> => page.context().close();
	if (kind === "moqtail-browser") {
		let sentGroup0 = false;
		return {
			kind,
			label,
			sendsSeveral: false,
			async send() {
				await page.evaluate((hex) => window.xmoq.sendObject(hex), payloadHex);
				sentGroup0 = true;
				return 0;
			},
			sendBatch: () => Promise.reject(new Error(`${kind} sends one Object`)),
			sendUnfinished: () =>
				Promise.reject(new Error(`${kind} decides itself when to finish`)),
			// moqtail 0.13.0 keeps the stream of a Group open for more Objects of
			// the Group: in every run, the relay reset the stream towards the
			// subscribers when this publisher left.
			streamsLeftOpen: () => (sentGroup0 ? 1 : 0),
			// WHY the page is closed: it is the one way this client has to end
			// its session. Its `disconnect()` leaves the session open (see
			// `leave` of the moqtail-browser subscriber below).
			async end() {
				await kill();
				return "page closed";
			},
			kill,
			async sendLastAndClose() {
				await onPage(
					label,
					"send and disconnect",
					page.evaluate(async (hex) => {
						await window.xmoq.sendObject(hex);
						await window.xmoq.disconnect();
					}, payloadHex),
				);
				return 0;
			},
		};
	}
	let nextGroup = 0;
	let leftOpen = 0;
	const sendIn = async (finish: boolean): Promise<number> => {
		const group = nextGroup;
		nextGroup += 1;
		if (!finish) leftOpen += 1;
		await onPage(
			label,
			`send Group ${group}`,
			page.evaluate(
				([g, hex, fin]) => window.xmoqOwn.sendObjectIn(g, hex, fin),
				[String(group), payloadHex, finish] satisfies [string, string, boolean],
			),
		);
		return group;
	};
	return {
		kind,
		label,
		sendsSeveral: true,
		send: () => sendIn(true),
		async sendBatch(count) {
			const groups: number[] = [];
			for (let index = 0; index < count; index++) {
				groups.push(await sendIn(true));
			}
			return groups;
		},
		sendUnfinished: () => sendIn(false),
		streamsLeftOpen: () => leftOpen,
		async end() {
			await onPage(
				label,
				"end the track",
				page.evaluate(() => window.xmoqOwn.endTrack()),
			);
			return onPage(
				label,
				"report how its subscription ended",
				page.evaluate(() => window.xmoqOwn.publishEnd()),
			);
		},
		kill,
		async sendLastAndClose(publishDone) {
			const group = nextGroup;
			nextGroup += 1;
			await onPage(
				label,
				"send the last Object and close",
				page.evaluate(
					([g, hex, done]) =>
						window.xmoqOwn.sendLastObjectAndClose(g, hex, done),
					[String(group), payloadHex, publishDone] satisfies [
						string,
						string,
						boolean,
					],
				),
			);
			return group;
		},
	};
}

/** Connects a publisher, announces the namespace and offers the track. */
export function startPublisher(
	kind: Client,
	run: ClientRun,
): Promise<Publisher> {
	const label = `pub:${kind}`;
	return isBrowserClient(kind)
		? startBrowserPublisher(kind, label, run)
		: startRustPublisher(kind, label, run);
}

function isStringOrAbsent(value: unknown): value is string | undefined {
	return value === undefined || typeof value === "string";
}

/** The report of the own page, checked field by field: it comes from another program. */
function parseSubscriptionReport(text: string): SubscriptionReport {
	const refuse = (): never => {
		throw new Error(`not a report of a subscription: ${text}`);
	};
	const parsed: unknown = JSON.parse(text);
	if (typeof parsed !== "object" || parsed === null) return refuse();
	if (
		!("end" in parsed && "groups" in parsed) ||
		!("streamCount" in parsed && "objectsEnd" in parsed)
	) {
		return refuse();
	}
	const { end, groups, streamCount, objectsEnd } = parsed;
	if (typeof end !== "object" || end === null || !("type" in end)) {
		return refuse();
	}
	const statusCode = "statusCode" in end ? end.statusCode : undefined;
	const errorReason = "errorReason" in end ? end.errorReason : undefined;
	if (
		typeof end.type !== "string" ||
		!isStringOrAbsent(statusCode) ||
		!isStringOrAbsent(errorReason) ||
		!Array.isArray(groups) ||
		!groups.every((group): group is string => typeof group === "string") ||
		!(streamCount === null || typeof streamCount === "string") ||
		typeof objectsEnd !== "string"
	) {
		return refuse();
	}
	return {
		end: {
			type: end.type,
			...(statusCode === undefined ? {} : { statusCode }),
			...(errorReason === undefined ? {} : { errorReason }),
		},
		groups,
		streamCount,
		objectsEnd,
	};
}

/** How a subscription of an own client ended, in the terms both of them report. */
export interface SubscriptionOutcome {
	/**
	 * own-browser: the `type` of the library's `SubscriptionEnd`. own-rust:
	 * "publish-done" when it printed PUBLISH_DONE, and otherwise what it gave
	 * as the reason in SUBSCRIPTION_ENDED.
	 */
	type: string;
	/** Status Code of the PUBLISH_DONE, when one arrived. */
	statusCode: number | null;
	/** Stream Count of the PUBLISH_DONE, when one arrived. */
	streamCount: string | null;
	/** The Group of every Object the client took, in the order taken. */
	groups: number[];
	/**
	 * own-rust only, null for own-browser. The Stream Reset Error Code of
	 * every subgroup stream that ended without a FIN; "none" where the client
	 * printed no code.
	 */
	abortedStreams: string[] | null;
	/**
	 * own-rust only, null for own-browser: whether every stream the
	 * PUBLISH_DONE counted had ended when the client dropped the subscription.
	 * False means it gave up waiting for them (§10.11).
	 */
	allStreamsEnded: boolean | null;
	/** What the client itself printed or returned. */
	reported: string;
}

export interface Subscriber {
	kind: Client;
	label: string;
	/**
	 * Resolves with the payload of Object 0 of the Group once it arrived. A
	 * moqtail client reports its first Object only, whatever its Group.
	 */
	object(group: number): Promise<string>;
	/** Resolves once the subscription has ended. Only the own clients can tell. */
	outcome(timeoutMs?: number): Promise<SubscriptionOutcome>;
	leave(way: LeaveWay): Promise<void>;
	/**
	 * Stops the process, so that it takes nothing and answers nothing, or lets
	 * it run again. Only a Rust client can be stopped.
	 */
	setStopped(stopped: boolean): Promise<void>;
}

function cannotTell(kind: Client): () => Promise<never> {
	return () =>
		Promise.reject(
			new Error(`${kind} does not report how a subscription ended`),
		);
}

function cannotLeave(kind: Client, way: LeaveWay): Promise<never> {
	return Promise.reject(new Error(`${kind} cannot leave by "${way}"`));
}

function cannotBeStopped(kind: Client): () => Promise<never> {
	return () => Promise.reject(new Error(`a ${kind} page cannot be stopped`));
}

/** What a subscriber is started for. */
export interface SubscriberPlan {
	/** The track to ask for, where it is not the published one. */
	track?: string;
	/** own-rust: how many Objects it takes before it leaves in order. Absent: it stays. */
	leavesAfterObjects?: number;
	/** own-browser: subscribes and never reads the Objects. */
	withoutReading?: boolean;
}

function startRustSubscriber(
	kind: RustClient,
	label: string,
	run: ClientRun,
	plan: SubscriberPlan,
): Subscriber & { subscribed: Promise<unknown> } {
	const track = plan.track ?? TRACK;
	const client = spawnRustClient(run.context, label, kind, "subscribe", track, [
		"--timeout-secs",
		RUST_CLIENT_TIMEOUT_SECS,
		// The moqtail client takes one Object and leaves, and has no flag for it.
		// WHY a number no run reaches: it keeps the own client subscribed until
		// the subscription is ended for it.
		...(kind === "own-rust"
			? ["--objects", String(plan.leavesAfterObjects ?? 1_000_000)]
			: []),
	]);
	const kill = async (): Promise<void> => {
		client.child.kill("SIGKILL");
		await client.exited;
	};
	const objectOf = async (pattern: RegExp): Promise<string> =>
		captureGroup(await client.waitForLine(pattern), 1);
	const setStopped = async (stopped: boolean): Promise<void> => {
		client.child.kill(stopped ? "SIGSTOP" : "SIGCONT");
	};
	if (kind === "moqtail-rust") {
		return {
			kind,
			label,
			subscribed: client.waitForLine(/^SUBSCRIBED /),
			object: () => objectOf(/^OBJECT .*payload_hex=([0-9a-f]*)$/),
			outcome: cannotTell(kind),
			setStopped,
			async leave(way) {
				if (way === "kill") return kill();
				// It closes its session by itself once it has its Object.
				if (way === "close") {
					await exitedCleanly(client, label);
					return;
				}
				return cannotLeave(kind, way);
			},
		};
	}
	return {
		kind,
		label,
		subscribed: client.waitForLine(/^SUBSCRIBED /),
		object: (group) =>
			objectOf(new RegExp(`^OBJECT group=${group} .*payload_hex=([0-9a-f]*)$`)),
		async outcome(timeoutMs) {
			const ended = captureGroup(
				await client.waitForLine(
					/^SUBSCRIPTION_ENDED why=(.*) received=/,
					timeoutMs,
				),
				1,
			);
			const done = client.matches(
				/^PUBLISH_DONE status_code=(0x[0-9a-f]+) stream_count=(\d+) /,
			)[0];
			const aborted = client.matches(/^STREAM_ABORTED \S+(?: code=(\S+))?$/);
			return {
				type: done === undefined ? ended : "publish-done",
				statusCode: done === undefined ? null : Number(captureGroup(done, 1)),
				streamCount: done === undefined ? null : captureGroup(done, 2),
				groups: client
					.matches(/^OBJECT group=(\d+) /)
					.map((match) => Number(captureGroup(match, 1))),
				abortedStreams: aborted.map((match) => match[1] ?? "none"),
				allStreamsEnded: ended === "PUBLISH_DONE, all streams closed",
				reported: [
					done?.[0],
					`SUBSCRIPTION_ENDED why=${ended}`,
					...aborted.map((match) => match[0]),
				]
					.filter((line) => line !== undefined)
					.join("; "),
			};
		},
		setStopped,
		async leave(way) {
			if (way === "kill") return kill();
			if (way === "close" && plan.leavesAfterObjects !== undefined) {
				// It cancels its subscription and closes the session once it has
				// the Objects it was started for.
				await exitedCleanly(client, label);
				return;
			}
			return cannotLeave(kind, way);
		},
	};
}

async function startBrowserSubscriber(
	kind: BrowserClient,
	label: string,
	run: ClientRun,
	plan: SubscriberPlan,
): Promise<Subscriber> {
	const page = await openConnectedPage(run.context, label, kind);
	const names = [NAMESPACE, plan.track ?? TRACK] satisfies [string, string];
	if (kind === "moqtail-browser") {
		await onPage(
			label,
			"subscribe",
			page.evaluate(([ns, name]) => window.xmoq.subscribe(ns, name), names),
		);
		return {
			kind,
			label,
			async object() {
				const object = await onPage(
					label,
					"receive",
					page.evaluate(() => window.xmoq.firstObject()),
				);
				return object.payloadHex;
			},
			outcome: cannotTell(kind),
			setStopped: cannotBeStopped(kind),
			async leave(way) {
				if (way === "kill") return page.context().close();
				// WHY cancel and not close: `disconnect()` of moqtail 0.13.0 closes
				// its request streams and leaves the WebTransport session open. It
				// tests `webTransport.closed`, a promise, before calling `close()`
				// (dist/client-*.mjs, read on 2026-10-10), and the own relay kept
				// the session of a page that had called it.
				if (way === "cancel") {
					return onPage(
						label,
						"disconnect",
						page.evaluate(() => window.xmoq.disconnect()),
					);
				}
				return cannotLeave(kind, way);
			},
		};
	}
	await onPage(
		label,
		"subscribe",
		plan.withoutReading === true
			? page.evaluate(
					([ns, name]) => window.xmoqOwn.subscribeWithoutReading(ns, name),
					names,
				)
			: page.evaluate(([ns, name]) => window.xmoq.subscribe(ns, name), names),
	);
	return {
		kind,
		label,
		object: (group) =>
			onPage(
				label,
				`receive Group ${group}`,
				page.evaluate((g) => window.xmoqOwn.objectOfGroup(g), String(group)),
			),
		async outcome(timeoutMs) {
			const reported = await onPage(
				label,
				"report how its subscription ended",
				page.evaluate(() => window.xmoqOwn.subscriptionReport()),
				timeoutMs,
			);
			const report = parseSubscriptionReport(reported);
			return {
				type: report.end.type,
				statusCode:
					report.end.statusCode === undefined
						? null
						: Number(report.end.statusCode),
				streamCount: report.streamCount,
				groups: report.groups.map(Number),
				abortedStreams: null,
				allStreamsEnded: null,
				reported,
			};
		},
		setStopped: cannotBeStopped(kind),
		leave(way) {
			switch (way) {
				case "cancel":
					return onPage(
						label,
						"cancel",
						page.evaluate(() => window.xmoqOwn.cancelSubscription()),
					);
				case "close":
					return onPage(
						label,
						"disconnect",
						page.evaluate(() => window.xmoq.disconnect()),
					);
				case "kill":
					return page.context().close();
			}
		},
	};
}

/** Connects a subscriber and subscribes to the track. */
export async function startSubscriber(
	kind: Client,
	label: string,
	run: ClientRun,
	plan: SubscriberPlan = {},
): Promise<Subscriber> {
	if (isBrowserClient(kind)) {
		return startBrowserSubscriber(kind, label, run, plan);
	}
	const { subscribed, ...subscriber } = startRustSubscriber(
		kind,
		label,
		run,
		plan,
	);
	await subscribed;
	return subscriber;
}
