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

## After the finding: the painted clock by `Date.now()`, and a sender that follows the wall clock

Two changes. `fromACanvas` in `demo/publish.ts` takes its times from `Date.now()`, as the tone does (`/publish?canvasClock=performance` gives the two clocks back). And the encoders of `@xmoq/media` take `clock: "least"` (`/publish?stamp=least`): the source's times are moved by the smallest of "wall clock less the source's time" among the frames of the last minute, which is the rule the viewer measures its publisher by (`createFastestWay`). The default stays `"first"`.

Flags that make a source's clock wrong on purpose: `/publish?clockSkewPpm=N` (the painted clock's times run N ppm fast against the tone's), `/publish?toneSkewPpm=N` (the tone makes its pieces N ppm faster than the wall clock passes, as a microphone with a clock of its own). 1,000 ppm is 60 ms a minute: 5 minutes of it are what 5 hours of this machine's two clocks were.

### Expected, written before the runs

10 minutes of the default (`Date.now()` for both, `"first"`): when the video came less its time stays within 2 ms of where it began, where it rose by 10 ms before.

5 minutes each, three viewers (50 ms, 150 ms, automatic):

| Publisher | Expected |
|---|---|
| `clockSkewPpm=1000`, first | The video's times run ahead, the viewer's fastest way follows them, and the sound is later against it by 1 ms a second: with 50 ms all of the sound too late from about the first minute on, with 150 ms from about the third |
| `clockSkewPpm=-1000`, first | The video is later by 1 ms a second: every frame after its turn from about 50 s with 50 ms, from about 150 s with 150 ms; the sound whole |
| `clockSkewPpm=1000&stamp=least` | The video's number stays where the sound's is (every frame renews the least); nothing of the sound too late beyond what the default has |
| `clockSkewPpm=-1000&stamp=least` | The video's number rises for a minute to about 60 ms over the sound's (the least is a minute old, and a minute at 1,000 ppm is 60 ms) and **stays there**. With 50 ms the frames are after their turn all the same; at the 17 ppm that was measured it would be 1 ms |
| `toneSkewPpm=1000&stamp=least` | The times of the sound's pieces are 19.98 ms apart for 20 ms of sound. The viewer's seam takes up to 8 ms (`SEAM_SECONDS`) by playing pieces at another speed: **more pieces at another speed, no breaks, nothing too late**. Both numbers of "came" stay |
| `toneSkewPpm=-1000&stamp=least` | The least gives way once a second, by about 1 ms: one piece a second is 21 ms after the one before. The same: pieces at another speed, no breaks. The sound's number about 60 ms up, as the video's above |
| `stamp=least` and no clock made wrong | As the default: nothing more too late, no more breaks, no more pieces at another speed |
| `toneSkewPpm=1000`, first (what it is without) | The sound's times run ahead by 1 ms a second, the fastest way follows them, the video is later and later: frames after their turn as in the second row |

Picture against sound (the marks, as they were played): within 20 ms of the default's where both numbers of "came" agree; off by what the numbers differ where they do not.

Then an hour of `canvasClock=performance&stamp=least`: the video's number stays within a few milliseconds of the sound's for the whole hour; with 50 ms the frames after their turn stay a handful. The renderers: the heap of the pages' scripts (`performance.memory.usedJSHeapSize`) goes up and down and does not grow; whether the resident size levels off is what the run is to say, and is not guessed here.

### Result: 10 minutes of the default

Began 06:07 JST, stopped after 610 s (the hour that was begun was cut to this). The same pages and the terminal viewer as above. Another workspace ran `cargo clippy`, tests and one `mise run check` between 06:08 and 06:13.

| What | 10 s | 2 min | 4 | 6 | 8 | 10 |
|---|---|---|---|---|---|---|
| When the video came less its time, median (all three alike) | -1 ms | -2 | -1 | -1 | -1 | -1 |
| When the sound came less its time, median | 3 ms | 4 | 5 | 6 | 7 | 9 |
| Lateness of 99 in 100 | 16 to 20 ms | 9 | 10 | 12 | 13 | 13 to 14 |

- **As expected**: the video's number stays (it rose by 10 ms in these minutes before). Frames after their turn with 50 ms: 3, all in the first 10 s. The terminal viewer: video came a median of 1.0 ms over the least, and it drew its frames a median of 2.7 ms after they were due.
- **The sound's number goes up by 6 ms and was not expected to**; the first hour has it too, between 2 and 9 ms and down again about every 10 minutes. A reading that fits and was not tested: the tone makes piece `n` at the first tick of a 10 ms timer after `n` × 20 ms of `Date.now()`, the timer keeps the other clock's pace, and so the wait for the tick goes from 0 to 10 ms and over again as the two clocks move. It is bounded by the tick, and is of the demo's tone only.

### Result: clocks made wrong by 1,000 ppm, 5 minutes each

06:28 to 06:49 JST, two at a time on two sets of ports, three watching pages each (no terminal viewer). Another workspace ran tests and one `mise run check` between 06:29 and 06:32. "Came" is the median of when Objects came less their time, video / sound, at the end; the counts are of 300 s (15,000 pieces, 9,137 frames).

| Publisher | Came, video / sound | Sound too late: 50 ms, 150 ms, automatic | Breaks | Pieces at another speed | Frames after their turn: 50 ms, 150 ms | As expected? |
|---|---|---|---|---|---|---|
| `stamp=least`, no clock wrong | 2 / 5 ms | 4, 0, 0 | 1, 1, 1 | 25, 0, 0 | 1, 0 | Yes: as the default |
| `clockSkewPpm=1000`, first | -299 / 3 | 12,794 (none sounds after about 44 s), 7,769 (none after about 144 s), 0 (its buffer went to 320 ms) | 119, 77, 1 | 59, 132, 37 | 1, 0 | Yes |
| `clockSkewPpm=1000&stamp=least` | 3 / 5 | 5, 0, 0 | 2, 0, 0 | 25, 0, 4 | 1, 0 | Yes |
| `clockSkewPpm=-1000`, first | 302 / 3 | 4, 0, 0 | 1, 1, 1 | 24, 0, 285 | 7,743, 4,735 | Yes |
| `clockSkewPpm=-1000&stamp=least` | 62 / 5, from the first minute on | 5, 0, 0 | 2, 1, 0 | 24, 0, 48 | 7,748, 0 | Yes: 60 ms up and no further |
| `toneSkewPpm=1000`, first | 0 / -298 | 4, 0, 0 | 1, 1, 1 | 323, 295, 31 | 7,646, 4,624 | Yes |
| `toneSkewPpm=1000&stamp=least` | 2 / 4 | 6, 0, 0 | 0, 0, 1 | 330, 299, 303 | 1, 0 | Yes |
| `toneSkewPpm=-1000&stamp=least` | 2 / 63, from the first minute on | **13,015 (none sounds after about 40 s)**, 0, 0 | 55, 0, 7 | 25, 239, 272 | 1, 0 | **Not for 50 ms**: see below |

- **A sender with `"least"` holds a source's clock that is wrong, both tracks, both ways**: the number that ran away by 300 ms in 5 minutes stays where it began when the source's clock is fast, and 60 ms above when it is slow.
- **What the viewer makes of sound whose times move by a millisecond now and then: pieces at another speed, and nothing else.** With a tone 1,000 ppm fast or slow the viewer played about one piece a second at another speed (299 and 239 in 300 s with 150 ms), with no break and no piece too late beyond the first seconds'. The seam (`SEAM_SECONDS`, 8 ms) reads the step as the same sound going on. Whether a person hears one piece a second 5% faster was not listened to; at 17 ppm it would be one piece a minute.
- **Expected otherwise, 50 ms and a slow source**: the row said "no breaks, nothing too late" and with 50 ms all of the sound was too late from about 40 s on. The 60 ms that the least of a minute is behind at 1,000 ppm are more than the buffer: the same thing the row above says of the frames, which was written there and not here. **A slow source is behind by what its clock loses in a minute**: 60 ms at 1,000 ppm, 6 ms at 100, 1 ms at the 17 that was measured.
- **The encoder numbers the sound's chunks by the samples and not by the times it is handed** (Chromium 156): with `clockSkewPpm=1000` and `"first"` the tone's `AudioData` carried times 1,000 ppm fast and the sound's number stayed at 1 to 3 ms. This is why the sender counts the sound's times from the samples.
- **Picture against sound**: where both numbers agree the marks were played -16 to 34 ms apart, with `stamp=least` whether a clock was wrong or not. Where the video's number is 60 ms up (`clockSkewPpm=-1000&stamp=least`) the picture was 16 to 48 ms **before** the sound after the first minute (3 ms once): its times are 60 ms early, and it is drawn by them. **With `toneSkewPpm` the marks say nothing**: the tone then makes its beep sooner or later than the painted mark by what its pace is off (304 ms after 5 minutes), which a microphone, hearing the room as it is, would not.

### Result: 43 minutes of `canvasClock=performance&stamp=least`

Began 06:54 JST and **stopped after 2,610 s, not the hour that was meant** (the machine was wanted). The same pages and the terminal viewer as the first hour. Another workspace ran `cargo clippy` and tests a dozen times and `mise run check` four times between 06:58 and 07:28, one pairing of the interop six times at 06:54:52 and twice at 07:23:42, and this workspace `mise run check` once at 06:56.

| What | 10 s | 10 min | 20 | 30 | 40 | 43 |
|---|---|---|---|---|---|---|
| When the video came less its time, median (all three alike) | 2 ms | 3 | 3 | 3 | 3 | 3 |
| When the sound came less its time, median | 9 ms | 4 | 3 | 4 | 2 to 3 | 2 |
| Lateness of 99 in 100 | 12 to 23 ms | 6 to 7 | 15 | 7 to 8 | 14 | 13 |
| Frames after their turn, 50 ms, in all | 2 | 2 | 2 | 2 | 2 | 4 of 79,167 |
| The automatic buffer | 30 ms | 20 | 30 | 20 | 30 | 30 |
| Heap of the script, publishing page | 10.7 MiB | 10.7 | 10.7 | 18.4 | 18.4 | 9.5 |
| Heap of the script, watching pages | 12.8 to 15.4 MiB | 12.8 to 15.4 | 12.8 to 15.4 | 18.4 to 29.8 | 18.4 to 29.8 | 14.5 to 29.8 |
| Renderer of the publishing page, resident | 149 MiB | 183 | 167 | 165 | 169 | 159 |
| Renderers of the watching pages, resident | 312 to 326 MiB | 342 to 349 | 331 to 337 | 327 to 342 | 344 to 348 | 326 to 331 |
| Relay, resident | 15.8 MiB | 15.8 | 15.5 | 15.5 | 15.6 | 15.1 |

- **As expected: with `"least"` the two clocks of the first hour are held.** The video's number stayed at 2 to 3 ms for 43 minutes, where it was at 41 ms after 40 minutes before. With 50 ms, 4 frames after their turn in all (216 after 40 minutes before, and all of them after 50). The automatic buffer stayed at 20 to 30 ms (60 before). The terminal viewer: video came a median of 2.4 ms over the least (59.0 in the first hour), the picture a median of 8.6 ms after the sound.
- **The sound**: 3, 0 and 3 pieces too late of 130,490 (50 ms, 150 ms, automatic); pieces at another speed 22, 1 and 22, which is no more than without `"least"` (54, 38 and 164 in the first hour).
- **One moment, at 41 min 50 s (07:36:34)**: the lateness of 99 in 100 read 30 to 33 ms for all three, a timer of one page was 84 ms late, the automatic buffer went to 50 ms for a reading, and the pages with 50 ms and the automatic buffer each lost 1 and 3 pieces of sound. The busiest other processes in that reading were at 41% and 40% of a core (one of them the window server); the publisher's timer was not late. Taken for the machine; the terminal viewer's 9 underruns and its one frame 127.8 ms over the least are probably of the same moment, which its one line at the end cannot say.
- **The renderers' memory does not grow with time in 43 minutes**: the resident size rose in the first 10 to 20 minutes (by 20 to 35 MiB) and then went up and down within about 20 MiB, ending near where it began. The heaps of the scripts go up and down between 9.5 and 29.8 MiB. What rose in the first hour's 60 minutes (to 349 to 360 MiB) was not followed to the hour here.
- **Not shown**: the last 17 minutes of the hour; a camera and a microphone.
