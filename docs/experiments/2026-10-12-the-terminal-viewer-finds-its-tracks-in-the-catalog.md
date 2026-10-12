# The terminal viewer finds its tracks in the catalog

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal

## Objective

The viewer of the terminal (`spikes/terminal-av`, `xmoq-watch`) was told the names of its tracks by flags, because the demo published no catalog. The demo's publishing page has an MSF catalog now, with `samplerate` and `channelConfig` on its audio track. Whether the viewer can find its tracks there, as the watching page does.

## What was built

- `crates/msf` reads `samplerate` (§5.2.28) and `channelConfig` (§5.2.29) of a track, and has `first_audio_track`. A `samplerate` that is no number and a `channelConfig` that is no string make the document invalid, as in `@xmoq/msf`; its tests read the five rows of `fixtures/msf-01/catalogs.tsv` on these fields, and the Opus and AAC tracks of `catalog-moqlivemock.json` (48000, `"2"`). A number that is no whole number of samples is read as none, where `@xmoq/msf` keeps it.
- `xmoq-watch` without `--video` and `--audio`: it subscribes to the track `catalog` of the namespace and sends a Relative Joining FETCH for its latest Group (the catalog is an Object published before the viewer came; a SUBSCRIBE alone does not bring it), reads the last Object of that Group, and takes the first H.264 video track and the first Opus audio track with LOC packaging. It waits 5 seconds for the catalog, as the page does. With both flags it asks for no catalog, as before.
- What the catalog says of the sound is printed and not acted on: Opus is decoded at 48 kHz whatever it was made from, and the decoder mixes two channels into the one that is played.

## Commands

```sh
spikes/terminal-av/try.sh 10                                  # by the catalog
spikes/terminal-av/try.sh 6 --video video --audio audio       # by names
spikes/terminal-av/try-room.sh 8 ; spikes/terminal-av/try-keys.sh
```

A checkout that had the demo's dependencies from before needs `bun install --frozen-lockfile` once: the demo's pages depend on `@xmoq/msf` now.

## Observed results

One run each; the picture to `/dev/null` or a pseudo-terminal, the sound at volume 0. Nobody saw or heard it.

| Run | Outcome |
|---|---|
| By the catalog, 10 s | `CATALOG tracks=2 video=video codec=avc3.42E01F size=640x360 audio=audio codec=opus samplerate=48000 channel_config=1`; 301 frames drawn, 496 packets of sound, no underrun; picture after sound -16.2 / 0.4 / 6.3 ms (least / median / most) |
| By names, 6 s | no catalog asked for; 181 frames, 295 packets |
| A name the broadcast has not (`--video nothing`) | `SUBSCRIBE of nothing was refused` (REQUEST_ERROR 16), and the viewer ends |
| Through a room, with the relay asking for tokens | the same `CATALOG` line; the FETCH carries the token |
| The page loaded anew while watching (`try-keys.sh`) | the catalog is read again on the new session, and the viewer goes on |

## Not done

- A catalog that changes while the viewer watches: the subscription to it stays open and what comes on it is not read.
- A catalog with several video or audio tracks to choose from by anything but the first; a track in another namespace than the catalog's (`namespace` of a track is read and not followed).
- The rest of `fixtures/msf-01/catalogs.tsv` in `crates/msf`.

## Next action

None from this record.
