# The default playout buffer: 50 ms, 150 ms, or one that follows the link

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2; the open decision on the default of the playout buffer ([open decisions](../open-questions.md)). Measurements for that decision. No code is changed.

## Objective

The watching page and the terminal viewer hold every Object 50 ms beyond the fastest way before they play it. [On a link narrower than the video that loses three of four pieces of the sound](2026-10-13-the-sound-behind-the-video-on-a-narrow-link-and-priorities.md). The choices are to leave it, a fixed 150 ms, or the automatic buffer (`?buffer=auto`, `--buffer-ms auto`), which nobody has listened to. The numbers that were missing:

1. The fixed 50 ms beside 150 ms and the automatic one on links that lose packets.
2. How long the automatic buffer stays large after a link was bad and is good again.
3. The first seconds on a narrow link.
4. The same as 1 for the terminal viewer.

## Expected, written before the runs

1. **Loss, 50 ms each way, room to spare.** A piece behind a lost packet comes a round trip later, 100 ms and more, so 50 ms does not cover it and 150 ms mostly does.
   - 1%: with 50 ms 20 to 60 of 1,000 pieces too late and 10 to 30 breaks; with 150 ms none or one; automatic 5 to 20 too late and 5 to 10 breaks, standing at 110 to 140 ms (what an earlier run had).
   - 5%: with 50 ms 150 to 300 too late; with 150 ms 60 to 130 (what an earlier run had); automatic under 10, standing near its limit of 400 ms, with the picture about 450 ms after it was made where the fixed ones have 100 to 220.
2. **A link that is good, narrow for 20 s, and good again.** The buffer goes to 400 ms within a second or two of the link getting narrow. When the link is good again it stays there for about 10 s, the time the rule looks back over, and then comes down 20 ms a second: back at its least about 28 s after the link got good.
3. **The first seconds on a narrow link.** 50 ms: three of four pieces too late from the first second on. 150 ms: none. Automatic: as 50 ms for the first second (it begins at 50 ms and moves once 50 Objects have come, which is under a second of video and sound), then none: some tens of pieces lost at the beginning and no more.
4. **The terminal viewer** shows the same pattern as the page in 1: most left out with 50 ms at 5%, few with 150 ms, fewest with the automatic one.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; `xmoq-watch` (release) with `--sound off`; the own relay with no flag, **Cubic** (its default), and one set with `--congestion bbr`; `spikes/lossy-link/proxy.mjs` between relay and viewer, 50 ms each way. With loss: no limit, and the demo's clock (about 450 kbit/s). Narrow: 2,000 kbit/s with a queue of 100 ms, and the busy picture at 2.5 Mbit/s. The video asks for its default timeout of 150 ms. Sound: Opus, 50 pieces a second, ten a Group.

## Result

### 1. The page on a link that loses packets

20 s counted, after 5.5 s of playing: 1,000 pieces of sound, 600 frames. Three runs each, the three buffers in turn.

| Loss | Buffer | Where it stood at the end, ms | Pieces too late, of 1,000 | Breaks in the sound | Frames shown, of 600 | A frame shown after it was made, ms |
|---|---|---|---|---|---|---|
| 1% | 50 ms | 50 | 100, 137, 92 | 58, 77, 47 | 555, 530, 553 | 106 to 110 |
| 1% | 150 ms | 150 | 0, 0, 0 | 2, 0, 0 | 586, 596, 602 | 207 to 208 |
| 1% | automatic | 140, 160, 110 | 6, 6, 4 | 5, 5, 5 | 604, 592, 605 | 203, 268, 166 |
| 5% | 50 ms | 50 | 469, 420, 425 | 164, 157, 186 | 318, 301, 317 | 113 to 116 |
| 5% | 150 ms | 150 | 88, 57, 53 | 23, 25, 16 | 327, 395, 376 | 208 to 210 |
| 5% | automatic | 400, 400, 400 | 3, 9, 0 | 2, 9, 0 | 497, 439, 430 | 458 to 459 |
| 5%, BBR (one run) | 50 ms | 50 | 137 | 67 | 517 | 110 |
| 5%, BBR | 150 ms | 150 | 0 | 0 | 591 | 209 |
| 5%, BBR | automatic | 170 | 9 | 5 | 586 | 234 |

- **With 50 ms the sound is broken at 1% already, more than expected**: 92 to 137 pieces of 1,000 too late and 47 to 77 breaks in 20 s, two to four a second (20 to 60 pieces were expected). At 5% nearly half of the sound is left out.
- **150 ms is whole at 1% and loses 5 to 9 of 100 at 5%**, as expected. The automatic one is whole at 5% and has 5 breaks in 20 s at 1%, where 150 ms has none or two.
- **The automatic buffer costs delay where the link is bad**: 460 ms from the making of a frame to the screen at 5%, against 210 and 115. At 1% it stands near the 150 ms.
- More frames are shown the longer the buffer is, at both rates of loss: a frame that comes after its turn in a Group is drawn late or not at all.
- With BBR at 5% every buffer does as it does with Cubic at 1%: the congestion control decides how late things come, and the buffer what is made of it.

### 2. A link that is good, narrow for 20 s, and good again

The automatic buffer, read every second; the link gets narrow at second 10 and is good again from second 30. Two runs.

| Second | 1 to 9 | 10 | 11 | 12 to 38 | 39 | 45 | 50 | 55 | 57 and on |
|---|---|---|---|---|---|---|---|---|---|
| Buffer, ms (run 1) | 150 down to 70 | 70 | 330 | 400 | 390 | 270 | 170 | 70 | 30 |
| Buffer, ms (run 2) | 140 down to 50 | 30 | 330 | 380, then 400 | 390 | 270 | 170 | 70 | 30 (230 from second 66) |

- **Up within 2 s, and 24 and 31 pieces of sound too late on the way** (all of them as the link got narrow; none after).
- **Down as expected, a little sooner**: it stays at 400 ms for 8 s after the link is good again and then comes down 20 ms a second, to its least 26 s after the link got good (28 s were expected). For those 26 s the viewer is further behind than the link asks.
- The least was 30 ms here, not the 40 ms of an earlier record; the rule has "10 ms more than what 99 of 100 came within", which is what the link gives.
- In run 2 the buffer went to 230 ms 36 s after the link was good, with nothing changed on the link, and stayed there to the end of the run (10 s). Not looked into: one late Object in the window is enough by the rule.

### 3. The first seconds on a narrow link

Pieces of sound too late and sounded, in each of the first 10 seconds after the sound was turned on. Two runs; the second is like the first.

| Buffer | Second 1 | Seconds 2 to 10 |
|---|---|---|
| 50 ms | 45 too late, 7 sounded | 1 to 49 too late in each, 276 of about 460 in all |
| 150 ms | 10 too late, 44 sounded (4 and 45 in the other run) | none too late |
| automatic | 24 too late, 28 sounded (25 and 28); the buffer at 400 ms by the end of the second | none too late |

- As expected: the automatic buffer loses half a second of sound at the beginning and none after; 150 ms loses a tenth to a fifth of a second; 50 ms goes on losing.

### 4. The terminal viewer on a link that loses packets

`xmoq-watch --buffer-ms N --sound off`, 25 s (about 1,200 packets of sound); three runs each. The script is `crates/watch/scripts/try-narrow-link.sh` with the link's loss set in place of its rate (a copy outside the repository; the script itself has no flag for loss).

| Loss | `--buffer-ms` | Packets left out as too late | The sound's output ran dry | Sound played, of 24 s | Where the buffer went |
|---|---|---|---|---|---|
| 1% | 50 | 73, 19, 68 | 136, 42, 134 | 22.5, 23.5, 22.5 | 50 |
| 1% | 150 | 0, 0, 0 | 0, 0, 2 | 23.9, 23.9, 23.6 | 150 |
| 1% | auto | 5, 2, 10 | 10, 9, 19 | 24.1, 23.9, 23.8 | up to 160, 310, 160 |
| 5% | 50 | 303, 275, 292 | 614, 559, 563 | 17.1, 17.1, 16.7 | 50 |
| 5% | 150 | 23, 68, 10 | 49, 124, 30 | 22.2, 21.6, 22.8 | 150 |
| 5% | auto | 16, 35, 0 | 36, 30, 10 | 23.7, 23.2, 23.2 | up to 400 |

- The same pattern as the page, as expected, and milder at 1% and 50 ms (19 to 73 packets of 1,200, where the page has 92 to 137 of 1,000). Why the page loses more there was not looked into.

`mise run check` passed; no code is changed.

## The four choices, by these numbers

| Choice | What the numbers say | What they do not say |
|---|---|---|
| Stay at 50 ms | Least delay where nothing is wrong (a frame on the screen about 110 ms after it was made at 50 ms each way). **The sound breaks on every link that was tried but a clean one**: 2 to 4 breaks a second at 1% loss, nearly half of the sound gone at 5%, three of four pieces gone behind a link narrower than the video | Whether a real path is ever as clean as loopback; none was tried |
| A fixed 150 ms | Whole at 1% loss and behind a narrow link, 5 to 9 of 100 pieces lost at 5%. Costs 100 ms everywhere, also where nothing is wrong (about 210 ms to the screen) | Whether 100 ms more matters to somebody who watches; paths with more than 50 ms each way, where the pieces behind a lost packet come later than 150 ms (at 100 ms each way and 5% loss half of the pieces came 210 to 300 ms late in an earlier record) |
| The automatic buffer | The least delay on a clean link (it stands at 30 to 40 ms) and the sound whole on every bad one. **It pays with delay when the link is bad, up to its limit of 400 ms, and for 26 s after the link is good again**; 5 breaks in 20 s at 1% loss where 150 ms has none; half a second of sound lost when a viewer begins on a narrow link, and as the link turns bad. Behind a link narrower than the video it puts picture and sound 480 ms behind together | **How it sounds while it moves**: pieces are played up to a twentieth slower or faster then (54 to 232 pieces in 20 s at 1% loss), and nobody has listened. Whether going to 230 ms for one late Object, as in one run, is common |
| The automatic one, after somebody has listened | The same numbers; the one thing missing above is what a person hears | |

**What the numbers favour** (the decision is the project owner's):

- **Not 50 ms.** It is the one choice that breaks the sound at 1% loss, and the delay it saves is 100 ms.
- **Between 150 ms and the automatic one, the automatic one is better by these numbers on a clean link and on a very bad one, and 150 ms on a link that loses a little.** What stands against the automatic one is not in the numbers: its changes of speed have not been heard, and it holds a viewer back for half a minute after a bad moment.
- A way that needs no listening first: **150 ms now**, which mends what was found broken and has nothing unheard in it, and the automatic one as the default once a person has listened to it with the steps below. The limit of the automatic one (400 ms) and how fast it comes down (20 ms a second) are values of their own, and the 26 s say that the second deserves a look before it is the default.

## Listening to the automatic buffer

Nothing of the machine is changed: the link is the proxy of this repository, a process that is started and stopped.

1. `mise run demo` (the relay on 4443, the pages on 8080).
2. In another terminal, the link: `node spikes/lossy-link/proxy.mjs --listen 4444 --to 4443 --control 4445`. It ends with Ctrl-C, and nothing is left behind.
3. A publisher with a microphone, so that there is speech to hear: `http://localhost:8080/publish?audio=mic`; and the page without the flag, whose tone goes up and down and shows a change of speed as a change of pitch. Both are worth a minute.
4. The viewer through the link, on headphones: `http://localhost:8080/watch?via=4444&buffer=auto`, and "sound" pressed. The first line under the picture has the buffer as it stands.
5. The links to listen to, a minute each, set with `curl`:
   - clean, 50 ms each way: `curl 'http://127.0.0.1:4445/set?loss=0&delay=50&rate=0'`. The buffer comes down to 30 to 40 ms; what to listen for is the first ten seconds, while it does.
   - 1% loss: `curl 'http://127.0.0.1:4445/set?loss=0.01&delay=50&rate=0'`. The buffer moves between 110 and 160 ms; this is where it changes speed most often, and where about one break in four seconds is left.
   - narrow, with `&picture=busy&bitrate=2500000` on the publishing page: `curl 'http://127.0.0.1:4445/set?loss=0&delay=50&rate=2000&queue=100'`. The buffer goes to 400 ms; then back to the clean link, and 26 s of coming down to listen to.
6. The same with `buffer=150` and `buffer=50` in place of `buffer=auto`, for what it is compared with.
7. Two variants of the automatic buffer that come down sooner ([record](2026-10-13-the-automatic-buffer-what-raises-it-and-how-fast-it-comes-down.md)): `buffer=auto&autoDown=quarter`, and `buffer=auto&autoDown=quarter&autoWindow=3000`, on the narrow link and back.

What to say afterwards: whether the changes of speed are heard, on a voice and on the tone; whether the half second lost at the beginning matters; whether half a minute further behind after a bad moment does.
