/**
 * The codec strings a video decoder is asked for, in order, for the codec
 * string of a track: the track's own, and for `avc3.…` the same profile and
 * level as `avc1.…` after it.
 *
 * WHY the second: Safari of iOS says it has no decoder for `avc3`
 * (`isConfigSupported`, an iPhone, 2026-10-29). Both are given Annex B here: the decoder
 * is configured without a description, and an AVC decoder without a
 * description takes the bitstream as Annex B, with the parameter sets in it
 * (WebCodecs AVC codec registration, §2 and §3), which is what `avc3` says.
 * WHY NOT `avc1` first: `avc3` is what the catalog says, and a browser that
 * has it is told the truth.
 */
export function decoderCodecs(codec: string): string[] {
	if (codec.startsWith("avc3.")) {
		return [codec, `avc1.${codec.slice("avc3.".length)}`];
	}
	return [codec];
}
