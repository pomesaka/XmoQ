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
	const meeting = (mostWatched?: number, mostOthers?: number) => () =>
		createMeeting({
			room: "r",
			self: "me",
			...(mostWatched === undefined ? {} : { mostWatched }),
			...(mostOthers === undefined ? {} : { mostOthers }),
			onFrame: () => {},
			onAudio: () => {},
		});

	test.each([
		["no video at all", 0, undefined],
		["one other", undefined, 1],
		["the defaults", undefined, undefined],
	])("%s is taken", (_case, mostWatched, mostOthers) => {
		expect(meeting(mostWatched, mostOthers)).not.toThrow();
	});

	test.each([
		["fewer than no videos", -1, undefined],
		["part of a video", 1.5, undefined],
		["no others", undefined, 0],
		["no number of others", undefined, Number.NaN],
	])("%s is refused", (_case, mostWatched, mostOthers) => {
		expect(meeting(mostWatched, mostOthers)).toThrow(RangeError);
	});
});
