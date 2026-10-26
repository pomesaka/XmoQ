// The meeting page of the demo: a participant of the meeting of the room
// `?room=NAME` ("demo" when left out). It asks the room of the meeting for
// a seat (`/meetings/<name>/seats`, at most 49, `packages/room/src/worker.ts`),
// publishes its own picture and voice under `xmoq/meet/<room>/<id>`, and
// shows the others it receives with `createMeeting` of `@xmoq/meeting`: the
// sound of all of them, and the small video of the first 8 who came.
//
// The sound is the microphone; `?audio=none` sends none. The picture is
// one the page paints, small (320x180), with its id on it.

import { type ChatRoom, roomFields } from "@xmoq/chat";
import {
	createAudioTrackEncoder,
	createVideoTrackEncoder,
	keepPublishing,
} from "@xmoq/media";
import {
	AGENT_ROOT,
	type AgentBridge,
	allowRoot,
	chooseRoot,
	createAgentBridge,
	createAgentCall,
	createMeeting,
	keepAttending,
	openAgentFolder,
	rememberedRoot,
} from "@xmoq/meeting";
import type { GoAwayNotice } from "@xmoq/moqt";
import { createCatalogPublisher } from "@xmoq/msf";
import {
	type KeptSeat,
	keepSeat,
	type MeetingRoom,
	meetingRoom,
	takeSeat,
	tokenOfARoom,
} from "@xmoq/room-client";
import { keepFollowingTranscript, TRANSCRIPT_PREFIX } from "@xmoq/transcript";
import { startChat } from "./room-chat.ts";
import {
	AUDIO_TRACK,
	type Broadcast,
	connectToRelay,
	DECODER_CODEC,
	ENCODER_CODEC,
	every,
	SAMPLE_RATE,
	say,
	TRACK,
	tokenOf,
	why,
} from "./shared.ts";

declare const XMOQ_DEMO: { roomsUrl: string | null };

/** The size each participant sends at: the small version the meeting receives of 8. */
const WIDTH = 320;
const HEIGHT = 180;
const FRAMES_A_SECOND = 15;
/** How long the page waits between two tries at the room or the relay. */
const AGAIN_MS = 2000;

const room = new URLSearchParams(location.search).get("room") ?? "demo";
/** A random id, as the room takes it: 16 hex digits, no `/` and no `*`. */
const self = Array.from(crypto.getRandomValues(new Uint8Array(8)), (byte) =>
	byte.toString(16).padStart(2, "0"),
).join("");
const utf8 = new TextEncoder();
const fields = (namespace: string[]): Uint8Array[] =>
	namespace.map((field) => utf8.encode(field));
const mine = ["xmoq", "meet", room, self];

/** What a seat of the room gives: where the relay is, and the two tokens as `@xmoq/moqt` carries them. */
interface Seat {
	relay: Broadcast;
	publish: Uint8Array | undefined;
	subscribe: Uint8Array | undefined;
}

/**
 * Where to connect after `goAway`: §10.4 has "The client MUST use this URI
 * for the new session if provided", with the room's certificate hash, as
 * the watching page does. The room's relay where it names none.
 */
const movedTo = (relay: Broadcast, goAway?: GoAwayNotice): Broadcast =>
	goAway === undefined || goAway.newSessionUri === ""
		? relay
		: { ...relay, relayUrl: goAway.newSessionUri };

/** What the page shows of itself, and a script that looks at it reads. */
const state: {
	self: string;
	room: string;
	seat: string;
	publishing: string;
	receiving: string;
	/** Of each other there: the frames drawn and the pieces of sound received. */
	others: Record<string, { frames: number; audio: number }>;
	/** The latest 5 of what could not be received. */
	failed: string[];
} = {
	self,
	room,
	seat: "asking",
	publishing: "not yet",
	receiving: "not yet",
	others: {},
	failed: [],
};
const show = (): void => {
	document.body.dataset.meet = JSON.stringify(state);
	say(
		`room “${room}” as ${self}: ${state.seat}; publishing: ${state.publishing}; receiving: ${state.receiving}; others: ${Object.keys(state.others).join(", ") || "none"}${state.failed.length === 0 ? "" : `\nnot received: ${state.failed.join("; ")}`}`,
	);
};

/**
 * The room of the meeting, asked over HTTP, and a seat from it whenever one
 * is asked for: taken once and kept by renewing it (`keepSeat` of
 * `@xmoq/room-client`), and taken again with the next connection where it
 * was lost. WHY asked for every connection: a token is good for 600 s, and
 * a renewal gives new ones.
 */
function seats(): {
	seat(): Promise<Seat>;
	token(
		role: "pub" | "sub",
		namespace: string[],
	): Promise<Uint8Array | undefined>;
	/** The prefix the room says a transcriber listens under for its meeting; null without one. */
	meetingTranscript(): Promise<string[] | null>;
} {
	const roomsUrl = XMOQ_DEMO.roomsUrl;
	if (roomsUrl === null) {
		throw new Error("no rooms: a meeting takes its seat from the room");
	}
	const address = { rooms: roomsUrl, name: room };
	/** What the room said of the meeting, asked for again after a failure. */
	let described: Promise<MeetingRoom> | undefined;
	const meeting = (): Promise<MeetingRoom> => {
		described ??= meetingRoom(address).catch((error: unknown) => {
			described = undefined;
			throw error;
		});
		return described;
	};
	/**
	 * The seat, taken once for every connection that asks at the same time:
	 * a second ask for the same id would be refused as taken.
	 */
	let kept: Promise<KeptSeat> | undefined;
	const keptSeat = (): Promise<KeptSeat> => {
		if (kept !== undefined) return kept;
		const taking = takeSeat(address, self).then((answer): KeptSeat => {
			switch (answer.type) {
				case "full":
					state.seat = `the meeting is full: it takes ${answer.most}`;
					document.body.dataset.full = "true";
					show();
					throw new Error(state.seat);
				case "refused":
					state.seat = `the room refused the seat: ${answer.why}`;
					show();
					throw new Error(state.seat);
				case "gone":
					throw new Error("the room gave no seat");
				case "seat": {
					const keeping = keepSeat(address, answer.seat);
					void keeping.lost.then((lost) => {
						if (kept === taking) kept = undefined;
						state.seat = `the seat was lost (${lost.type}): taken again with the next connection`;
						show();
					});
					return keeping;
				}
			}
		});
		kept = taking;
		taking.catch(() => {
			if (kept === taking) kept = undefined;
		});
		return taking;
	};
	async function seat(): Promise<Seat> {
		const { relay } = await meeting();
		if (relay === null) {
			throw new Error("the room names no relay, where its meeting would be");
		}
		const keeping = await keptSeat();
		state.seat = "seated";
		show();
		return {
			relay: {
				relayUrl: relay.url,
				certSha256Hex: relay.certSha256Hex,
				namespace: [],
			},
			publish: tokenOf(keeping.seat.publishToken),
			subscribe: tokenOf(keeping.seat.subscribeToken),
		};
	}
	return {
		// The chat's tokens, and the transcript's, from the meeting's room.
		async token(role, namespace) {
			return tokenOf(await tokenOfARoom("meetings", address, role, namespace));
		},
		seat,
		async meetingTranscript() {
			return (await meeting()).transcript;
		},
	};
}

/** Not in the DOM library of TypeScript yet: a track of a `MediaStream` as a stream of frames (Chromium has it). */
declare class MediaStreamTrackProcessor {
	constructor(init: { track: MediaStreamTrack });
	readonly readable: ReadableStream<VideoFrame | AudioData>;
}

async function fromMicrophone(
	onData: (data: AudioData) => void,
): Promise<void> {
	const media = await navigator.mediaDevices.getUserMedia({
		audio: { sampleRate: SAMPLE_RATE, channelCount: 1 },
	});
	const [track] = media.getAudioTracks();
	if (track === undefined) throw new Error("the microphone gave no track");
	const reader = new MediaStreamTrackProcessor({ track }).readable.getReader();
	for (;;) {
		const next = await reader.read();
		if (next.done) return;
		if (next.value instanceof AudioData) onData(next.value);
		else next.value.close();
	}
}

/** The tile of a participant: a canvas, made when its first frame comes. */
function tileOf(id: string): HTMLCanvasElement {
	const found = document.querySelector<HTMLCanvasElement>(
		`canvas[data-id="${id}"]`,
	);
	if (found !== null) return found;
	const tile = document.createElement("canvas");
	tile.width = WIDTH;
	tile.height = HEIGHT;
	tile.dataset.id = id;
	tile.title = id;
	document.getElementById("tiles")?.append(tile);
	return tile;
}

/**
 * The sound of each participant, played one piece after the other once the
 * page was clicked (`sound`): a page cannot start an `AudioContext` before.
 * WHY no playout buffer as the watching page has: a meeting's sound is
 * many tracks that each speak now and then, and a piece that comes late is
 * played late rather than waited for.
 */
const audio: { context?: AudioContext; next: Map<string, number> } = {
	next: new Map(),
};
document.getElementById("sound")?.addEventListener("click", () => {
	audio.context ??= new AudioContext({ sampleRate: SAMPLE_RATE });
	void audio.context.resume();
});
function play(id: string, data: AudioData): void {
	const context = audio.context;
	if (context === undefined || context.state !== "running") return;
	const buffer = context.createBuffer(
		data.numberOfChannels,
		data.numberOfFrames,
		data.sampleRate,
	);
	for (let channel = 0; channel < data.numberOfChannels; channel++) {
		const samples = new Float32Array(data.numberOfFrames);
		data.copyTo(samples, { planeIndex: channel, format: "f32-planar" });
		buffer.copyToChannel(samples, channel);
	}
	const source = context.createBufferSource();
	source.buffer = buffer;
	source.connect(context.destination);
	const at = Math.max(context.currentTime, audio.next.get(id) ?? 0);
	source.start(at);
	audio.next.set(id, at + buffer.duration);
}

const others = (id: string): { frames: number; audio: number } => {
	state.others[id] ??= { frames: 0, audio: 0 };
	return state.others[id];
};

async function main(): Promise<void> {
	const theRoom = seats();
	const firstGroup = BigInt(Date.now());
	const video = await createVideoTrackEncoder({
		trackNamespace: fields(mine),
		trackName: utf8.encode(TRACK),
		firstGroup,
		codec: ENCODER_CODEC,
		width: WIDTH,
		height: HEIGHT,
		bitrate: 500_000,
		framerate: FRAMES_A_SECOND,
		keyFrameIntervalMs: 1000,
	});
	const withSound =
		new URLSearchParams(location.search).get("audio") !== "none";
	const sound = withSound
		? await createAudioTrackEncoder({
				trackNamespace: fields(mine),
				trackName: utf8.encode(AUDIO_TRACK),
				firstGroup: BigInt(Date.now()),
				codec: "opus",
				sampleRate: SAMPLE_RATE,
				numberOfChannels: 1,
			})
		: undefined;
	if (sound !== undefined) {
		void fromMicrophone((data) => sound.encode(data)).catch(
			(error: unknown) => {
				state.publishing = `the microphone failed: ${String(error)}`;
				show();
			},
		);
	}
	const catalog = createCatalogPublisher({
		namespace: fields(mine),
		catalog: {
			version: "draft-01",
			tracks: [
				{
					name: TRACK,
					packaging: "loc",
					isLive: true,
					role: "video",
					codec: DECODER_CODEC,
					bitrate: 500_000,
					width: WIDTH,
					height: HEIGHT,
					framerate: FRAMES_A_SECOND,
				},
				...(sound === undefined
					? []
					: [
							{
								name: AUDIO_TRACK,
								packaging: "loc",
								isLive: true,
								role: "audio",
								codec: "opus",
								// What Chromium's encoder was seen to make without a
								// setting, as `publish.ts` has it.
								bitrate: 67_000,
								samplerate: SAMPLE_RATE,
								channelConfig: "1",
							},
						]),
			],
		},
	});

	// A full meeting ends the trying of both, and the page says so; it is
	// opened again to try again. WHY NOT the publishing asked again every
	// 2 s, as `keepPublishing` would: it would take a seat once one is free,
	// and publish on a page that has stopped receiving.
	const full = new AbortController();
	// WHY two connections, one to publish and one to receive: the token of
	// a connection goes on every request it sends (`authorizationToken`),
	// and the seat's `pub` allows the PUBLISH_NAMESPACE of this page alone,
	// its `sub` the SUBSCRIBE_NAMESPACE and the SUBSCRIBEs alone.
	void keepPublishing({
		connect: async (goAway) => {
			const seat = await theRoom.seat();
			return connectToRelay(movedTo(seat.relay, goAway), seat.publish);
		},
		trackNamespace: fields(mine),
		tracks: [
			catalog,
			video.publisher,
			...(sound === undefined ? [] : [sound.publisher]),
		],
		againMs: AGAIN_MS,
		signal: full.signal,
		onState(publishing) {
			state.publishing =
				publishing.type === "failed"
					? `could not publish (${why(publishing.error)})`
					: publishing.type;
			show();
		},
	});

	/**
	 * Claude Code on this machine, while it is called: what the meeting
	 * holds is written into `~/Documents/xmoq/<room>/<id>/` (`AGENT_ROOT`),
	 * as `xmoq-agent --out` writes, and the lines it writes into `inbox.txt`
	 * there are said in the chat, as this page.
	 */
	const agent: { bridge: AgentBridge | undefined } = { bridge: undefined };

	// The chat of the room, under the page's id, as the other pages have it
	// (`room-chat.ts`): the person and Claude Code share it, as they share
	// the seat. Its tokens are asked of the room each time, as theirs.
	const chatRoom: ChatRoom = { kind: "meeting", name: room };
	const chat = startChat(
		chatRoom,
		self,
		async (goAway) => {
			const seat = await theRoom.seat();
			const [sub, pub] = await Promise.all([
				theRoom.token("sub", roomFields(chatRoom)),
				theRoom.token("pub", [...roomFields(chatRoom), self]),
			]);
			return {
				connection: await connectToRelay(movedTo(seat.relay, goAway), sub),
				...(pub === undefined ? {} : { publishNamespaceToken: pub }),
			};
		},
		(line) => agent.bridge?.chat(line),
	);

	const agentNote = document.getElementById("agent-note");
	const linkButton = document.getElementById("agent-link");
	const callButton = document.getElementById("agent-call");
	const tellAgent = (text: string): void => {
		if (agentNote !== null) agentNote.textContent = text;
		document.body.dataset.agent = text;
	};
	/** What the page says of the root before Claude Code is called. */
	const showRoot = async (): Promise<void> => {
		const root = await rememberedRoot();
		switch (root.type) {
			case "unsupported":
				tellAgent("this browser cannot write into a folder: Chromium can");
				return;
			case "none":
				tellAgent(`not linked: make ${AGENT_ROOT} and choose it`);
				return;
			case "granted":
				tellAgent(
					`linked: Claude Code reads ${AGENT_ROOT}/${room}/${self}/ once called`,
				);
				return;
			case "to-allow":
				tellAgent(
					`linked to ${AGENT_ROOT}; the page asks to write there again when Claude Code is called`,
				);
		}
	};
	void showRoot();
	linkButton?.addEventListener("click", () => {
		chooseRoot().then(showRoot, (error: unknown) =>
			tellAgent(`not linked: ${why(error)}`),
		);
	});

	/** Begins writing for Claude Code, choosing the root or allowing it again where it has to be. */
	const call = async (): Promise<() => void> => {
		const remembered = await rememberedRoot();
		let root: FileSystemDirectoryHandle;
		switch (remembered.type) {
			case "unsupported":
				throw new Error(
					"this browser cannot write into a folder: Chromium can",
				);
			case "none":
				root = await chooseRoot();
				break;
			case "to-allow":
				root = remembered.root;
				if (!(await allowRoot(root))) {
					throw new Error(
						`not allowed to write in ${AGENT_ROOT}: call again to be asked again`,
					);
				}
				break;
			case "granted":
				root = remembered.root;
		}
		const folder = await openAgentFolder(root, room, self, {
			onError: (error) => tellAgent(`writing for Claude Code: ${why(error)}`),
		});
		const bridge = createAgentBridge({
			folder,
			say: (text) =>
				chat?.say(text) ?? Promise.resolve("this page has no chat"),
		});
		for (const id of meeting.ids()) bridge.participant("joined", id);
		const stop = new AbortController();
		agent.bridge = bridge;
		// The meeting's transcript, for Claude Code alone: the people there
		// hear the voices, and the page shows no captions.
		let newest = -1n;
		// The prefix the room names, which it gives a token in; the room's
		// meeting where it names none.
		const listened = (await theRoom.meetingTranscript()) ?? [
			"xmoq",
			"meet",
			room,
		];
		void keepFollowingTranscript({
			connect: async (goAway) => {
				const seat = await theRoom.seat();
				return connectToRelay(
					movedTo(seat.relay, goAway),
					await theRoom.token("sub", [...TRANSCRIPT_PREFIX, ...listened]),
				);
			},
			follow: {
				listened,
				latest: 1,
				onSpeech(speech) {
					// A new session fetches the latest piece again.
					if (speech.number <= newest) return;
					newest = speech.number;
					bridge.speech(speech);
				},
			},
			signal: stop.signal,
		});
		tellAgent(
			`Claude Code reads ${AGENT_ROOT}/${folder.path}/ and writes lines into inbox.txt there`,
		);
		// What was written stays.
		return () => {
			bridge.stop();
			stop.abort();
			agent.bridge = undefined;
		};
	};
	// One call at a time: a press while one is made does nothing
	// (`createAgentCall`).
	const agentCall = createAgentCall({
		start: call,
		onState(state) {
			if (callButton !== null) {
				callButton.textContent =
					state.type === "on" ? "stop Claude Code" : "call Claude Code";
				callButton.toggleAttribute("disabled", state.type === "calling");
			}
			if (state.type === "failed")
				tellAgent(`Claude Code not called: ${why(state.error)}`);
			if (state.type === "idle") void showRoot();
		},
	});
	callButton?.addEventListener("click", () => agentCall.press());

	const meeting = createMeeting({
		room,
		self,
		onFrame(id, frame, at) {
			others(id).frames += 1;
			tileOf(id).getContext("2d")?.drawImage(frame, 0, 0, WIDTH, HEIGHT);
			agent.bridge?.frame(id, frame, at);
			frame.close();
		},
		onAudio(id, data) {
			others(id).audio += 1;
			play(id, data);
			data.close();
		},
		onEvent(event) {
			if (event.type === "left") {
				delete state.others[event.id];
				document.querySelector(`canvas[data-id="${event.id}"]`)?.remove();
			}
			if (event.type === "came") others(event.id);
			if (event.type === "came" || event.type === "left") {
				agent.bridge?.participant(
					event.type === "came" ? "joined" : "left",
					event.id,
				);
			}
			if (event.type === "failed") {
				state.failed = [
					...state.failed,
					`${event.id} ${event.what}: ${why(event.error)}`,
				].slice(-5);
			}
			show();
		},
	});
	void keepAttending({
		meeting,
		connect: async (goAway) => {
			const seat = await theRoom.seat();
			return connectToRelay(movedTo(seat.relay, goAway), seat.subscribe);
		},
		againMs: AGAIN_MS,
		signal: full.signal,
		onState(attending) {
			state.receiving =
				attending.type === "failed"
					? `could not attend (${why(attending.error)})`
					: attending.type === "attending-ended"
						? `the session ended (${attending.ended.type})`
						: attending.type;
			show();
			if (document.body.dataset.full === "true") full.abort();
		},
	});

	// The page's own picture: its id, and a clock that moves.
	const preview = document.querySelector("canvas");
	const context = preview?.getContext("2d");
	if (preview === null || context === null || context === undefined) {
		throw new Error("no canvas");
	}
	preview.width = WIDTH;
	preview.height = HEIGHT;
	const began = Date.now();
	const hue = Number.parseInt(self.slice(0, 2), 16) * 1.4;
	every(1000 / FRAMES_A_SECOND, () => {
		const elapsed = Date.now() - began;
		context.fillStyle = `hsl(${hue} 50% 20%)`;
		context.fillRect(0, 0, WIDTH, HEIGHT);
		context.fillStyle = "#ffffff";
		context.font = "20px monospace";
		context.fillText(self, 16, 40);
		context.fillText(new Date().toISOString().slice(11, 22), 16, 80);
		context.fillStyle = "#f2c14e";
		context.fillRect(((elapsed / 20) % (WIDTH + 40)) - 40, HEIGHT - 40, 40, 20);
		video.encode(new VideoFrame(preview, { timestamp: elapsed * 1000 }));
	});
	// The counts of what is received change with every frame.
	every(500, show);
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
