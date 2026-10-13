# An hour of the demo

Date: 2026-10-13

## Objective

Nearly every run so far lasted 20 seconds to a few minutes. The first of 30 minutes ([record](2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md)) had something that is not explained: in its last 5 minutes all three viewers at once were 68 to 74 ms late. Run an hour with the default values and no link in between, write the state down every 10 seconds, and look for what grows with time and for a moment at which all are late together.

## What is run

One publishing page (the painted clock and the tone), three watching pages (buffers of 50 ms, of 150 ms, the automatic one) in one headless Chromium, the terminal viewer (`xmoq-watch`, 50 ms, its picture written to `/dev/null`, its sound at volume 0), the own relay with no flag, on one machine. Every 10 s:

- each watching page: when Objects came less their time, the median of the last 200 of each track (before the fastest way is taken off), the lateness of 99 in 100 after it, the buffer, pieces too late, frames after their turn, breaks in the sound, pieces at another speed, frames that wait;
- the publishing page: frames handed to the encoder, chunks it gave, frames not encoded and chunks not sent by their reason (`encoder-behind` is what a full queue of the encoder gives; the queue's length itself is not read);
- every page: the longest a timer of 50 ms was late (what a pause of the page's thread looks like), the heap (`performance.memory`);
- memory and CPU by `ps`: the relay, the terminal viewer, each process of Chromium; the five busiest processes of the machine that are not of the run, and its load.

**Not read**: the bytes the relay keeps (it has no such line; its resident memory stands for it), the gaps between the publisher's frames one by one (the count in 10 s and the timer's lateness stand for them), the terminal viewer's numbers over time (it prints them at its end; over time there is its memory and CPU).

## Expected, written before the run

- **Should not grow**: the relay's memory (it keeps the latest Group of each track, a second of video and a few pieces of sound); the heaps of the pages (the measuring of the watching page keeps the last 200 values; `diag` is off); the frames that wait; the lateness; the median of when Objects came less their time, **except by what the page's two clocks do**: the publisher times its frames by `performance.now()` and the viewers measure with `Date.now()`, which the machine's clock correction moves against each other, a few milliseconds an hour at most.
- **Where it may grow**: the resident memory of Chromium's renderers and its GPU process (decoded frames, WebGL or canvas); the terminal viewer's memory if anything in it is kept by the Object; the publishing page's heap if its counters keep lists.
- **A moment when all are late**: if it comes again, the publisher's numbers say whether it began there (fewer frames in the 10 s, a timer late in the publishing page, `encoder-behind`); if the publisher is steady and the viewers' timers are late, it is the browser or the machine (the busiest other processes and the load say which); if only what came is late and no timer is, it is between them: the relay or the loopback. One Chromium holds all four pages here, so a pause of its GPU or network process reaches all at once, which two machines would not share.
- The sound: no piece too late with 150 ms; a few with 50 ms and the automatic buffer in an hour would not surprise.

## Environment

macOS 14.4 on arm64, Chromium 156.0.8078.4 (headless, playwright-core 1.64.0), the own relay and `xmoq-watch` of this workspace, loopback. Ports 45381 (pages), 45382 (relay), 45384 (room). Began 04:57:08 JST, 3,600 s. The machine was shared: another workspace ran `cargo clippy`, tests and `mise run check` a few times between 04:55 and 05:10 (a minute or two each), and one pair of the interop (a relay and three small clients, 8 MiB once) from 05:00:41 to 05:00:46. Its load was 0.8 to 4.7.

## Result

### What did not grow

| What | First 10 s | After 10 min | After 30 min | After 60 min |
|---|---|---|---|---|
| Relay, resident memory | 16.0 MiB | 15.7 MiB | 15.6 MiB | 15.7 MiB (most: 16.6) |
| Relay, CPU | 4% | 5% | 7% | 7% (3.7 to 9.1 throughout) |
| Terminal viewer, resident memory | 36.7 MiB | 34.5 MiB | 34.4 MiB | 34.4 MiB (most: 36.9) |
| Terminal viewer, CPU | 10% | 11% | 19% | 19% (9.8 to 21.7) |
| Chromium's GPU process | 95 MiB | 91 MiB | 88 MiB | 88 MiB |
| Publishing page: frames handed to the encoder less chunks it gave, at most | 1 | 1 | 1 | 1 |
| Publishing page: frames in 10 s | 302 to 304 throughout | | | |
| Watching pages: frames that wait, at most | 5 (150 ms), 2 (the others) | | | |
| Every page: the longest a 50 ms timer was late | 80 ms once, in the first 10 s of the page with 50 ms; 1 to 6 ms in every other reading | | | |
| When the **sound** came less its time, median | 3 ms | 3 ms | 3 ms | 3 ms (2 to 9 throughout) |

No frame was left unencoded and no chunk unsent. The heaps of the pages read 9.5 to 35.6 MiB and went down as well as up.

### What grew

| What | First 10 s | After 10 min | 20 | 30 | 40 | 50 | 60 |
|---|---|---|---|---|---|---|---|
| When the **video** came less its time, median (all three pages alike) | 1 ms | 10 | 21 | 31 | 41 | 51 | 60 |
| Lateness of 99 in 100 (all three alike) | 7 to 18 ms | 14 | 25 | 34 | 43 | 54 | 65 |
| Frames after their turn, 50 ms, in all | 2 | 2 | 4 | 45 | 216 | 2,296 | 20,413 of 109,167 |
| Frames after their turn, 150 ms | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| Frames after their turn, automatic | 0 | 75 | 180 | 299 | 408 | 526 | 647 |
| The automatic buffer | 20 ms | 30 | 40 | 50 | 60 | 70 | 80 |
| Renderer of the publishing page | 149 MiB | 175 | 177 | 178 | 183 | 186 | 184 |
| Renderers of the watching pages | 312 to 323 MiB | 342 to 346 | | 317 to 351 | | | 349 to 360 |

The video's number rose by 1 ms a minute, never by more than 1 ms between two readings 10 s apart: no step, no moment. The terminal viewer says the same at its end: video came 57.2 to 92.8 ms over the least (median 59.0), sound 0.0 to 15.4 (median 2.4); it drew all 109,207 frames, a median of 5.5 ms after they were due, left no piece of sound out, and had no underrun.

The sound in the hour (180,000 pieces): 2 too late with 50 ms (both in the first 10 s), 0 with 150 ms, 0 with the automatic buffer; breaks 0, 1 (in the first 10 s), 0.

### Why the video is later and later

The publishing page's painted clock times its frames by `performance.now()` and its tone times its pieces by `Date.now()` (`demo/publish.ts`, `fromACanvas` and `tone`). The viewer measures when Objects came with `Date.now()`, so the sound keeps the fastest way where it is and the video, timed by a clock that runs 17 parts in a million slower on this machine in this hour, comes later against it by 60 ms an hour. The two clocks read side by side for 5 minutes right after the run (in Bun, not in Chromium; every 30 s at the instant `Date.now()` steps) moved the same way, steadily, by 2.85 ms: 9.5 parts in a million, about half of what the hour had. The rate is the machine's correction of its clock and is not one number.

This is the case the summary names as not handled (a publisher whose video and sound are timed by two clocks), and it is in the demo's own synthetic source. **Nothing of the relay, the transport or the playout is behind it.**

## What this says

- **Expected and so**: nothing of the relay, the terminal viewer, the encoder's queue or the pages' timers grew in an hour. The sound was whole with every buffer.
- **Expected otherwise**: "a few milliseconds an hour at most" between `performance.now()` and `Date.now()` was wrong by more than ten times: 60 ms in this hour. With the default 50 ms that is every frame after its turn from about the 50th minute on (9,062 of 9,091 in the last 5 minutes); the picture is still drawn, later than the sound by what the clocks have moved.
- **A moment when all are late did not come.** What the run of 30 minutes had in its last 5 minutes (a lateness of 13 to 20 ms for 25 minutes, then 68 to 74 ms for all three viewers alike, frames after their turn, the sound whole) has the same shape as this, only faster: it is what a correction of the machine's clock within those minutes would do, since it moves `Date.now()` and not `performance.now()`. **That is a reading, not a finding**: the machine's log of that correction was not read.
- **Not shown**: whether the renderers' memory levels off (the watching pages rose by about 30 MiB in the first 10 minutes and by about 14 MiB in the 50 after); what a camera and a microphone do (the procedure in the [record of the clocks](2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md) was not run); anything over a real network.

## Next action

- Time the painted clock's frames by `Date.now()`, as the tone is (one line in `demo/publish.ts`, not done here), and run the hour again: the video's number should then stay where the sound's is.
- The question that stays is the one already open: a real camera against a real microphone.
