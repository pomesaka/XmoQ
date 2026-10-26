import { describe, expect, test } from "bun:test";
import { chooseTracks, type OfferedTrack } from "../src/core/tracks.ts";
import { createMeeting } from "../src/web/meeting.ts";

const large: OfferedTrack = {
	name: "720p",
	role: "video",
	width: 1280,
	height: 720,
};
const small: OfferedTrack = {
	name: "180p",
	role: "video",
	width: 320,
	height: 180,
};
const sizeless: OfferedTrack = { name: "video", role: "video" };
const sound: OfferedTrack = { name: "audio", role: "audio" };
const elsewhere: OfferedTrack = {
	name: "tiny",
	namespace: "xmoq/meet/r/other",
	role: "video",
	width: 16,
	height: 9,
};

describe("which tracks of a catalog are received", () => {
	test.each([
		[
			"the smaller of two videos, and the sound",
			[large, small, sound],
			"180p",
			"audio",
		],
		[
			"a video with its size before one without",
			[sizeless, large],
			"720p",
			undefined,
		],
		[
			"a video without a size where it is the only one",
			[sizeless],
			"video",
			undefined,
		],
		["the sound alone", [sound], undefined, "audio"],
		["nothing of another namespace", [elsewhere, large], "720p", undefined],
		["nothing in an empty catalog", [], undefined, undefined],
	])("%s", (_case, tracks, video, audio) => {
		const chosen = chooseTracks(tracks);
		expect(chosen.video?.name).toBe(video);
		expect(chosen.audio?.name).toBe(audio);
	});
});

describe("the limits of a meeting", () => {
	const meeting = (mostWatched: number, mostOthers: number) => () =>
		createMeeting({
			room: "r",
			self: "me",
			mostWatched,
			mostOthers,
			onFrame: () => {},
			onAudio: () => {},
		});

	test.each([
		["no video at all", 0, 48],
		["one other", 8, 1],
		["more videos than others", 8, 4],
	])("%s is taken", (_case, mostWatched, mostOthers) => {
		expect(meeting(mostWatched, mostOthers)).not.toThrow();
	});

	test.each([
		["fewer than no videos", -1, 48],
		["part of a video", 1.5, 48],
		["no others", 8, 0],
		["no number of others", 8, Number.NaN],
		["no number of videos", Number.POSITIVE_INFINITY, 48],
	])("%s is refused", (_case, mostWatched, mostOthers) => {
		expect(meeting(mostWatched, mostOthers)).toThrow(RangeError);
	});
});
