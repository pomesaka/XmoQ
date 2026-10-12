# The demo's broadcast has an MSF catalog, and others read it

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the catalog; a viewer in a terminal is to find the demo's tracks by it

## Objective

The demo's pages agreed on the names of their tracks in code. The publishing page now puts out an MSF catalog with a video and an audio track, and the watching page finds its tracks there. Does the demo play as before, also after a session ends, and can the Rust client and `mlmsub` of moqlivemock read the catalog?

Expected, written before the runs: `bun demo/check.ts` says the same as before; after a break the watching page plays again; `xmoq-moqt-client --catalog-video` names the track `video` with `avc3.42E01F` and writes frames; `mlmsub` selects a video and an audio track.

## Environment

macOS 14.4, arm64, loopback. Chromium 156.0.8078.4 (playwright-core 1.64.0), headless. The own relay as `bun demo/serve.ts` starts it, on ports of the run. `mlmsub` of moqlivemock v0.16.1 as the interop run builds it.

## Commands

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384
bun demo/check.ts http://localhost:45381
target/debug/xmoq-moqt-client --mode subscribe --url https://127.0.0.1:45382 --cert-sha256 <hash> \
  --namespace xmoq/demo --track catalog --catalog-video --objects 60 --annex-b-out demo.h264
interop/peers/moqlivemock/.tools/bin/mlmsub -addr https://127.0.0.1:45382 -namespace xmoq/demo \
  -videoname video -audioname audio -duration 4
```

## Result

The catalog the watching page read:

```json
[{"name":"video","packaging":"loc","isLive":true,"role":"video","codec":"avc3.42E01F","bitrate":1500000,"width":640,"height":360,"framerate":30},
 {"name":"audio","packaging":"loc","isLive":true,"role":"audio","codec":"opus","bitrate":67000}]
```

| What | Before | After |
|---|---|---|
| `demo/check.ts`, two runs each | 30 frames and 49 to 50 pieces of sound a second, 0 breaks in 4 s, "THE DEMO PLAYS" | 30 frames and 50 pieces a second, 0 and 1 breaks, "THE DEMO PLAYS" |
| Both pages through `spikes/lossy-link`, cut for 10 s | (as in the record of the reconnect) | session 2 on both pages 4 s after the link was back, 30 frames and 50 pieces a second: the catalog is read anew on the new session |

- **`xmoq-moqt-client --catalog-video`**: fetched the catalog (272 bytes, one Object by the Joining FETCH), printed `CATALOG_TRACK name=video codec=avc3.42E01F width=640 height=360`, and wrote the frames it was asked for (60) to a file of 97,925 bytes, the first with NAL units 7, 8 and 5; its last lines were not read.
- **`mlmsub`**: fetched the catalog by its Joining FETCH, said "catalog uses LOC packaging", selected `video` and `audio`, and saw Groups of both begin (video a second apart, audio 200 ms apart). **Not as expected: without `-videoname` and `-audioname` it finds no track** ("no matching tracks found"): it matches names by the substrings `_avc` and `_aac` unless told. The names of the demo's tracks were not changed for it.

## Not done

- The catalog has no sample rate and no channels for the audio track: `@xmoq/msf` does not write `samplerate` and `channelConfig` (its README has why). The watching page takes 48,000 Hz, mono, from the demo's code.
- The audio's `bitrate` in the catalog, 67,000, is what Chromium's Opus encoder was measured to make, not a setting.
- A catalog that changes while a page watches is not followed: the page reads the first one of a session.
- `mlmsub` was not made to write the tracks out.
