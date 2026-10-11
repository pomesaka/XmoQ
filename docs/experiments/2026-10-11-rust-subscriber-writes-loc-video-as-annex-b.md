# A Rust subscriber writes a LOC video track as an H.264 byte stream

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, Rust MoQ subscriber

## Objective

The Rust client takes the Objects of a video track of LOC (`draft-ietf-moq-loc-04`), reads the Timestamp of each, makes a start code of every length prefix, and writes the frames to a file, beginning with a key frame. The file is to be one a decoder reads.

## Environment

The Rust client and the own relay built from the working copy this record is part of; `mlmpub` of moqlivemock v0.16.1 as `scripts/ci/interop.sh` builds it; `ffprobe` of ffmpeg 8.1. Loopback, macOS.

## Commands

```sh
# 1: from mlmpub, which is a server, without a relay.
mlmpub -addr 127.0.0.1:48810 -sideport 49810 -asset interop/peers/moqlivemock/.tools/assets/test10s
xmoq-moqt-client --mode subscribe --objects 60 --url https://127.0.0.1:48810/moq \
    --cert-sha256 "$(curl -s http://127.0.0.1:49810/fingerprint)" \
    --namespace mlm/msf/clear --track video_400kbps_avc --annex-b-out direct.h264

# 2: through the own relay. The publisher is the Rust client, with the key frame of
# fixtures/loc-04 as Object 0 of five Groups and its Timestamp as an Object Property.
xmoq-relay --port 48820 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
xmoq-moqt-client --mode publish --namespace xmoq/video --track key-frames \
    --payload-hex <fixtures/loc-04/video-key-length-prefixed-from-moqlivemock.hex> \
    --object-property 16=1791690527000000        # `send` five times on its stdin
xmoq-moqt-client --mode subscribe --objects 5 --namespace xmoq/video --track key-frames --annex-b-out relay.h264

ffprobe -v error -count_frames -show_entries stream=codec_name,profile,width,height,nb_read_frames <file>
```

`--annex-b-out <file>` is new. With it `--mode subscribe` prints `VIDEO_FRAME group=... object=... timestamp=... nal=... bytes=...` for every frame it writes in place of `OBJECT`, and `VIDEO_WRITTEN file=... frames=... groups=... skipped=... bytes=...` when it stops.

## Observed results

One run each.

1. 60 frames of 3 Groups, none skipped, 124,039 bytes. The first: `group=1791713538 object=0 timestamp=1791713538000000 nal=7,8,5 bytes=21925`, then `nal=1` frames 40,000 apart in their Timestamps. `ffprobe`: `h264`, `Main`, 1280x720, 60 frames read.
2. 5 frames of 5 Groups, each `nal=7,8,5 bytes=21533` with the Timestamp that was sent. `ffprobe`: `h264`, 1280x720, 5 frames read.

3. **Whole Groups through the own relay**, added afterwards as the run `interop/video-rust.ts` (`bun video-rust.ts`): a page of Chromium 156 encodes 320x180 at 25 frames a second, a key frame a second, and publishes it; the Rust client comes in the middle and takes 60 Objects. One run, as written at the top of the file before it: exit code 0, 60 frames of 3 Groups, none skipped, 5,939 bytes; the first `nal=7,8,5`, an IDR picture in every Object 0 and nowhere else, each frame the next Object ID and 40,000 later in its Timestamp, which is wall-clock time. `ffprobe`: `h264`, 320x180, 60 frames.

## Limitations and blockers

- In runs 1 and 2 no whole Group went through the relay: `mlmpub` is a server and does not connect to one, and the Rust publisher sends one Object a Group. Run 3 is the one with whole Groups.
- `ffprobe` is not on the path of the matrix here or in CI: the run in the matrix says so and checks the frames by their lines alone.
- The track was named on the command line; no catalog was read. Rust has nothing of `@xmoq/msf`.
- The writer takes the next Object ID of the Group it is in, or Object 0 of a later Group with an IDR picture, and skips everything else. Neither run had an Object out of order, so the skipping ran in no run.
- `ffprobe` read the frames; nothing was shown on a screen.
- The Timescale is read and not used: both publishers send a Timestamp alone.

## Decision or next action

Decoding and drawing in a terminal come next.
