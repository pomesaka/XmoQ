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
} from "./shared.ts";

const query = new URLSearchParams(location.search);
/** **Provisional, not chosen by measurement**: enough for a frame of 33 ms and the pieces of audio of 20 ms to be there before their turn, on one machine. */
const BUFFER_MS = Number(query.get("buffer") ?? 150);

/** The last 200 values of something measured, for its median and its largest. */
function measured(): { add(value: number): void; text(): string; values: number[] } {
	const values: number[] = [];
	return {
		values,
		add(value) {
			values.push(value);
			if (values.length > 200) values.shift();
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
	// fastest took at least no time. Everything is played `BUFFER_MS` after
	// that, so that picture and sound with the same time come out together.
	let fastestMs = Number.POSITIVE_INFINITY;
	const dueAt = (timestampUs: number): number => {
		fastestMs = Math.min(fastestMs, Date.now() - timestampUs / 1000);
		return timestampUs / 1000 + fastestMs + BUFFER_MS;
	};

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
	let frames: { frame: VideoFrame; due: number; mark: boolean }[] = [];
	let markShown = false;
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
			frames.push({
				frame,
				due: dueAt(frame.timestamp),
				mark: (red ?? 0) > 128,
			});
		},
	};
	const repaint = (): void => {
		const now = Date.now();
		const due = frames.filter((each) => each.due <= now);
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
		frames = frames.filter((each) => each.due > now);
		if (show !== undefined) {
			if (
				canvas.width !== show.frame.displayWidth ||
				canvas.height !== show.frame.displayHeight
			) {
				canvas.width = show.frame.displayWidth;
				canvas.height = show.frame.displayHeight;
			}
			context.drawImage(show.frame, 0, 0);
			played.video.add(now - show.frame.timestamp / 1000);
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
	document.getElementById("sound")?.addEventListener("click", () => {
		sound ??= new AudioContext({ sampleRate: SAMPLE_RATE });
		void sound.resume();
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
			const due = dueAt(data.timestamp);
			const loud = samples.some((sample) => Math.abs(sample) > 0.3);
			if (loud && !markHeard) marks.audio.push({ time: timeMs, played: due });
			markHeard = loud;
			if (due < now) {
				// Its time is over: it is not sounded.
				tooLate += 1;
				return;
			}
			played.audio.add(due - timeMs);
			firstSoundAfterMs ??= Math.round(performance.now() - audioSince);
			if (sound === undefined || sound.state !== "running") return;
			const buffer = sound.createBuffer(1, samples.length, SAMPLE_RATE);
			buffer.copyToChannel(samples, 0);
			const source = sound.createBufferSource();
			source.buffer = buffer;
			source.connect(sound.destination);
			source.start(sound.currentTime + (due - now) / 1000);
			lastHeard = now;
		},
	};

	setInterval(() => {
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
				`${link} · buffer ${BUFFER_MS} ms`,
				`video: ${stats.decoded} frames, first after ${firstFrameAfterMs ?? "?"} ms; came ${arrived.video.text()} after it was made, shown ${played.video.text()} after; dropped ${JSON.stringify(stats.dropped)}; joining FETCH ${stats.joiningFetch}`,
				`audio: ${heard === undefined ? "none" : `${heard.decoded} pieces; came ${arrived.audio.text()} after it was made, sounded ${played.audio.text()} after; ${tooLate} too late; dropped ${JSON.stringify(heard.dropped)}${sound?.state === "running" ? "" : " — click “sound” to hear it"}`}`,
				`picture and sound: ${together.length} marks; the last one: the picture ${lastMark === undefined ? "?" : Math.round(lastMark.played)} ms after the sound (${lastMark === undefined ? "?" : Math.round(lastMark.sent)} ms as they were sent)`,
			].join("\n"),
		);
		// For a script that looks at the page.
		document.body.dataset.report = JSON.stringify({
			bufferMs: BUFFER_MS,
			videoFrames: stats.decoded,
			audioPieces: heard?.decoded ?? 0,
			tooLate,
			shown,
			stalls,
			lastShown: lastShown ?? null,
			lastHeard: lastHeard ?? null,
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
	}, 500);
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
				connection = await connectToRelay(broadcast);
				const trackNamespace = namespace(broadcast.namespace);
				// Where the rooms give tokens, the requests carry one.
				const token = await room?.token("sub", broadcast.namespace);
				const authorization =
					token === undefined ? {} : { authorizationToken: token };
				// `?video=no` plays the sound alone, for looking into it.
				const video =
					query.get("video") === "no"
						? undefined
						: await playVideoTrack(connection, {
								...videoOptions,
								...authorization,
								trackNamespace,
							});
				playback = video;
				audio = await playAudioTrack(connection, {
					...audioOptions,
					...authorization,
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
