# The automatic buffer: what raises it, and how fast it comes down

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2; the open decision on the default of the playout buffer. Two things about the automatic buffer that [the record of the default](2026-10-13-the-default-playout-buffer.md) left: a run in which it went to 230 ms on a link nobody touched, and the 26 s it takes to come down after a link was bad.

## The rule as it is

`createPlayoutClock` of `@xmoq/media` with `auto` (and the terminal viewer by the same table, `fixtures/playout/`): the buffer wanted is what 99 of 100 of the frames and pieces of the last 10 seconds came within, after the fastest, and 10 ms more, up to 400 ms. Twice a second the buffer goes to it at once where that is more, and 10 ms towards it where it is less.

## 1. What raised it to 230 ms

**That record's "one late Object in the window is enough by the rule" is wrong**, read against the rule before anything was run: the buffer goes by the 99th of 100. In 10 seconds a viewer has about 800 frames and pieces (30 and 50 a second), so it takes 8 of them, a hundredth, coming 220 ms late to put the buffer at 230 ms. One Object does nothing.

Expected of a look at it: what does it is **a pause of a tenth of a second or more somewhere on the way** (the page, the relay, the machine), after which everything that waited comes at once: 100 ms of waiting is 8 Objects. The buffer then stands at the length of the pause for the 10 s the rule looks back over, and comes down after. To be seen: in a long run on a link nobody touches, each time the buffer goes up, how many Objects of that second came how late.

## 2. Coming down sooner: two other rules, expected before the runs

| | The rule for coming down | Expected |
|---|---|---|
| as it is | What is wanted is of the last 10 s; down 10 ms twice a second | Good, narrow for 20 s, good: at its least 26 s after the link is good (8 s at the limit, then 18 s of steps), as measured |
| quick steps | The same 10 s; down a quarter of what is left twice a second, 10 ms at least | The 8 s at the limit stay, the steps take about 5 s: at its least about 13 s after. No more sound lost than as it is, on any link: it never goes lower than what the 10 s want. More pieces played faster at once (the buffer moves up to 90 ms in a step), over a shorter time |
| short memory | What is wanted is of the last 3 s; quick steps | At its least about 6 s after the link is good. **It forgets a bad moment in 3 s**, so where the bad moments come further apart than that the buffer is down again when the next one comes: on a link that loses 1% steadily more pieces too late than as it is (twice as many or more), and on a link whose loss comes and goes about as many as with a fixed buffer of 50 ms in the first second of each bad spell |

What is counted: the seconds from the link being good to the buffer at its least, the pieces of sound too late in that time and over the run, and the pieces played slower or faster.

If quick steps does what is expected it costs nothing but how it sounds, which is not known for any of the three, and is worth having behind a flag; short memory is worth it only if the sound it loses is little.

## What was built

Behind flags, and the automatic buffer without them is as it was (`fixtures/playout/` and the terminal viewer are not touched):

- `createPlayoutClock` takes `autoWindowMs` (10,000 when left out) and `autoDown: "quarter"` (the buffer comes down a quarter of what is left at a tick, 10 ms at least). Table tests in `packages/media/test/playout.test.ts`, with one more for what raises the buffer: one Object half a second late does not, and a way that stands still for 0.3 s does.
- `/watch?buffer=auto&autoDown=quarter` and `&autoWindow=MS`.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag, Cubic; `spikes/lossy-link/proxy.mjs` between relay and viewer, 50 ms each way. The busy picture at 2.5 Mbit/s where the link gets narrow (2,000 kbit/s, queue 100 ms), the clock with loss. 50 pieces of sound a second.

## Result

### 1. What raises it

- **By the rule** (a test, with numbers worked out before it ran): of 1,000 Objects in the window, one that comes 500 ms late leaves the buffer where it is; a way that stands still for 0.3 s, so that 30 Objects come at once, raises it to 220 ms, which is what the tenth latest of them came within and 10 ms more. For a viewer with 80 Objects a second a buffer of 230 ms is a standstill of about 0.3 s.
- **Not seen again**: three runs of two minutes on a link nobody touched had the buffer between 20 and 40 ms throughout, and no second in which a piece of sound came 50 ms after the fastest. What stood still for 0.3 s in that one run is not known. Everything is on one machine, with a headless browser that encodes and another that decodes; a standstill of the machine is as likely as one of the path.
- **One run of the trials below was worse, and is not explained either**: with the short memory the buffer stood at 370 ms from before the link got narrow to the end of the run, 65 s, and 859 of the 1,000 pieces of the narrow 20 s were too late. Three more runs of the same did not show it. A buffer that does not move while nearly every piece is too late is not what the rule gives for any arrivals, so what was wrong is elsewhere (the clock of that page, or what it was told of the arrivals), and it is the thing to look at before the automatic buffer is anybody's default. It was looked for afterwards in 70 more runs and not found ([record](2026-10-13-the-run-in-which-the-buffer-stood-still.md)).
- **Is it right that a standstill of 0.3 s holds the viewer back for 10 s and more?** By what it is for, yes for the 10 s: a path that stood still once may do so again, and the next time the sound is whole. What is too long is the coming down after, which is the second part.

### 2. Coming down

Good, narrow from second 10 to 30, good again. "Down" is the buffer at 50 ms or less.

| Rule | The buffer 5, 10, 15, 20 s after the link is good, ms | Down after | Pieces too late while narrow | Too late after the link is good | Pieces played slower or faster, the whole run |
|---|---|---|---|---|---|
| as it is (2 runs) | 400, 370 to 390, 270 to 290, 170 to 190 | 27 to 28 s | 20, 20 | 0 | 723, 749 |
| quick steps (2 runs) | 400, 350 to 360, 40, 30 | between 10 and 15 s | 19, 28 | 0 | 621, 698 |
| short memory (4 runs that went as the rule has it) | 180 to 250, 30 to 40, 30, 20 to 30 | between 5 and 10 s | 17 to 22 | 0 | 735 to 792 |

A link that loses 1% throughout, 30 s (1,500 pieces):

| Rule | Too late | Breaks | Played slower or faster | The buffer: mean, least to most, ms |
|---|---|---|---|---|
| as it is | 0, 0 | 4, 0 | 233, 144 | 185 and 153; 120 to 250 |
| quick steps | 11, 9 | 5, 9 | 217, 161 | 204 and 152; 120 to 250 |
| short memory | 16, 12 | 3, 8 | 311, 333 | 126 and 161; 70 to 370 |

A link whose loss comes and goes: 5% for 5 s, none for 10 s, four times, 60 s (3,000 pieces):

| Rule | Too late | Breaks | Played slower or faster | The buffer: mean, least to most, ms |
|---|---|---|---|---|
| as it is | 14, 28 | 8, 11 | 403, 481 | 360; 100 to 400 |
| quick steps | 59, 28 | 17, 13 | 946, 918 | 333 and 304; 120 to 400 |
| short memory | 76, 61 | 27, 28 | 1,633, 2,012 | 201 and 196; 30 to 400 |

Against what was expected:

- **Quick steps is down between 10 and 15 s after the link is good, where the rule as it is takes 27**, as expected (13 were expected), and loses nothing after the link is good. (The buffer was written down every 5 s for these rows, so no nearer than that.)
- **"No more sound lost than as it is, on any link" was wrong.** Where the loss comes and goes, the buffer as it is has not come far down when the next bad spell begins, and that is what saves its sound: 14 and 28 pieces too late against 28 and 59, and half as many pieces played at another speed. Its slowness is what it has for a link that is bad again and again. At 1% throughout quick steps had 9 and 11 pieces too late where the rule as it is had none.
- **Short memory is down within 10 s and pays as expected**: twice to five times the pieces too late on the two links with loss, and three to four times the pieces played at another speed where the loss comes and goes.

## What this says, and what it does not

- **There is no rule here that comes down sooner for nothing.** Coming down sooner after one bad spell is being down when the next one comes. Quick steps halves the 27 s and costs up to 30 pieces more of 3,000 on a link that is bad every 15 s (in one of two runs none more); short memory costs more and gains some 5 s more.
- **Which is wanted depends on what a bad moment is**: one narrow spell in an evening, or loss every few seconds. On the first the 27 s are a viewer held back for nothing; on the second they are why the sound is whole. Nothing here knows which it is on; a rule that told them apart (how many bad spells the last minute had) was not built.
- Both variants are in `@xmoq/media` behind flags, for listening and for more links; neither is offered as the automatic buffer's rule.
- **For the terminal viewer** nothing changes: it reads the rule of `fixtures/playout/`, which is as it was. If one of the variants became the rule, the table would get its rows (a window and a way of coming down are two columns more) and `crates/watch/src/clock.rs` the same two values; it is a small change there, and was not made.
- Not known: how any of the three sounds (the pieces played slower or faster are counted, not heard); the cause of the two runs in which the buffer stood where nothing put it; real networks.

## Listening

The steps of [the record of the default](2026-10-13-the-default-playout-buffer.md), with `http://localhost:8080/watch?via=4444&buffer=auto&autoDown=quarter` for quick steps and `&autoWindow=3000` added for the short memory, on the link that is narrow and then good (the buffer is down within 15 s and within 10 s where it takes 27), and on the link with 1% loss.
