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
	pollFor,
	type RunContext,
	type RustClient,
	STEP_TIMEOUT_MS,
	spawnRustClient,
	TRACK,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageFetch,
	PageFetchReport,
	PageFetchServing,
	PageProperty,
	PageReceivedObject,
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

/** What a publisher is started with, where the defaults of a run do not do. */
export interface PublisherPlan {
	/** Tells the publishers of a run apart in the log; `pub:<kind>` when left out. */
	label?: string;
	/**
	 * The Group of its first Object, 0 when left out. The moqtail clients
	 * cannot start elsewhere: they send Group 0 and nothing else.
	 */
	firstGroup?: number;
	/**
	 * What its Objects carry, the payload of the run when left out. Two
	 * publishers with different payloads show a subscriber which of them an
	 * Object came from.
	 */
	payloadHex?: string;
	/**
	 * The own clients: the client announces the namespace and sends no
	 * PUBLISH. It serves the track when it is asked for it with a SUBSCRIBE:
	 * "at-once", or, own-rust only, "on-line", where it holds its answer
	 * back until `serve()`, so that something else can happen to the track
	 * while the relay waits for it.
	 */
	servesWhenAsked?: "at-once" | "on-line";
	/**
	 * own-browser, with `servesWhenAsked`: what the page does with a
	 * Standalone FETCH for the track. "refuse" when left out.
	 */
	servesFetches?: PageFetchServing;
	/**
	 * own-rust: the client connects, announces the namespace, and holds its
	 * PUBLISH back until `publish()`. WHY: what takes time on a loaded
	 * machine, the start of the process and its connection, is then done
	 * before a scenario opens a window the PUBLISH has to arrive in.
	 */
	publishesOnLine?: boolean;
	/**
	 * The own clients: the Object Properties every Object it sends carries
	 * (§11.2.1.2), in ascending order of type as they go onto the wire. None
	 * when left out. The moqtail clients cannot send any.
	 */
	properties?: PageProperty[];
	/**
	 * The own clients: the Publisher Priority in the Subgroup Headers it
	 * sends, or "default" for headers that leave it out (§11.4.2: the
	 * Subgroup then has the track's default, 128 where the publisher gave
	 * none). 0 when left out, as the clients did before they could be told.
	 */
	publisherPriority?: PublisherPriority;
}

export type PublisherPriority = number | "default";

/** The clients that take no line of the driver before a PUBLISH or an answer: every one but own-rust. */
const NO_LINES: Pick<
	Publisher,
	"publish" | "asked" | "serve" | "servedFetchEnds"
> = {
	servedFetchEnds: () =>
		Promise.reject(new Error("only an own page that serves a track says")),
	publish: () =>
		Promise.reject(new Error("only own-rust holds a PUBLISH back")),
	asked: () => Promise.reject(new Error("only own-rust holds an answer back")),
	serve: () => Promise.reject(new Error("only own-rust holds an answer back")),
};

export interface Publisher {
	kind: Client;
	label: string;
	/** What every Object it sends carries. */
	payloadHex: string;
	/** The moqtail clients send Object 0 of Group 0 and nothing else. */
	sendsSeveral: boolean;
	/** Sends Object 0 of the next Group on a stream of its own, finishes the stream, and resolves with the Group. */
	send(): Promise<number>;
	/** Sends `count` Objects like `send`, one after the other, and resolves with their Groups. */
	sendBatch(count: number): Promise<number[]>;
	/**
	 * own-rust: sends Object 0 of the next Group with the status END_OF_TRACK
	 * and no payload, as `send` sends one, and resolves with the Group.
	 */
	sendEndOfTrack?(): Promise<number>;
	/**
	 * own-rust: sends the next Group on a stream that ends with a FIN, then
	 * the same Subgroup on a second such stream with one Object more
	 * (`longer`) or one fewer (`subgroup-again-longer`,
	 * `subgroup-again-shorter`), and resolves with the Group once both were
	 * sent.
	 */
	sendSubgroupAgain?(longer: boolean): Promise<number>;
	/**
	 * own-browser: sends the next Group as `objects` Objects of `bytes` bytes
	 * each, on one stream, and resolves with the Group.
	 */
	sendLargeGroup?(objects: number, bytes: number): Promise<number>;
	/**
	 * own-browser: the Group `writeToSubgroup` writes to next, which is the
	 * next Group; taken once for all the Objects of it.
	 */
	takeGroup?(): number;
	/** own-browser: writes one Object to a subgroup stream that is opened at its first Object and left open. */
	writeToSubgroup?(
		group: number,
		to: { subgroup: number; publisherPriority: number },
		object: number,
		payloadHex: string,
		properties: PageProperty[],
	): Promise<void>;
	/** own-browser: finishes the streams `writeToSubgroup` opened for the Group. */
	closeSubgroups?(group: number): Promise<void>;
	/**
	 * own-rust: ends its subscription with PUBLISH_DONE and publishes the
	 * track anew on the same session, under the same Track Alias or the next.
	 * The PUBLISH_DONE has the Stream Count of a publisher that cannot count
	 * (2^62 - 1), so that the relay is still waiting for streams of the first
	 * subscription when the PUBLISH comes.
	 *
	 * `peerHasReadPublishDone` is awaited between the two: the PUBLISH is
	 * sent when it resolves. WHY: the two messages are on two streams,
	 * between which QUIC keeps no order, and a relay that reads the PUBLISH
	 * first has to refuse it (`DUPLICATE_SUBSCRIPTION`, §5.1). A publisher
	 * has no way to know; a driver that reads the relay's log has.
	 *
	 * Resolves when the client is ready to send again, with the alias of the
	 * new PUBLISH, how long after `peerHasReadPublishDone` the PUBLISH was
	 * established, and how, and how long after it, the relay ended its
	 * direction of the first PUBLISH stream.
	 */
	publishAnew(
		sameAlias: boolean,
		peerHasReadPublishDone: () => Promise<void>,
	): Promise<{
		trackAlias: number;
		publishedAfterMs: number;
		peerEnd: string;
		peerEndAfterMs: number;
	}>;
	/**
	 * own-browser: sends Object 0 of a Group on a new stream, with this
	 * Publisher Priority, whether the page sent the Group before or not. The
	 * other clients send each Group once, with `send`.
	 *
	 * For a stream the relay is expected to stop. Resolves as well when the
	 * stream was stopped before the Object was written, which a relay that
	 * decides on the Subgroup Header can do; a caller reads the relay's log
	 * for what the relay did. WHY not `send` for it: a page whose write lost
	 * against the STOP_SENDING failed there, 3 runs of 150 under load.
	 */
	sendAgain(group: number, publisherPriority: PublisherPriority): Promise<void>;
	/**
	 * own-rust: resolves with the code with which the subscriber ended its
	 * PUBLISH, as "0x1", once it has. The client ends with an error then.
	 *
	 * WHY the code alone: a relay that cancels a PUBLISH resets its direction
	 * of the request stream and stops the publisher's with the same code, and
	 * which of the two the client sees first differs from run to run.
	 *
	 * To be called when the relay has seen the client's session end. WHY: the
	 * client prints the line as its process ends, and the process ends when
	 * its stdin does, so this closes the stdin; on a client whose PUBLISH is
	 * still there, that would end the track in order instead.
	 */
	publishCancelledWith(): Promise<string>;
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
	/**
	 * How the subscriber ended its direction of the request stream after
	 * this client's PUBLISH_DONE: "FIN", or what else the client says it saw.
	 * `undefined` for a client that sent none or does not say (moqtail).
	 */
	peerEndAfterPublishDone(): Promise<string | undefined>;
	/**
	 * Resolves once the client has a subscription to send on: at once, but
	 * for a publisher that serves when asked and has not answered yet.
	 */
	serving(): Promise<void>;
	/** own-browser that serves when asked: how each FETCH it accepted ended, the library's `ServedFetchEnd` as JSON. */
	servedFetchEnds(): Promise<string[]>;
	/** A publisher that holds its PUBLISH back (`publishesOnLine`): sends it, and resolves when it is accepted. */
	publish(): Promise<void>;
	/** A publisher that holds its answer back (`servesWhenAsked: "on-line"`): resolves when the relay's SUBSCRIBE has come to it. */
	asked(): Promise<void>;
	/** That publisher: answers the SUBSCRIBE, and resolves when it has. To be called after `asked()`. */
	serve(): Promise<void>;
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
	plan: PublisherPlan,
): Promise<Publisher> {
	if (plan.publisherPriority !== undefined && kind !== "own-rust") {
		throw new Error(
			`${label} cannot be told a Publisher Priority: only the own clients can`,
		);
	}
	if (plan.properties !== undefined && kind !== "own-rust") {
		throw new Error(
			`${label} cannot send Object Properties: only the own clients can`,
		);
	}
	const firstGroup = plan.firstGroup ?? 0;
	if (firstGroup !== 0 && kind !== "own-rust") {
		throw new Error(
			`${label} cannot start at Group ${firstGroup}: the client sends Group 0 and nothing else`,
		);
	}
	const payloadHex = plan.payloadHex ?? run.payloadHex;
	const { servesWhenAsked, publishesOnLine = false } = plan;
	if (servesWhenAsked !== undefined && publishesOnLine) {
		throw new Error(`${label} cannot both serve when asked and send a PUBLISH`);
	}
	if (
		(servesWhenAsked !== undefined || publishesOnLine) &&
		kind !== "own-rust"
	) {
		throw new Error(
			`${label} cannot serve a track when asked, or hold its PUBLISH back: only own-rust can`,
		);
	}
	const client = spawnRustClient(
		run.context,
		label,
		kind,
		servesWhenAsked === undefined ? "publish" : "publish-namespace",
		TRACK,
		[
			"--payload-hex",
			payloadHex,
			...(firstGroup === 0 ? [] : ["--first-group", String(firstGroup)]),
			...(servesWhenAsked === "on-line" ? ["--serve-on-line"] : []),
			...(publishesOnLine ? ["--publish-on-line"] : []),
			...(plan.publisherPriority === undefined
				? []
				: ["--publisher-priority", String(plan.publisherPriority)]),
			...(plan.properties ?? []).flatMap((property) => [
				"--object-property",
				rustPropertyText(property, "="),
			]),
			"--timeout-secs",
			RUST_CLIENT_TIMEOUT_SECS,
		],
	);
	const PUBLISHED = /^PUBLISHED /;
	await client.waitForLine(
		servesWhenAsked !== undefined
			? /^NAMESPACE_PUBLISHED/
			: publishesOnLine
				? /^READY_TO_PUBLISH$/
				: PUBLISHED,
	);
	const SERVING = /^SERVING /;
	const onLineOnly = (what: string): Error =>
		new Error(`${label} was not started to ${what} on a line of the driver`);
	let nextGroup = firstGroup;
	const sent = async (): Promise<number> => {
		const group = nextGroup;
		nextGroup += 1;
		await client.waitForLine(new RegExp(`^SENT group=${group} `));
		return group;
	};
	return {
		kind,
		label,
		payloadHex,
		sendsSeveral: kind === "own-rust",
		send() {
			client.child.stdin.write("send\n");
			return sent();
		},
		sendEndOfTrack() {
			client.child.stdin.write("end-of-track\n");
			return sent();
		},
		async sendSubgroupAgain(longer) {
			client.child.stdin.write(
				longer ? "subgroup-again-longer\n" : "subgroup-again-shorter\n",
			);
			const group = await sent();
			const twice = new RegExp(`^SENT group=${group} `);
			await pollFor(
				() => (client.matches(twice).length >= 2 ? true : undefined),
				`${label} to send Group ${group} on its second stream`,
			);
			return group;
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
		peerEndAfterPublishDone: () =>
			Promise.resolve(
				client.matches(/^PUBLISH_DONE_PEER_ENDED how=(.*)$/).at(-1)?.[1],
			),
		sendAgain: () => Promise.reject(new Error(`${kind} sends each Group once`)),
		async publishCancelledWith() {
			client.child.stdin.end();
			return captureGroup(
				await client.waitForLine(
					/^Error: the subscriber ended the subscription: (?:reset|STOP_SENDING), code (\S+)$/,
				),
				1,
			);
		},
		async publishAnew(sameAlias, peerHasReadPublishDone) {
			const PUBLISHED = /^PUBLISHED track_alias=(\d+) /;
			const PEER_ENDED = /^PUBLISH_DONE_PEER_ENDED how=(.*)$/;
			const before = {
				published: client.matches(PUBLISHED).length,
				peerEnded: client.matches(PEER_ENDED).length,
			};
			client.child.stdin.write("publish-done\n");
			await client.waitForLine(/^PUBLISH_DONE_SENT$/);
			await peerHasReadPublishDone();
			const started = performance.now();
			client.child.stdin.write(
				sameAlias ? "publish-anew-same-alias\n" : "publish-anew\n",
			);
			// A client whose PUBLISH is refused ends with a line that says so;
			// the run then fails with that line and not at a time limit.
			let exited = false;
			void client.exited.then(() => {
				exited = true;
			});
			const orItsError = (pattern: RegExp, already: number) => () => {
				const found = client.matches(pattern)[already];
				if (found !== undefined || !exited) return found;
				const error = client.matches(/^Error: (.*)$/).at(-1)?.[1];
				throw new Error(
					`${label} ended before it had published the track anew: ${error ?? "it printed no error"}`,
				);
			};
			const published = await pollFor(
				orItsError(PUBLISHED, before.published),
				`${label} to print PUBLISHED for its new PUBLISH`,
			);
			const publishedAfterMs = Math.round(performance.now() - started);
			// The client prints the line of the first subscription's end last,
			// and reads its next line after that.
			const peerEnded = await pollFor(
				orItsError(PEER_ENDED, before.peerEnded),
				`${label} to say how the relay ended the first PUBLISH stream`,
			);
			return {
				trackAlias: Number(captureGroup(published, 1)),
				publishedAfterMs,
				peerEnd: captureGroup(peerEnded, 1),
				peerEndAfterMs: Math.round(performance.now() - started),
			};
		},
		async serving() {
			if (servesWhenAsked !== undefined) await client.waitForLine(SERVING);
		},
		async publish() {
			if (!publishesOnLine) throw onLineOnly("publish");
			client.child.stdin.write("publish\n");
			await client.waitForLine(PUBLISHED);
		},
		async asked() {
			if (servesWhenAsked !== "on-line") throw onLineOnly("serve");
			await client.waitForLine(/^ASKED request_id=\d+$/);
		},
		async serve() {
			if (servesWhenAsked !== "on-line") throw onLineOnly("serve");
			client.child.stdin.write("serve\n");
			await client.waitForLine(SERVING);
		},
		servedFetchEnds: NO_LINES.servedFetchEnds,
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

/**
 * An own page that announces the namespace and publishes nothing: a
 * publisher that refuses when a relay asks it for a track of the namespace
 * (`announceNamespace` in xmoq-page.ts).
 */
export async function startRefusingAnnouncer(
	label: string,
	run: ClientRun,
): Promise<{ label: string; announce(): Promise<void> }> {
	const page = await openConnectedPage(run.context, label, "own-browser");
	const announce = (): Promise<void> =>
		onPage(
			label,
			"announce the namespace",
			page.evaluate((ns) => window.xmoqOwn.announceNamespace(ns), NAMESPACE),
		);
	await announce();
	return { label, announce };
}

async function startBrowserPublisher(
	kind: BrowserClient,
	label: string,
	run: ClientRun,
	plan: PublisherPlan,
): Promise<Publisher> {
	if (plan.publishesOnLine === true || plan.servesWhenAsked === "on-line") {
		throw new Error(
			`${label} cannot hold a PUBLISH or an answer back until it is told: only own-rust can`,
		);
	}
	const servesWhenAsked = plan.servesWhenAsked !== undefined;
	if (servesWhenAsked && kind !== "own-browser") {
		throw new Error(
			`${label} cannot serve a track when asked: only the own clients can`,
		);
	}
	const page = await openConnectedPage(run.context, label, kind);
	const names = [NAMESPACE, TRACK] satisfies [string, string];
	if (servesWhenAsked) {
		await onPage(
			label,
			"announce the namespace",
			page.evaluate(
				([ns, track, fetches, unsent]) =>
					window.xmoqOwn.serveTrack(ns, track, fetches, unsent),
				[
					...names,
					plan.servesFetches ?? "refuse",
					plan.payloadHex ?? run.payloadHex,
				] satisfies [string, string, PageFetchServing, string],
			),
		);
	} else {
		await onPage(
			label,
			"publish the track",
			page.evaluate(
				([ns, track]) => window.xmoq.publishTrack(ns, track),
				names,
			),
		);
	}
	const payloadHex = plan.payloadHex ?? run.payloadHex;
	const kill = (): Promise<void> => page.context().close();
	if (kind === "moqtail-browser") {
		if (plan.properties !== undefined) {
			throw new Error(
				`${label} cannot send Object Properties: only the own clients can`,
			);
		}
		if ((plan.firstGroup ?? 0) !== 0) {
			throw new Error(
				`${label} cannot start at Group ${plan.firstGroup}: the page sends Group 0 and nothing else`,
			);
		}
		let sentGroup0 = false;
		return {
			kind,
			label,
			payloadHex,
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
			peerEndAfterPublishDone: () => Promise.resolve(undefined),
			sendAgain: () =>
				Promise.reject(new Error(`${kind} sends each Group once`)),
			publishCancelledWith: () =>
				Promise.reject(
					new Error(`${kind} does not say what ended its PUBLISH`),
				),
			publishAnew: () =>
				Promise.reject(new Error(`${kind} cannot publish its track anew`)),
			serving: () => Promise.resolve(),
			...NO_LINES,
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
	let nextGroup = plan.firstGroup ?? 0;
	let leftOpen = 0;
	const pagePriority =
		plan.publisherPriority === "default" ? null : (plan.publisherPriority ?? 0);
	const sendIn = async (finish: boolean): Promise<number> => {
		const group = nextGroup;
		nextGroup += 1;
		if (!finish) leftOpen += 1;
		await onPage(
			label,
			`send Group ${group}`,
			page.evaluate(
				([g, hex, fin, properties, priority]) =>
					window.xmoqOwn.sendObjectIn(g, hex, fin, properties, priority),
				[
					String(group),
					payloadHex,
					finish,
					plan.properties ?? [],
					pagePriority,
				] satisfies [string, string, boolean, PageProperty[], number | null],
			),
		);
		return group;
	};
	return {
		kind,
		label,
		payloadHex,
		sendsSeveral: true,
		send: () => sendIn(true),
		async sendBatch(count) {
			const groups: number[] = [];
			for (let index = 0; index < count; index++) {
				groups.push(await sendIn(true));
			}
			return groups;
		},
		async sendLargeGroup(objects, bytes) {
			const group = nextGroup;
			nextGroup += 1;
			await onPage(
				label,
				`send a Group of ${objects} Objects`,
				page.evaluate(
					([groupId, count, length]) =>
						window.xmoqOwn.sendLargeGroup(groupId, count, length),
					[String(group), objects, bytes] satisfies [string, number, number],
				),
			);
			return group;
		},
		takeGroup() {
			const group = nextGroup;
			nextGroup += 1;
			return group;
		},
		writeToSubgroup: (group, to, object, hex, properties) =>
			onPage(
				label,
				`write Object ${object} to Subgroup ${to.subgroup}`,
				page.evaluate(
					([g, s, priority, o, payload, with_]) =>
						window.xmoqOwn.writeToSubgroup(g, s, priority, o, payload, with_),
					[
						String(group),
						String(to.subgroup),
						to.publisherPriority,
						String(object),
						hex,
						properties,
					] satisfies [string, string, number, string, string, PageProperty[]],
				),
			),
		closeSubgroups: (group) =>
			onPage(
				label,
				"finish the subgroup streams",
				page.evaluate(([g]) => window.xmoqOwn.closeSubgroups(g), [
					String(group),
				] satisfies [string]),
			),
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
		async sendAgain(group, publisherPriority) {
			await onPage(
				label,
				`send Group ${group} again`,
				page.evaluate(
					([g, hex, priority]) =>
						window.xmoqOwn.sendObjectInWhatMayBeStopped(g, hex, priority),
					[
						String(group),
						payloadHex,
						publisherPriority === "default" ? null : publisherPriority,
					] satisfies [string, string, number | null],
				),
			);
		},
		publishCancelledWith: () =>
			Promise.reject(new Error(`${kind} does not say what ended its PUBLISH`)),
		publishAnew: () =>
			Promise.reject(new Error(`${kind} cannot publish its track anew`)),
		async serving() {
			if (!servesWhenAsked) return;
			await onPage(
				label,
				"accept the SUBSCRIBE it is asked with",
				page.evaluate(() => window.xmoqOwn.servingTrack()),
			);
		},
		...NO_LINES,
		servedFetchEnds: () =>
			onPage(
				label,
				"say how the FETCHes it served ended",
				page.evaluate(() => window.xmoqOwn.servedFetchEnds()),
			),
		async peerEndAfterPublishDone() {
			const reported = await onPage(
				label,
				"report how the subscriber ended its direction",
				page.evaluate(() => window.xmoqOwn.peerStreamEnd()),
			);
			if (reported === "null") return undefined;
			return reported === '{"type":"fin"}' ? "FIN" : reported;
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
	plan: PublisherPlan = {},
): Promise<Publisher> {
	const label = plan.label ?? `pub:${kind}`;
	return isBrowserClient(kind)
		? startBrowserPublisher(kind, label, run, plan)
		: startRustPublisher(kind, label, run, plan);
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
	/**
	 * The payload of every Object of the Group the client has got so far:
	 * more than one where the Group came to it twice. Only the own clients
	 * can tell.
	 */
	payloads(group: number): Promise<string[]>;
	/**
	 * The Object Properties of Object 0 of the Group once it arrived, in the
	 * order they were on the wire. Only the own clients can tell.
	 */
	properties(group: number): Promise<PageProperty[]>;
	/**
	 * What the client can say when an Object does not come: whether its
	 * session is open, and how many Objects it has. Only the own clients can tell.
	 */
	state(): Promise<string>;
	/**
	 * own-browser: sends a FETCH and resolves once it is over, with what the
	 * page saw of it. The FETCH is cancelled after `timeoutMs`.
	 */
	fetch(fetch: PageFetch, timeoutMs: number): Promise<PageFetchReport>;
	/** own-browser: how many Objects of the Group its subscription has delivered so far. */
	objectsOfGroup?(group: number): Promise<number>;
	/** own-browser: the Objects of the Group its subscription has delivered so far, in the order they came. */
	receivedOfGroup?(group: number): Promise<PageReceivedObject[]>;
	/**
	 * own-browser: sends the FETCH, reads nothing of its fetch stream, and
	 * gives it up after `cancelAfterMs`. Resolves with the End Location of
	 * the FETCH_OK and how the FETCH ended.
	 */
	fetchAndLeaveUnread?(
		fetch: PageFetch,
		cancelAfterMs: number,
	): Promise<{ endLocation: [string, string]; ended: string }>;
	/** Resolves once the subscription has ended. Only the own clients can tell. */
	outcome(timeoutMs?: number): Promise<SubscriptionOutcome>;
	leave(way: LeaveWay): Promise<void>;
	/**
	 * Stops the process, so that it takes nothing and answers nothing, or lets
	 * it run again. Only a Rust client can be stopped.
	 */
	setStopped(stopped: boolean): Promise<void>;
}

function isPageProperty(value: unknown): value is PageProperty {
	if (typeof value !== "object" || value === null) return false;
	if (!("type" in value) || typeof value.type !== "string") return false;
	return (
		("varint" in value && typeof value.varint === "string") ||
		("bytesHex" in value && typeof value.bytesHex === "string")
	);
}

/**
 * A property as the own Rust client takes it in `--object-property` (with
 * "=") and prints it in its `OBJECT` line (with ":"): the type in decimal,
 * the value of an even type in decimal and that of an odd type in hexadecimal.
 */
function rustPropertyText(property: PageProperty, separator: string): string {
	return `${property.type}${separator}${"varint" in property ? property.varint : property.bytesHex}`;
}

/** The `properties=` field of the own Rust client's `OBJECT` line. */
function parseRustProperties(printed: string, label: string): PageProperty[] {
	if (printed === "none") return [];
	return printed.split(",").map((text): PageProperty => {
		const [type, value, ...rest] = text.split(":");
		if (
			type === undefined ||
			value === undefined ||
			rest.length > 0 ||
			!/^\d+$/.test(type)
		) {
			throw new Error(
				`${label} printed a property that cannot be read: ${text}`,
			);
		}
		// §1.4.3: an even type has one integer, an odd type bytes.
		return BigInt(type) % 2n === 0n
			? { type, varint: value }
			: { type, bytesHex: value };
	});
}

function cannotTell(
	kind: Client,
	what = "how a subscription ended",
): () => Promise<never> {
	return () => Promise.reject(new Error(`${kind} does not report ${what}`));
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
	/**
	 * own-rust: once its SUBSCRIBE is accepted it asks for this
	 * SUBGROUP_DELIVERY_TIMEOUT with a REQUEST_UPDATE (transport §10.9), and
	 * it counts as subscribed when the relay has answered REQUEST_OK.
	 */
	updatesSubgroupTimeoutMs?: number;
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
		...(plan.updatesSubgroupTimeoutMs === undefined
			? []
			: [
					"--update-subgroup-timeout-ms",
					String(plan.updatesSubgroupTimeoutMs),
				]),
	]);
	if (plan.updatesSubgroupTimeoutMs !== undefined && kind !== "own-rust") {
		throw new Error(`${kind} sends no REQUEST_UPDATE`);
	}
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
			payloads: cannotTell(kind),
			properties: cannotTell(kind, "Object Properties"),
			state: cannotTell(kind, "its state"),
			fetch: cannotTell(kind, "a FETCH: it sends none"),
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
		subscribed:
			plan.updatesSubgroupTimeoutMs === undefined
				? client.waitForLine(/^SUBSCRIBED /)
				: client.waitForLine(/^UPDATE_ACCEPTED$/),
		object: (group) =>
			objectOf(new RegExp(`^OBJECT group=${group} .*payload_hex=([0-9a-f]*)$`)),
		async properties(group) {
			const printed = await objectOf(
				new RegExp(`^OBJECT group=${group} .* properties=(\\S+) payload_len=`),
			);
			return parseRustProperties(printed, label);
		},
		fetch: cannotTell(kind, "a FETCH: it sends none"),
		state: () =>
			Promise.resolve(
				`${client.matches(/^OBJECT /).length} OBJECT lines, ${client.matches(/^STREAM_ABORTED /).length} STREAM_ABORTED, last line: ${client.matches(/^.*$/).at(-1)?.[0] ?? "none"}`,
			),
		payloads: (group) =>
			Promise.resolve(
				client
					.matches(
						new RegExp(`^OBJECT group=${group} .*payload_hex=([0-9a-f]*)$`),
					)
					.map((match) => captureGroup(match, 1)),
			),
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
			payloads: cannotTell(kind),
			properties: cannotTell(kind, "Object Properties"),
			state: cannotTell(kind, "its state"),
			fetch: cannotTell(kind, "a FETCH: it sends none"),
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
		async payloads(group) {
			const reported: unknown = JSON.parse(
				await onPage(
					label,
					`report its Objects of Group ${group}`,
					page.evaluate(
						(g) => window.xmoqOwn.payloadsOfGroup(g),
						String(group),
					),
				),
			);
			if (
				!Array.isArray(reported) ||
				!reported.every((item): item is string => typeof item === "string")
			) {
				throw new Error(
					`${label} reported no list of payloads: ${String(reported)}`,
				);
			}
			return reported;
		},
		state: () =>
			onPage(
				label,
				"report its state",
				page.evaluate(() => window.xmoqOwn.state()),
			),
		objectsOfGroup: (group) =>
			onPage(
				label,
				"count the Objects of a Group",
				page.evaluate(([groupId]) => window.xmoqOwn.objectsOfGroup(groupId), [
					String(group),
				] satisfies [string]),
			),
		async receivedOfGroup(group) {
			const received: PageReceivedObject[] = JSON.parse(
				await onPage(
					label,
					"list the Objects of a Group",
					page.evaluate(([g]) => window.xmoqOwn.receivedOfGroup(g), [
						String(group),
					] satisfies [string]),
				),
			);
			return received;
		},
		fetchAndLeaveUnread: (fetch, cancelAfterMs) =>
			onPage(
				label,
				`send a ${fetch.type} FETCH, leave it unread and give it up`,
				page.evaluate(
					([asked, after]) => window.xmoqOwn.fetchAndLeaveUnread(asked, after),
					[fetch, cancelAfterMs] satisfies [PageFetch, number],
				),
				cancelAfterMs + STEP_TIMEOUT_MS,
			),
		async fetch(fetch, timeoutMs) {
			const reported: PageFetchReport = JSON.parse(
				await onPage(
					label,
					`send a ${fetch.type} FETCH and report it`,
					page.evaluate(
						([asked, limit]) => window.xmoqOwn.fetchRange(asked, limit),
						[fetch, timeoutMs] satisfies [PageFetch, number],
					),
					timeoutMs + STEP_TIMEOUT_MS,
				),
			);
			return reported;
		},
		async properties(group) {
			const reported: unknown = JSON.parse(
				await onPage(
					label,
					`report the properties of its Object of Group ${group}`,
					page.evaluate(
						(g) => window.xmoqOwn.propertiesOfGroup(g),
						String(group),
					),
				),
			);
			if (!Array.isArray(reported) || !reported.every(isPageProperty)) {
				throw new Error(
					`${label} reported no list of properties: ${JSON.stringify(reported)}`,
				);
			}
			return reported;
		},
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
