# The terminal viewer at 720p and 1080p

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal

## Objective

The demo's publishing page sends at 640x360, 1280x720 or 1920x1080 now (`/publish?size=`). What the viewer of the terminal (`spikes/terminal-av`, `xmoq-watch`) does with the larger two: how long the decoder takes, how much is written for the terminal, whether it keeps up, and whether writing the picture smaller helps.

## Environment

One machine, macOS on Apple silicon, loopback. The demo of this working copy on ports of its own, its publishing page in headless Chromium at each size (H.264, 30 frames a second, 1.5, 3 and 6 Mbit/s), the clock's picture. `xmoq-watch` with `--release`: `openh264` 0.9.8 decodes, the sound is at volume 0. **The picture is written to `/dev/null`: what a terminal does with it is not in any number here, and nobody looked.**

## What was added

- `--fit none|<width>x<height>`: a frame with more pixels than that is made smaller by a whole number each way before it is written, every pixel the mean of a square of the frame. Without the flag, and with the picture going to a terminal that says how many pixels its window has (`TIOCGWINSZ`), the limit is that window less the lines under the picture; where the terminal does not say, or with `--out`, the frame is written as it is. The cells the picture takes are counted from the terminal's own cell where it says it.
- `AV_PLAYED` has the size of the frames, by how much they were made smaller, and what was written for a frame.
- `SIZE=720 spikes/terminal-av/try.sh` has the page send at that size. With `--medium file` and nobody to read the files, `XMOQ_WATCH_REMOVES_FILES=1` removes each when the next is written, as a terminal would.

## Commands

```sh
SIZE=1080 spikes/terminal-av/try.sh 15 --medium direct
SIZE=1080 XMOQ_WATCH_REMOVES_FILES=1 spikes/terminal-av/try.sh 15 --medium file --out /dev/null --fit 960x540
```

## Observed results

One run of 15 seconds each. In every run all 453 or 454 frames that came were drawn, at 30.3 or 30.4 a second. CPU is user and system time of the viewer over the 15 seconds, of one core.

| Sent | Written as | Medium | Decode, ms a frame | Shrink and write, ms a frame | Written a frame | A frame after it was due, ms (median / max) | CPU | Resident, MB |
|---|---|---|---|---|---|---|---|---|
| 640x360 | as it is | direct | 0.75 | 0.88 | 0.9 MB | 7.7 / 11.8 | 16% | 28 |
| 640x360 | as it is | file | 0.75 | 0.87 | 0.7 MB | 7.9 / 15.3 | 16% | 26 |
| 1280x720 | as it is | direct | 2.32 | 2.86 | 3.6 MB | 9.3 / 13.9 | 25% | 44 |
| 1280x720 | as it is | file | 2.48 | 1.54 | 2.7 MB | 7.9 / 13.4 | 21% | 37 |
| 1280x720 | 640x360 | direct | 2.45 | 2.34 | 0.9 MB | 8.9 / 13.5 | 24% | 39 |
| 1920x1080 | as it is | direct | 4.03 | 5.26 | 8.1 MB | 11.2 / 16.6 | 36% | 74 |
| 1920x1080 | as it is | file | 4.48 | 2.36 | 6.1 MB | 8.7 / 22.7 | 27% | 53 |
| 1920x1080 | 960x540 | direct | 4.23 | 4.21 | 2.0 MB | 10.0 / 19.5 | 33% | 59 |
| 1920x1080 | 640x360 | direct | 4.34 | 3.11 | 0.9 MB | 8.9 / 13.7 | 30% | 56 |
| 1920x1080 | 960x540 | file | 4.22 | 3.60 | 1.5 MB | 9.5 / 14.4 | 32% | 55 |
| 1920x1080 | 640x360 | file | 4.41 | 3.14 | 0.7 MB | 9.2 / 14.0 | 31% | 54 |

- **The viewer keeps up at every size**: decoding a frame of 1080p takes 4 to 4.5 ms and writing it 2.4 to 5.3 ms, of the 33 ms a frame has.
- **What it hands the terminal grows with the pixels**: 8.1 MB of escape codes a frame at 1080p with `direct`, 243 MB a second; 6.1 MB a frame in a file with `file`. Whether a terminal draws that 30 times a second is the open part.
- **Making the picture smaller cuts what is written to a quarter or a ninth, and costs the viewer about what it saves it**: with `direct`, 5.3 ms a frame becomes 4.2 and 3.1; with `file`, where writing is a copy, 2.4 ms becomes 3.6 and 3.1, more than before. It is worth it for the terminal's sake or not at all.
- Picture and sound: the first frame with the mark was written 3 to 39 ms after the sound of the mark was played (medians of 7 or 8 marks a run), more at the larger sizes, where the frame is written later by what writing takes. Not corrected for.
- The sound had 0 to 4 underruns in a run of 15 seconds, at every size alike, where the runs of the days before had none. Not looked into.

## Not done

- **Any of it in a terminal.** The numbers that decide this, how many frames Ghostty shows of 1080p and what it costs it, need a person.
- The limit taken from the terminal's window (the default in a terminal): the runs gave the limit by `--fit`, and the pseudo-terminal of `try-keys.sh` says no pixels.
- A size that changes while the viewer watches: a frame of another size than the first is left out.
- Making the picture smaller by other than a whole number; decoding at a lower size (`openh264` decodes the whole frame).
- The low version the demo publishes beside the main one (`video-low`, 320x180): the viewer takes the first video track of the catalog.

## Next action

A person runs `spikes/terminal-av/watch.sh` against `/publish?size=1080` in Ghostty, once as it is and once with `--fit 960x540`, and reads `fps=` in `AV_PLAYED` and looks at the picture.
