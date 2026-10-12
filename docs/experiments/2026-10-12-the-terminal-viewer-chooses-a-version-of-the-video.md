# The terminal viewer chooses a version of the video

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2 and the viewer in a terminal: a publisher with two versions of its video (`/publish?ladder=1`, a trial) and a viewer that is not a page.

## Objective

`xmoq-watch` played the first H.264 video track of the catalog, whatever else the catalog had: with the demo's two versions always the large one. That it can be told which version to play, and what the small one does for it behind a link narrower than the large one.

## What was built

- `xmoq-msf` reads the `bitrate` of a track (MSF §5.2.22) and has `video_tracks`: the video tracks a viewer with a given decoder chooses from, in the order of the catalog.
- `xmoq-watch --video-version first|name:<track>|height:<n>|bitrate:<n>`: the first of the catalog (as before, and when the flag is left out), the one of that name, or the largest that is at most so tall or has at most so many bits a second. **Where none is within the limit the smallest is taken**; a version the catalog says no height or bitrate of is not counted, and where it says it of none the first is taken. The rule is `choose` of `crates/watch/src/tracks.rs`, with a table test over the demo's catalog and the one `moqlivemock` sent (three H.264 versions).
- **Not built: a viewer that changes the version by itself** (below).

## Environment

macOS 14.4, arm64, loopback; `xmoq-watch` (release) with `--sound off --medium direct --fit none --out /dev/null --video-timeout-ms 150 --measure-demo-marks on`, 25 s; the demo's publishing page in headless Chromium 156.0.8078.4 with `bitrate=2500000&picture=busy&ladder=1` (640x360 at 2.5 Mbit/s and 320x180 at 0.5 Mbit/s); the own relay with no flag; `spikes/lossy-link/proxy.mjs` between relay and viewer at 2,000 kbit/s, 50 ms each way, queue 100 ms. `crates/watch/scripts/try-narrow-link.sh` with `PUBLISH_QUERY`.

Expected (not written down before the first run, which was a trial of the script): the large version short of frames and some hundreds of milliseconds behind, as the pages are on this link; the small one whole and in time, since it fits the link four times over.

## Result

| `--video-version` | Plays | Frames that came, of about 735 | Drawn a second | The picture came after the fastest, ms (half of the frames; 95 of 100) | The sound came after the fastest, ms (half; 95 of 100) | Times the sound's output ran dry |
|---|---|---|---|---|---|---|
| `first` | 640x360 | 536, 534 | 22.2, 22.1 | 312 and 296; 461 and 429 | 68 and 51; 94 and 77 | 1,516, 766 |
| `name:video-low` | 320x180 | 736, 737 | 30.5, 30.5 | 10 and 10; 32 and 30 | 10 and 10; 19 and 20 | 0, 0 |
| `bitrate:1000000` | 320x180 | 737 | 30.5 | 10; 28 | 4; 19 | 0 |

- **As expected.** With the small version every frame comes, 10 ms after the fastest, where the large one loses a quarter of its frames and comes 300 ms late.
- **The sound gains too**: its output ran dry 766 and 1,516 times in 24 s with the large version on the link (the sound is counted and not heard in these runs, `--sound off`), and never with the small one.
- `bitrate:1000000` chose the small version, as the rule has it.

`mise run check` and `mise run interop` passed (the numbers are in the pull request). A person did not look at either picture.

## A viewer that changes the version by itself: what it would take

Not built. The rule is the small part: `createQualitySwitch` of `@xmoq/media` is some 150 lines without I/O, and it could be put in the library of `xmoq-watch` with a table under `fixtures/` that both implementations read, as the playout has. The work is in the binary:

- a second subscription on the session while the first goes on (the reader takes one video track and one audio track now), and its end when the change is made or a try fails;
- a second H.264 decoder for the version that is tried, since a try decodes without drawing;
- the lateness of each version apart (the clock has one for the picture);
- drawing from the new version at the right frame, and the size of the picture changing under the terminal's layout.

That is the session, the picture and the main loop of the binary, which another change is touching now. Until then `--video-version` is chosen by whoever starts the viewer.
