// The watching page of the demo: plays the video track and the audio track
// together, by the times their Objects carry, and shows how long a frame
// and a piece of audio took from the publisher's clock to this page.
//
//   ?buffer=N   how many milliseconds a frame or a piece is held before it
//               is shown or sounded, counted from the fastest one that came
//               (150 when left out; 0 plays each as it comes)
//   ?join=no    waits for the next key frame, as without the Joining FETCH

import {
	type AudioPlayback,
	AUTO_MARGIN_MS,
	AUTO_WINDOW_MS,
	createPlayout,
	createQualitySwitch,
	keepPlaying,
	DEFAULT_AUDIO_MIN_MS,
	DEFAULT_AUTO_MAX_MS,
	DEFAULT_AUTO_SHARE,
	DEFAULT_PLAYOUT_BUFFER_MS,
	playAudioTrack,
	playVideoTrack,
	type VideoPlayback,
} from "@xmoq/media";
import type { MoqtConnection } from "@xmoq/moqt";
import { type CatalogTrack, followCatalog, trackNamespaceOf } from "@xmoq/msf";
import {
	connectToRelay,
	firstBytes,
	HERE,
	MARK,
	namespace,
	joinRoom,
	SAMPLE_RATE,
	WIDTH,
	say,
	through,
	why,
	every,
	perSecond,
} from "./shared.ts";

const query = new URLSearchParams(location.search);
// When a frame and a piece are played, the buffer between, and how the
// pieces of sound are joined are @xmoq/media's (`createPlayout`); this page
// draws, counts and says what it sees. `?audioMin=`, `?autoMax=` and
// `?autoShare=` set what the package has a value for.
const AUDIO_MIN_MS = Number(query.get("audioMin") ?? DEFAULT_AUDIO_MIN_MS);
const AUTO_MAX_MS = Number(query.get("autoMax") ?? DEFAULT_AUTO_MAX_MS);
const AUTO_SHARE = Number(query.get("autoShare") ?? DEFAULT_AUTO_SHARE);


/** The last 200 values of something measured, for its median and its largest. */
function measured(): {
	add(value: number): void;
	median(): number;
	text(): string;
	values: number[];
} {
	const values: number[] = [];
	return {
		values,
		add(value) {
			values.push(value);
			if (values.length > 200) values.shift();
		},
		median() {
			if (values.length === 0) return Number.NaN;
			const sorted = [...values].sort((a, b) => a - b);
			return Math.round(sorted[Math.floor(sorted.length / 2)] ?? 0);
		},
		text() {
			if (values.length === 0) return "?";
			const sorted = [...values].sort((a, b) => a - b);
			return `median ${Math.round(sorted[Math.floor(sorted.length / 2)] ?? 0)} ms, at most ${Math.round(sorted.at(-1) ?? 0)} ms`;
		},
	};
}

/** How long the catalog of a broadcast has to come, in milliseconds. */
const CATALOG_TIMEOUT_MS = 5000;

/**
 * Reads the MSF catalog of the broadcast and gives its first video track
 * and its first audio track, by their `role`. The page is told no track
 * name: what it plays is what the catalog has. Rejects where no catalog
 * comes in time, or it has no video track.
 */
async function tracksOf(
	connection: MoqtConnection,
	catalogNamespace: Uint8Array[],
): Promise<{
	video: CatalogTrack;
	versions: CatalogTrack[];
	audio: CatalogTrack | undefined;
}> {
	const follower = await followCatalog(connection, {
		namespace: catalogNamespace,
	});
	const reader = follower.states.getReader();
	const late = new Promise<never>((_, reject) => {
		setTimeout(
			() => reject(new Error("no catalog came within 5 s")),
			CATALOG_TIMEOUT_MS,
		);
	});
	late.catch(() => {});
	try {
		for (;;) {
			const next = await Promise.race([reader.read(), late]);
			if (next.done) throw new Error("the catalog track ended without a catalog");
			const tracks = next.value.current?.catalog.tracks;
			if (tracks === undefined) continue;
			const video = tracks.find((track) => track.role === "video");
			if (video === undefined) throw new Error("the catalog has no video track");
			document.body.dataset.catalog = JSON.stringify(tracks);
			// The versions of the video: the tracks of its alternate group
			// (MSF §5.2.12), the one that takes the most room first.
			const versions =
				video.altGroup === undefined
					? [video]
					: tracks
							.filter(
								(track) =>
									track.role === "video" && track.altGroup === video.altGroup,
							)
							.sort((a, b) => (b.bitrate ?? 0) - (a.bitrate ?? 0));
			return {
				video,
				versions,
				audio: tracks.find((track) => track.role === "audio"),
			};
		}
	} finally {
		// The subscription to the catalog goes on with the session; a later
		// catalog is not read.
		reader.releaseLock();
	}
}

async function main(): Promise<void> {
	const canvas = document.querySelector("canvas");
	const context = canvas?.getContext("2d");
	const probe = new OffscreenCanvas(1, 1).getContext("2d", {
		willReadFrequently: true,
	});
	if (canvas === null || context === null || context === undefined) {
		throw new Error("no canvas");
	}
	if (probe === null) throw new Error("no canvas to read a pixel with");
	// Where rooms run, the room says where the broadcast is; else it is the
	// relay this page was served with.
	const room = joinRoom(`viewer-${Math.floor(Math.random() * 1000)}`);
	if (room !== undefined) await room.broadcast;
	/** What the first line under the picture says of the session. */
	let link = "connecting";
	let sessions = 0;

	/** Of the frames that were drawn: how long each took to get here, waited for its turn, and waited for the repaint. */
	const delay = { arrive: measured(), buffer: measured(), paint: measured() };
	/** Frames that came after their turn, and were drawn late. */
	let framesLate = 0;

	const arrived = { video: measured(), audio: measured() };
	const played = { video: measured(), audio: measured() };
	/** When the mark came on, by the time of the Object and by this page's clock at the play, in milliseconds. */
	const marks: {
		video: { time: number; played: number }[];
		audio: { time: number; played: number }[];
	} = { video: [], audio: [] };

	const began = performance.now();
	let firstFrameAfterMs: number | undefined;
	let firstShownAfterMs: number | undefined;
	/** When the tracks of the first session were subscribed to, since the page began. */
	let playingAfterMs: number | undefined;
	const shownLog: [number, number][] = [];
	/** Whether a frame that waits has the mark. */
	const marked = new WeakMap<VideoFrame, boolean>();
	let markShown = false;
	const framesDecoded = perSecond();
	const framesShown = perSecond();
	/** When a frame was last drawn, how many were, and when the picture stood still, by this page's clock in milliseconds. */
	let lastShown: number | undefined;
	let shown = 0;
	const stalls: { from: number; to: number }[] = [];
	let playback: VideoPlayback | undefined;
	/** The namespace of the broadcast the page last connected to. */
	let playingNamespace: Uint8Array[] = namespace();
	let audio: AudioPlayback | undefined;

	let tooLate = 0;
	/** From the SUBSCRIBE of the audio being accepted to the first piece decoded, and to the first that was in time to sound. */
	let audioSince = performance.now();
	let firstAudioAfterMs: number | undefined;
	let firstSoundAfterMs: number | undefined;
	/** When a piece of audio was last in time to be sounded, by this page's clock. */
	let lastHeard: number | undefined;
	let markHeard = false;
	// Everything that sounds goes through the playout's output, where it is
	// listened to (`listener`) on its way to the loudspeaker: what is said
	// under the picture about the sound is what leaves here, not what was
	// decoded.
	let listener: AnalyserNode | undefined;
	let sounded = 0;
	/** Pieces that did not follow the one before them to the sample: the first, and one after a gap. */
	let apart = 0;
	/** Pieces that were played slower or faster. */
	let stretched = 0;
	const piecesSounded = perSecond();
	const piecesTooLate = perSecond();

	const playout = createPlayout({
		bufferMs:
			query.get("buffer") === "auto"
				? DEFAULT_PLAYOUT_BUFFER_MS
				: Number(query.get("buffer") ?? DEFAULT_PLAYOUT_BUFFER_MS),
		auto: query.get("buffer") === "auto",
		audioMinMs: AUDIO_MIN_MS,
		autoMaxMs: AUTO_MAX_MS,
		autoShare: AUTO_SHARE,
		sampleRate: SAMPLE_RATE,
		draw(frame) {
			if (
				canvas.width !== frame.displayWidth ||
				canvas.height !== frame.displayHeight
			) {
				canvas.width = frame.displayWidth;
				canvas.height = frame.displayHeight;
			}
			context.drawImage(frame, 0, 0);
		},
		onFramesDue(due, now) {
			for (const each of due) {
				// A mark that comes on is noted with the frame that shows it,
				// also where that frame is passed over for a later one.
				const mark = marked.get(each.frame) ?? false;
				if (mark && !markShown) {
					marks.video.push({ time: each.frame.timestamp / 1000, played: now });
				}
				markShown = mark;
			}
		},
		onFrameShown(show) {
			const { now, turn } = show;
			firstShownAfterMs ??= Math.round(performance.now() - began);
			// `?diag=1`: when each of the first frames was shown, and how long after it was made.
			if (shownLog.length < 300) {
				shownLog.push([
					Math.round(performance.now() - began),
					Math.round(now - show.frame.timestamp / 1000),
				]);
			}
			framesShown.count();
			played.video.add(now - show.frame.timestamp / 1000);
			delay.arrive.add(show.came - show.frame.timestamp / 1000);
			delay.buffer.add(Math.max(0, turn - show.came));
			delay.paint.add(now - Math.max(turn, show.came));
			if (show.came > turn) framesLate += 1;
			// A picture that stood still for more than 100 ms.
			if (lastShown !== undefined && now - lastShown > 100) {
				stalls.push({ from: lastShown, to: now });
			}
			lastShown = now;
			shown += 1;
		},
		onPiece(piece) {
			firstAudioAfterMs ??= Math.round(performance.now() - audioSince);
			const timeMs = piece.timestampUs / 1000;
			arrived.audio.add(piece.now - timeMs);
			const loud = piece.samples.some((sample) => Math.abs(sample) > 0.3);
			if (loud && !markHeard) {
				marks.audio.push({ time: timeMs, played: piece.due });
			}
			markHeard = loud;
			if (piece.result === "too-late" || piece.result === "unheard-too-late") {
				tooLate += 1;
				piecesTooLate.count();
				return;
			}
			played.audio.add(piece.due - timeMs);
			firstSoundAfterMs ??= Math.round(performance.now() - audioSince);
			// Nothing sounds: only what would have is counted.
			if (piece.result === "unheard") return;
			if (!piece.follows) apart += 1;
			if (piece.stretched) stretched += 1;
			sounded += 1;
			piecesSounded.count();
			lastHeard = piece.now;
		},
		connectOutput(output, sound) {
			listener = sound.createAnalyser();
			listener.fftSize = 2048;
			output.connect(listener);
			listener.connect(sound.destination);
		},
	});
	const { clock } = playout;

	// The buffer can be set while watching.
	const controls = document.createElement("span");
	const slider = document.createElement("input");
	slider.type = "range";
	slider.min = "0";
	slider.max = "500";
	slider.step = "10";
	slider.id = "buffer";
	controls.append(" buffer: ");
	for (const ms of [0, 50, 150]) {
		const button = document.createElement("button");
		button.textContent = `${ms} ms`;
		button.id = `buffer-${ms}`;
		button.addEventListener("click", () => clock.setBuffer(ms));
		controls.append(button, " ");
	}
	const autoButton = document.createElement("button");
	autoButton.textContent = "auto (a trial)";
	autoButton.id = "buffer-auto";
	autoButton.addEventListener("click", () => clock.setAuto());
	slider.addEventListener("input", () => clock.setBuffer(Number(slider.value)));
	controls.append(autoButton, " ", slider);
	document.getElementById("sound")?.after(controls);
	slider.value = String(clock.bufferMs());

	// `?abr=1` (**a trial**): where the catalog has the video in several
	// versions, the page plays a lower one when the picture comes late and
	// a higher one again when it has come in time for a while
	// (`createQualitySwitch` of @xmoq/media).
	const ABR = query.has("abr");
	/** The fastest a frame came, and how much later than that the frames of the last second came. */
	let fastestVideo = Number.POSITIVE_INFINITY;
	let recent: { at: number; late: number }[] = [];
	const lateNow = (): number | undefined => {
		const since = performance.now() - 1000;
		recent = recent.filter((each) => each.at > since);
		const sorted = recent.map((each) => each.late).sort((a, b) => a - b);
		return sorted[Math.floor(sorted.length / 2)];
	};
	/** Which version is played, and every change: when it was decided, to which, and how long until its first frame. */
	const quality: {
		level: number;
		track: string;
		changes: { at: number; to: string; tookMs: number | null }[];
	} = { level: 0, track: "", changes: [] };
	const videoOptions = {
		joiningFetch: query.get("join") !== "no",
		onFrame(frame: VideoFrame) {
			firstFrameAfterMs ??= Math.round(performance.now() - began);
			const took = Date.now() - frame.timestamp / 1000;
			arrived.video.add(took);
			fastestVideo = Math.min(fastestVideo, took);
			recent.push({ at: performance.now(), late: took - fastestVideo });
			// The mark is where it is in a picture of the full size.
			const scale = frame.displayWidth / WIDTH;
			probe.drawImage(
				frame,
				(MARK.x + MARK.size / 2) * scale,
				(MARK.y + MARK.size / 2) * scale,
				1,
				1,
				0,
				0,
				1,
				1,
			);
			const [red] = probe.getImageData(0, 0, 1, 1).data;
			framesDecoded.count();
			marked.set(frame, (red ?? 0) > 128);
			playout.onFrame(frame);
		},
	};

	// The sound: it sounds once the button was clicked (a browser lets a page
	// make sound only after a click); the times are measured either way.
	document.getElementById("sound")?.addEventListener("click", () => {
		playout.startSound();
	});
	/** The largest sample of the last 43 ms on its way out, of 1, and the pitch that is loudest in it, in Hz. */
	const leaving = (): { peak: number; hz: number } | undefined => {
		const sound = playout.sound();
		if (listener === undefined || sound === undefined) return undefined;
		const samples = new Float32Array(listener.fftSize);
		listener.getFloatTimeDomainData(samples);
		const pitches = new Uint8Array(listener.frequencyBinCount);
		listener.getByteFrequencyData(pitches);
		const loudest = pitches.indexOf(Math.max(...pitches));
		return {
			peak: samples.reduce((most, each) => Math.max(most, Math.abs(each)), 0),
			hz: Math.round((loudest * sound.sampleRate) / listener.fftSize),
		};
	};
	// Breaks in what leaves: a step from one sample to the next that no tone
	// of the demo makes (the beep, 0.6 of full scale at 1,000 Hz, moves 0.08
	// a sample at most). A piece that begins a little late or early leaves
	// such a step, and is heard as a click. Looked at every 40 ms, over the
	// 40 ms that came since.
	const BREAK = 0.15;
	let breaks = 0;
	const breaksNow = perSecond();
	every(40, () => {
		const sound = playout.sound();
		if (listener === undefined || sound?.state !== "running") return;
		const samples = new Float32Array(listener.fftSize);
		listener.getFloatTimeDomainData(samples);
		const from = samples.length - Math.round(sound.sampleRate * 0.04);
		for (let index = Math.max(from, 1); index < samples.length; index++) {
			const step = Math.abs(
				(samples[index] ?? 0) - (samples[index - 1] ?? 0),
			);
			if (step > BREAK) {
				breaks += 1;
				breaksNow.count();
			}
		}
	});
	const audioOptions = {
		// `?audioJoin=1` fetches the Group of audio the page comes into.
		joiningFetch: query.get("audioJoin") !== null,
		onData: playout.onData,
	};

	every(500, () => {
		const sound = playout.sound();
		const bufferMs = clock.bufferMs();
		const auto = clock.isAuto();
		const lateness99 = clock.lateness;
		// The slider follows the buttons and the automatic buffer.
		slider.value = String(bufferMs);
		const stats = playback?.stats() ?? {
			decoded: 0,
			dropped: {},
			joiningFetch: "not-sent",
		};
		const heard = audio?.stats();
		// The marks of the two tracks that belong together: the same moment
		// of the publisher's clock, within half the time between two marks.
		const together = marks.video.flatMap((seen) => {
			const beep = marks.audio.find(
				(each) => Math.abs(each.time - seen.time) < 1000,
			);
			return beep === undefined
				? []
				: [{ sent: seen.time - beep.time, played: seen.played - beep.played }];
		});
		const lastMark = together.at(-1);
		say(
			[
				`${link} · ${framesShown.rate()} frames a second shown, ${framesDecoded.rate()} decoded${document.visibilityState === "hidden" ? " (this tab is in the background: nothing is drawn)" : ""} · buffer ${bufferMs} ms${bufferMs < AUDIO_MIN_MS ? ` (the sound waits ${AUDIO_MIN_MS} ms at least: it is ${AUDIO_MIN_MS - bufferMs} ms behind the picture)` : ""}${auto ? ` (auto, ${AUTO_MAX_MS} ms at most: ${Math.round(AUTO_SHARE * 100)} of 100 came within ${Math.round(lateness99())} ms of the fastest in the last ${AUTO_WINDOW_MS / 1000} s, and ${AUTO_MARGIN_MS} more)` : ""}`,
				`delay of the picture: ${played.video.median()} ms = ${delay.arrive.median()} to get here (encoder, link, relay, decoder) + ${delay.buffer.median()} waiting in the buffer + ${delay.paint.median()} for the next repaint (medians of the last 200 frames; ${framesLate} frames came after their turn). Not in it: from the camera to the encoder, and from the repaint to the screen`,
				`video: ${stats.decoded} frames, first after ${firstFrameAfterMs ?? "?"} ms; came ${arrived.video.text()} after it was made, shown ${played.video.text()} after; dropped ${JSON.stringify(stats.dropped)}; joining FETCH ${stats.joiningFetch}`,
				`audio: ${heard === undefined ? "none" : `${heard.decoded} pieces; came ${arrived.audio.text()} after it was made, sounded ${played.audio.text()} after; ${tooLate} too late; dropped ${JSON.stringify(heard.dropped)}`}`,
				`sound: ${
					sound === undefined
						? "off — click “sound” to hear it"
						: `${sound.state}; ${piecesSounded.rate()} pieces a second go to the loudspeaker, ${piecesTooLate.rate()} a second come too late for it; ${breaksNow.rate()} breaks a second in it (${breaks} in all, ${apart} pieces apart from the one before, ${stretched} played slower or faster); leaving now at ${(leaving()?.peak ?? 0).toFixed(2)} of full scale, ${leaving()?.hz ?? 0} Hz${(leaving()?.peak ?? 0) < 0.01 ? " — SILENT" : ""}`
				}`,
				`picture and sound: ${together.length} marks; the last one: the picture ${lastMark === undefined ? "?" : Math.round(lastMark.played)} ms after the sound (${lastMark === undefined ? "?" : Math.round(lastMark.sent)} ms as they were sent)`,
			].join("\n"),
		);
		// For a script that looks at the page.
		document.body.dataset.report = JSON.stringify({
			bufferMs,
			auto,
			lateness99: Math.round(lateness99()),
			delay: {
				total: played.video.median(),
				arrive: delay.arrive.median(),
				buffer: delay.buffer.median(),
				paint: delay.paint.median(),
			},
			framesLate,
			videoFrames: stats.decoded,
			decoderFailures: playback?.stats().decoderFailures ?? 0,
			quality,
			audioPieces: heard?.decoded ?? 0,
			tooLate,
			shown,
			waiting: playout.waitingFrames(),
			shownASecond: framesShown.rate(),
			decodedASecond: framesDecoded.rate(),
			stalls,
			lastShown: lastShown ?? null,
			lastHeard: lastHeard ?? null,
			sound: {
				state: sound?.state ?? "not started",
				sounded,
				soundedASecond: piecesSounded.rate(),
				breaks,
				apart,
				stretched,
				breaksASecond: breaksNow.rate(),
				tooLateASecond: piecesTooLate.rate(),
				leaving: leaving() ?? null,
			},
			firstAudioAfterMs: firstAudioAfterMs ?? null,
			firstFrameAfterMs: firstFrameAfterMs ?? null,
			firstShownAfterMs: firstShownAfterMs ?? null,
			playingAfterMs: playingAfterMs ?? null,
			joiningFetch: stats.joiningFetch,
			...(query.get("diag") === null ? {} : { shownLog }),
			firstSoundAfterMs: firstSoundAfterMs ?? null,
			...(query.get("diag") === null
				? {}
				: {
						audioCame: arrived.audio.values,
						firstBytes: [...firstBytes].map(([stream, first]) => [
							stream,
							first.at,
							first.length,
						]),
					}),
			sessions,
			link,
			dropped: { video: stats.dropped, audio: heard?.dropped ?? {} },
			arrived: { video: arrived.video.text(), audio: arrived.audio.text() },
			played: { video: played.video.text(), audio: played.audio.text() },
			together,
		});
	});
	// One session after the other (`keepPlaying` of @xmoq/media): when the
	// session ends, or the tracks do (their publisher went), the page says
	// so and connects again after two seconds, asking the room anew where
	// the broadcast is.
	await keepPlaying({
		async connect() {
			const found = room === undefined ? HERE : room.current();
			if (found === null) return undefined;
			const broadcast = through(found);
			document.body.dataset.broadcast = JSON.stringify(broadcast);
			playingNamespace = namespace(broadcast.namespace);
			// Where the rooms give tokens, the connection carries one.
			return connectToRelay(
				broadcast,
				await room?.token("sub", broadcast.namespace),
			);
		},
		async play(connection) {
			// What is played, and with which codec, is what the catalog says.
			const tracks = await tracksOf(connection, playingNamespace);
			const utf8 = new TextEncoder();
			// `?video=no` plays the sound alone, for looking into it.
			const playVersion = (
				track: CatalogTrack,
				more: Partial<Parameters<typeof playVideoTrack>[1]> = {},
			): Promise<VideoPlayback> =>
				playVideoTrack(connection, {
					...videoOptions,
					trackNamespace: trackNamespaceOf(track, playingNamespace),
					trackName: utf8.encode(track.name),
					codec: track.codec ?? "",
					...more,
				});
			/** Stops the version that is played. */
			let leave = new AbortController();
			const video =
				query.get("video") === "no"
					? undefined
					: await playVersion(tracks.video, { signal: leave.signal });
			playback = video;
			quality.level = 0;
			quality.track = tracks.video.name;
			/** Resolves when the version that is played ends; one that was left for another does not count. */
			let over: (how: string) => void = () => {};
			const ended = new Promise<string>((resolve) => {
				over = resolve;
			});
			void video?.ended.then((end) => {
				if (playback === video) over(end.type);
			});
			if (ABR && video !== undefined && tracks.versions.length > 1) {
				const versions = tracks.versions;
				const chooser = createQualitySwitch({ levels: versions.length });
				let changing = false;
				const ticking = setInterval(() => {
					// Nothing is judged before the first frame, or while a change is under way.
					if (fastestVideo === Number.POSITIVE_INFINITY || changing) return;
					const wanted = chooser.tick(performance.now(), lateNow());
					const track = versions[wanted];
					if (wanted === quality.level || track === undefined) return;
					changing = true;
					const change: (typeof quality.changes)[number] = {
						at: Date.now(),
						to: track.name,
						tookMs: null,
					};
					quality.changes.push(change);
					const decided = performance.now();
					const stopBefore = leave;
					const next = new AbortController();
					let first = true;
					// WHY no Joining FETCH: the version begins with its next
					// Group, and the one before is played until then, so the
					// change is at a Group's start and nothing old is fetched.
					playVersion(track, {
						joiningFetch: false,
						signal: next.signal,
						onFrame(frame) {
							if (first) {
								first = false;
								change.tookMs = Math.round(performance.now() - decided);
								// From its first frame on, the new version alone.
								stopBefore.abort();
								leave = next;
								quality.level = wanted;
								quality.track = track.name;
								changing = false;
							}
							videoOptions.onFrame(frame);
						},
					}).then(
						(playing) => {
							// The version before is left when this one has its
							// first frame; until then both come.
							playback = playing;
							void playing.ended.then((end) => {
								if (playback === playing) over(end.type);
							});
						},
						(error: unknown) => {
							console.log(`no change of version: ${String(error)}`);
							quality.changes.pop();
							changing = false;
						},
					);
				}, 250);
				void connection.closed.then(() => clearInterval(ticking));
			}
			const sound = tracks.audio;
			audio = sound === undefined ? undefined : await playAudioTrack(connection, {
				...audioOptions,
				// An Object that comes before its turn waits for the ones
				// before it as long as the buffer leaves, less what the
				// decoder and the loudspeaker need.
				...(query.get("order") === "no"
					? {}
					: { holdForOrderMs: clock.holdForOrderMs }),
				trackNamespace: trackNamespaceOf(sound, playingNamespace),
				trackName: utf8.encode(sound.name),
				codec: sound.codec ?? "",
				// From the catalog; the demo's own where a catalog has none.
				sampleRate: sound.samplerate ?? SAMPLE_RATE,
				numberOfChannels: Number(sound.channelConfig ?? 1),
			})
				.then((playing) => {
					audioSince = performance.now();
					return playing;
				})
				.catch((error: unknown) => {
					console.log(`no audio: ${String(error)}`);
					return undefined;
				});
			return {
				// Without video, the session's end is the end.
				ended,
			};
		},
		onState(state) {
			const at = new Date().toISOString().slice(11, 19);
			switch (state.type) {
				case "connecting":
					link = "connecting";
					return;
				case "nothing-to-play":
					link = "nobody publishes in this room; asking again every 2 s";
					return;
				case "playing":
					sessions = state.session;
					playingAfterMs ??= Math.round(performance.now() - began);
					link = `playing (session ${sessions})`;
					return;
				case "session-ended":
					link = `the session ended (${state.close.type}) at ${at}; connecting again every 2 s`;
					return;
				case "playing-ended":
					link = `the video track ended (${state.end}) at ${at}; connecting again every 2 s`;
					return;
				case "failed":
					link = `could not play (${why(state.error)}); trying again every 2 s`;
					return;
			}
		},
	});
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
