// The watching page of the demo: plays the video track and the audio track
// together, by the times their Objects carry, and shows how long a frame
// and a piece of audio took from the publisher's clock to this page.
//
//   ?buffer=N   how many milliseconds a frame or a piece is held before it
//               is shown or sounded, counted from the fastest one that came
//               (150 when left out; 0 plays each as it comes)
//   ?join=no    waits for the next key frame, as without the Joining FETCH

import type { MoqtConnection } from "@xmoq/moqt";
import {
	type AudioPlayback,
	playAudioTrack,
	playVideoTrack,
	type VideoPlayback,
} from "@xmoq/media";
import {
	audioTrackName,
	connectToRelay,
	DECODER_CODEC,
	firstBytes,
	HERE,
	MARK,
	namespace,
	joinRoom,
	SAMPLE_RATE,
	say,
	through,
	why,
	trackName,
	every,
	perSecond,
} from "./shared.ts";

const query = new URLSearchParams(location.search);
/** **Provisional**: on one machine 99 of 100 frames and pieces came within 17 to 24 ms of the fastest, and with 50 ms none was late (docs/experiments/2026-10-11-where-the-delay-of-the-demo-comes-from.md). */
const DEFAULT_BUFFER_MS = 50;
/**
 * The sound waits at least this long, also where the buffer is less
 * (`?audioMin=`). WHY: a piece is given to the loudspeaker for a time, and
 * with no buffer that time is over when the piece comes: nothing sounds.
 * The picture is drawn as it comes. **Provisional**: the least of 10, 20,
 * 30 and 40 ms with which no piece was too late on one machine.
 */
const AUDIO_MIN_MS = Number(query.get("audioMin") ?? 30);
/**
 * `?buffer=auto` (**a trial, not the default**): the buffer follows how
 * late things come. It is what 99 of 100 of the last AUTO_WINDOW_MS came
 * within, after the fastest, and AUTO_MARGIN_MS more. It grows at once and
 * shrinks AUTO_STEP_MS at a time, twice a second.
 */
const AUTO_WINDOW_MS = 10_000;
const AUTO_MARGIN_MS = 10;
const AUTO_STEP_MS = 10;
/**
 * How far the automatic buffer goes, and which share of what came it
 * covers: `?autoMax=` and `?autoShare=`. WHY a limit: with loss, a few of
 * 100 come seconds late (measured: 0.8 s at 1 %, 5 s at 5 %), and a
 * buffer that waits for those is not live any more.
 */
const AUTO_MAX_MS = Number(query.get("autoMax") ?? 400);
const AUTO_SHARE = Number(query.get("autoShare") ?? 0.99);

/** How many decoded frames may wait to be drawn: a second of them. */
const MAX_WAITING_FRAMES = 30;

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

	// The time of a frame or a piece is the publisher's clock, in
	// microseconds. WHY the smallest difference to this page's clock is taken
	// as "no delay": the two clocks need not agree, and the one that came
	// fastest took at least no time. Everything is played `bufferMs` after
	// that, so that picture and sound with the same time come out together.
	let fastestMs = Number.POSITIVE_INFINITY;
	let bufferMs =
		query.get("buffer") === "auto"
			? DEFAULT_BUFFER_MS
			: Number(query.get("buffer") ?? DEFAULT_BUFFER_MS);
	let auto = query.get("buffer") === "auto";
	/** How long after the fastest each frame and piece of the last AUTO_WINDOW_MS came, and when. */
	let lateness: { at: number; ms: number }[] = [];
	/** What 99 of 100 of them came within. */
	const lateness99 = (): number => {
		const sorted = lateness.map((each) => each.ms).sort((a, b) => a - b);
		return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * AUTO_SHARE))] ?? 0;
	};
	/** When something with this time would be played with no buffer: its time, on this page's clock. */
	const baseOf = (timestampUs: number): number => {
		const now = Date.now();
		fastestMs = Math.min(fastestMs, now - timestampUs / 1000);
		const base = timestampUs / 1000 + fastestMs;
		lateness.push({ at: now, ms: now - base });
		return base;
	};

	// The buffer can be set while watching.
	const controls = document.createElement("span");
	const slider = document.createElement("input");
	slider.type = "range";
	slider.min = "0";
	slider.max = "500";
	slider.step = "10";
	slider.id = "buffer";
	const setBuffer = (ms: number, byAuto = false): void => {
		bufferMs = ms;
		if (!byAuto) auto = false;
		slider.value = String(ms);
	};
	controls.append(" buffer: ");
	for (const ms of [0, 50, 150]) {
		const button = document.createElement("button");
		button.textContent = `${ms} ms`;
		button.id = `buffer-${ms}`;
		button.addEventListener("click", () => setBuffer(ms));
		controls.append(button, " ");
	}
	const autoButton = document.createElement("button");
	autoButton.textContent = "auto (a trial)";
	autoButton.id = "buffer-auto";
	autoButton.addEventListener("click", () => {
		auto = true;
	});
	slider.addEventListener("input", () => setBuffer(Number(slider.value)));
	controls.append(autoButton, " ", slider);
	document.getElementById("sound")?.after(controls);
	setBuffer(bufferMs, auto);
	every(500, () => {
		const since = Date.now() - AUTO_WINDOW_MS;
		lateness = lateness.filter((each) => each.at > since);
		if (!auto || lateness.length < 50) return;
		const wanted = Math.min(
			AUTO_MAX_MS,
			Math.ceil((lateness99() + AUTO_MARGIN_MS) / AUTO_STEP_MS) * AUTO_STEP_MS,
		);
		if (wanted > bufferMs) setBuffer(wanted, true);
		else if (wanted < bufferMs) setBuffer(bufferMs - AUTO_STEP_MS, true);
	});

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

	// Video: decoded frames wait in a queue until their time, and the newest
	// one that is due is drawn at the next repaint.
	const began = performance.now();
	let firstFrameAfterMs: number | undefined;
	let frames: {
		frame: VideoFrame;
		/** When it would be drawn with no buffer, and when it was decoded, by this page's clock. */
		base: number;
		came: number;
		mark: boolean;
	}[] = [];
	let markShown = false;
	const framesDecoded = perSecond();
	const framesShown = perSecond();
	/** When a frame was last drawn, how many were, and when the picture stood still, by this page's clock in milliseconds. */
	let lastShown: number | undefined;
	let shown = 0;
	const stalls: { from: number; to: number }[] = [];
	let playback: VideoPlayback | undefined;
	let audio: AudioPlayback | undefined;
	const videoOptions = {
		trackName: trackName(),
		codec: DECODER_CODEC,
		joiningFetch: query.get("join") !== "no",
		onFrame(frame: VideoFrame) {
			firstFrameAfterMs ??= Math.round(performance.now() - began);
			arrived.video.add(Date.now() - frame.timestamp / 1000);
			probe.drawImage(
				frame,
				MARK.x + MARK.size / 2,
				MARK.y + MARK.size / 2,
				1,
				1,
				0,
				0,
				1,
				1,
			);
			const [red] = probe.getImageData(0, 0, 1, 1).data;
			framesDecoded.count();
			// WHY a limit: a tab in the background is not repainted, and the
			// frames it decodes would wait without end, each with its
			// picture. The oldest go; the picture catches up at once when
			// the tab is looked at again.
			while (frames.length >= MAX_WAITING_FRAMES) frames.shift()?.frame.close();
			frames.push({
				frame,
				base: baseOf(frame.timestamp),
				came: Date.now(),
				mark: (red ?? 0) > 128,
			});
		},
	};
	const repaint = (): void => {
		const now = Date.now();
		// WHY the buffer is added here and not when the frame came: the
		// buffer can be moved while watching, and the frames that wait
		// follow it at once.
		const due = frames.filter((each) => each.base + bufferMs <= now);
		const show = due.at(-1);
		for (const each of due) {
			// A mark that comes on is noted with the frame that shows it,
			// also where that frame is passed over for a later one.
			if (each.mark && !markShown) {
				marks.video.push({ time: each.frame.timestamp / 1000, played: now });
			}
			markShown = each.mark;
			if (each !== show) each.frame.close();
		}
		frames = frames.filter((each) => each.base + bufferMs > now);
		if (show !== undefined) {
			if (
				canvas.width !== show.frame.displayWidth ||
				canvas.height !== show.frame.displayHeight
			) {
				canvas.width = show.frame.displayWidth;
				canvas.height = show.frame.displayHeight;
			}
			context.drawImage(show.frame, 0, 0);
			framesShown.count();
			played.video.add(now - show.frame.timestamp / 1000);
			const turn = show.base + bufferMs;
			delay.arrive.add(show.came - show.frame.timestamp / 1000);
			delay.buffer.add(Math.max(0, turn - show.came));
			delay.paint.add(now - Math.max(turn, show.came));
			if (show.came > turn) framesLate += 1;
			show.frame.close();
			// A picture that stood still for more than 100 ms.
			if (lastShown !== undefined && now - lastShown > 100) {
				stalls.push({ from: lastShown, to: now });
			}
			lastShown = now;
			shown += 1;
		}
		requestAnimationFrame(repaint);
	};
	requestAnimationFrame(repaint);

	// Audio: each piece is given to the `AudioContext` for its time. It
	// sounds once the button was clicked (a browser lets a page make sound
	// only after a click); the times are measured either way.
	let sound: AudioContext | undefined;
	let tooLate = 0;
	/** From the SUBSCRIBE of the audio being accepted to the first piece decoded, and to the first that was in time to sound. */
	let audioSince = performance.now();
	let firstAudioAfterMs: number | undefined;
	let firstSoundAfterMs: number | undefined;
	/** When a piece of audio was last in time to be sounded, by this page's clock. */
	let lastHeard: number | undefined;
	let markHeard = false;
	// Everything that sounds goes through `output`, where it is listened to
	// (`listener`) on its way to the loudspeaker: what is said under the
	// picture about the sound is what leaves here, not what was decoded.
	let output: GainNode | undefined;
	let listener: AnalyserNode | undefined;
	let sounded = 0;
	/** Pieces that did not follow the one before them to the sample: the first, and one after a gap. */
	let apart = 0;
	/** Where the last piece that was given to the AudioContext ends, by its clock. */
	let soundUntil: number | undefined;
	/** How far from its own time a piece may begin so as to follow the one before it without a seam, in seconds: less than half a piece of 20 ms. */
	const SEAM = 0.008;
	const piecesSounded = perSecond();
	const piecesTooLate = perSecond();
	document.getElementById("sound")?.addEventListener("click", () => {
		if (sound === undefined) {
			sound = new AudioContext({ sampleRate: SAMPLE_RATE });
			output = sound.createGain();
			listener = sound.createAnalyser();
			listener.fftSize = 2048;
			output.connect(listener);
			listener.connect(sound.destination);
		}
		void sound.resume();
	});
	/** The largest sample of the last 43 ms on its way out, of 1, and the pitch that is loudest in it, in Hz. */
	const leaving = (): { peak: number; hz: number } | undefined => {
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
		trackName: audioTrackName(),
		codec: "opus",
		sampleRate: SAMPLE_RATE,
		numberOfChannels: 1,
		// `?audioJoin=1` fetches the Group of audio the page comes into.
		joiningFetch: query.get("audioJoin") !== null,
		onData(data: AudioData) {
			firstAudioAfterMs ??= Math.round(performance.now() - audioSince);
			const samples = new Float32Array(data.numberOfFrames);
			data.copyTo(samples, { planeIndex: 0, format: "f32-planar" });
			const timeMs = data.timestamp / 1000;
			data.close();
			const now = Date.now();
			arrived.audio.add(now - timeMs);
			const due = baseOf(data.timestamp) + Math.max(bufferMs, AUDIO_MIN_MS);
			const loud = samples.some((sample) => Math.abs(sample) > 0.3);
			if (loud && !markHeard) marks.audio.push({ time: timeMs, played: due });
			markHeard = loud;
			const context = sound;
			if (
				context === undefined ||
				output === undefined ||
				context.state !== "running"
			) {
				// Nothing sounds: only what would have is counted.
				if (due < now) {
					tooLate += 1;
					piecesTooLate.count();
				} else {
					played.audio.add(due - timeMs);
					firstSoundAfterMs ??= Math.round(performance.now() - audioSince);
				}
				return;
			}
			// When the piece is to begin, by the AudioContext's clock: its
			// own time; or, where that is where the piece before it ends,
			// exactly there.
			//
			// WHY not its own time each time: this page's clock counts whole
			// milliseconds, and a decoder gives times a microsecond off. A
			// piece that begins a fraction of a millisecond late leaves a gap
			// of some samples, and one that begins early sounds over the end
			// of the one before: a click at every piece, 100 a second. So a
			// piece follows the one before it to the sample, as long as that
			// is within SEAM of its own time; beyond that the two are apart
			// for a reason (a piece that did not come, a clock that was set),
			// and the piece begins at its own time.
			const wanted = context.currentTime + (due - now) / 1000;
			const follows = soundUntil !== undefined && Math.abs(wanted - soundUntil) <= SEAM;
			const begin = follows && soundUntil !== undefined ? soundUntil : wanted;
			if (begin < context.currentTime) {
				// Its time is over: it is not sounded.
				tooLate += 1;
				piecesTooLate.count();
				return;
			}
			played.audio.add(due - timeMs);
			firstSoundAfterMs ??= Math.round(performance.now() - audioSince);
			const buffer = context.createBuffer(1, samples.length, SAMPLE_RATE);
			buffer.copyToChannel(samples, 0);
			const source = context.createBufferSource();
			source.buffer = buffer;
			source.connect(output);
			source.start(begin);
			if (!follows) apart += 1;
			soundUntil = begin + buffer.duration;
			sounded += 1;
			piecesSounded.count();
			lastHeard = now;
		},
	};

	every(500, () => {
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
						: `${sound.state}; ${piecesSounded.rate()} pieces a second go to the loudspeaker, ${piecesTooLate.rate()} a second come too late for it; ${breaksNow.rate()} breaks a second in it (${breaks} in all, ${apart} pieces apart from the one before); leaving now at ${(leaving()?.peak ?? 0).toFixed(2)} of full scale, ${leaving()?.hz ?? 0} Hz${(leaving()?.peak ?? 0) < 0.01 ? " — SILENT" : ""}`
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
			audioPieces: heard?.decoded ?? 0,
			tooLate,
			shown,
			waiting: frames.length,
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
				breaksASecond: breaksNow.rate(),
				tooLateASecond: piecesTooLate.rate(),
				leaving: leaving() ?? null,
			},
			firstAudioAfterMs: firstAudioAfterMs ?? null,
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
	// One session after the other: when the session ends, or the tracks do
	// (their publisher went), the page says so and connects again after two
	// seconds, asking the room anew where the broadcast is.
	for (;;) {
		const found = room === undefined ? HERE : room.current();
		if (found === null) {
			link = "nobody publishes in this room; asking again every 2 s";
		} else {
			const broadcast = through(found);
			document.body.dataset.broadcast = JSON.stringify(broadcast);
			let connection: MoqtConnection | undefined;
			try {
				link = "connecting";
				// Where the rooms give tokens, the connection carries one.
				connection = await connectToRelay(
					broadcast,
					await room?.token("sub", broadcast.namespace),
				);
				const trackNamespace = namespace(broadcast.namespace);
				// `?video=no` plays the sound alone, for looking into it.
				const video =
					query.get("video") === "no"
						? undefined
						: await playVideoTrack(connection, {
								...videoOptions,
								trackNamespace,
							});
				playback = video;
				audio = await playAudioTrack(connection, {
					...audioOptions,
					trackNamespace,
				})
					.then((playing) => {
						audioSince = performance.now();
						return playing;
					})
					.catch((error: unknown) => {
					console.log(`no audio: ${String(error)}`);
					return undefined;
				});
				sessions += 1;
				link = `playing (session ${sessions})`;
				const how = await Promise.race([
					connection.closed.then((close) => `the session ended (${close.type})`),
					...(video === undefined
						? []
						: [
								video.ended.then(
									(end) => `the video track ended (${end.type})`,
								),
							]),
				]);
				link = `${how} at ${new Date().toISOString().slice(11, 19)}; connecting again every 2 s`;
			} catch (error) {
				link = `could not play (${why(error)}); trying again every 2 s`;
			} finally {
				// Also a session in which nothing could be played is not left open.
				connection?.close();
			}
		}
		await new Promise((resolve) => setTimeout(resolve, 2000));
	}
}

main().catch((error: unknown) => {
	say(`failed: ${String(error)}`);
	console.error(error);
});
