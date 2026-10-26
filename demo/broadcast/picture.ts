// The picture of the broadcast: the mark a viewer measures by, the size it
// is painted in, and the sizes it is sent at.

/**
 * The mark both tracks carry at the same moment of the publisher's clock,
 * for a viewer to measure by: for the first 200 ms of every 2 seconds the
 * picture has a white square in its upper right corner, and the sound is a
 * loud beep.
 */
export const marked = (wallClockMs: number): boolean =>
	wallClockMs % 2000 < 200;
/** Where the square is. */
export const MARK = { x: 540, y: 20, size: 80 };
/** The size the picture is painted in, whatever size it is sent at: the mark and the clock are at the same place of every size. */
export const WIDTH = 640;
export const HEIGHT = 360;

/**
 * The sizes the publishing page sends at, `/publish?size=360|720|1080`
 * (360 when left out), 30 frames a second, 16:9.
 *
 * - **The level** is the lowest of H.264 that has the size at 30 frames a
 *   second (Table A-1 of H.264, read from `libavcodec/h264_levels.c` of
 *   FFmpeg on 2026-10-12): Level 3.1 (`1F`) has frames of up to 3,600
 *   macroblocks and 108,000 a second, which 1280x720 at 30 is exactly
 *   (80x45 = 3,600); 1920x1080 is 8,160 macroblocks (120x68) and 244,800 a
 *   second, which Level 4 (`28`: 8,192 and 245,760) has and 3.1 and 3.2
 *   (5,120) do not. The profile stays Constrained Baseline (`42E0`).
 * - **The bitrate** where `?bitrate=` gives none: **provisional**, twice
 *   and four times that of 360, not from a measurement of what looks well.
 */
export const SIZES = {
	"360": { width: 640, height: 360, level: "1F", bitrate: 1_500_000 },
	"720": { width: 1280, height: 720, level: "1F", bitrate: 3_000_000 },
	"1080": { width: 1920, height: 1080, level: "28", bitrate: 6_000_000 },
} as const;

