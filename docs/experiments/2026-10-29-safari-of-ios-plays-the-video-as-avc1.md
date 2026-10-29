# Safari of iOS plays the video as avc1

- Date: 2026-10-29
- Commit SHA: the commit that adds this record
- Roadmap item: none; a viewer on an iPhone of the demo shown on the LAN (`--lan` of `demo/serve.ts`).

## Objective

The person opened `/meet` of the demo on an iPhone, and the tile of the PC's page showed nothing; the page said `NotSupportedError: this browser has no decoder for avc3.42E01F`. Does Safari of iOS decode the same track when the decoder is asked for `avc1` with the same profile and level, without a description?

## Expected

Said before the run (in the session, not in a file): yes. A decoder of AVC without a description takes the bitstream as Annex B, with the parameter sets in it (WebCodecs AVC codec registration), which is what the track sends.

## Environment

The person's iPhone, Safari; **the versions of iOS and Safari were not taken**. The PC: macOS 14.4 (Darwin 23.4.0), Chrome, one tab of `/meet`. Bun 1.3.14. The demo: `mise run demo -- --port 8082 --transcribe-language ja --lan 192.168.50.228`, with tokens.

## Commands

1. With main at #560: `/meet` on the PC and on the iPhone, the iPhone past the certificate's warning.
2. With `decoderCodecs` of `@xmoq/media` (`avc3.…`, then `avc1.…` of the same profile and level, the first that `VideoDecoder.isConfigSupported` takes): the demo started again, and both pages loaded again.

## Observed results

1. On the iPhone: no picture of the PC, and `not received … video: NotSupportedError: this browser has no decoder for avc3.42E01F`.
2. On the iPhone: the picture of the PC, said by the person. Besides it a black tile, which went when the iPhone's page was loaded again a while later: taken for the participant of the iPhone's page of before, which had not left yet (not looked into).

Not looked at: whether the PC played the iPhone's video, how many frames a second the iPhone played, and the sound.

## Conclusion

Safari of iOS (versions not taken) decodes the track when asked for `avc1`, where it says it has no decoder for `avc3`. A player asks for the codec of the catalog first, then `avc1` of the same profile and level.
