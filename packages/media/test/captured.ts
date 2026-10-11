import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createSubgroupStreamDecoder, type ReceivedObject } from "@xmoq/moqt";

/**
 * The Objects of the subgroup stream of `mlmpub`, the publisher of
 * moqlivemock, that `fixtures/moqt-18/` has: the start of a video stream,
 * a key frame and the frame after it. Decoded by @xmoq/moqt, as a
 * subscription would deliver them; nothing of them is written out here.
 */
/**
 * A chunk of the H.264 encoder of Chromium, in Annex B form, that
 * `fixtures/loc-04/` has: "key", the first chunk of a track, or "delta",
 * the one after it, 40 ms later.
 */
export function capturedChunk(which: "key" | "delta"): Uint8Array {
	return Uint8Array.from(
		readFileSync(
			join(
				import.meta.dir,
				"..",
				"..",
				"..",
				"fixtures",
				"loc-04",
				`video-${which}-annex-b-from-chromium.hex`,
			),
			"utf8",
		)
			.trim()
			.split(/\s+/)
			.map((byte) => Number.parseInt(byte, 16)),
	);
}

export function capturedObjects(): ReceivedObject[] {
	const bytes = Uint8Array.from(
		readFileSync(
			join(
				import.meta.dir,
				"..",
				"..",
				"..",
				"fixtures",
				"moqt-18",
				"subgroup-stream-two-objects-from-moqlivemock.hex",
			),
			"utf8",
		)
			.trim()
			.split(/\s+/)
			.map((byte) => Number.parseInt(byte, 16)),
	);
	const decoder = createSubgroupStreamDecoder({
		// More than the key frame, 21,533 bytes.
		maxObjectPayloadLength: 32 * 1024,
		maxObjectPropertiesLength: 1024,
	});
	const { items, failure } = decoder.push(bytes);
	if (failure !== undefined) {
		throw new Error(`the captured stream is not decoded: ${failure.scope}`);
	}
	const [first, ...rest] = items;
	if (first?.type !== "header") {
		throw new Error("the captured stream has no header");
	}
	const { groupId, publisherPriority } = first.header;
	return rest.flatMap((item) =>
		item.type === "object"
			? [{ groupId, subgroupId: 0n, publisherPriority, object: item.object }]
			: [],
	);
}
