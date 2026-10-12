# A publisher and a viewer whose clocks run at different speeds

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2; a long broadcast between two machines, which nothing so far was. Every run before this one had publisher and viewer on one machine and one clock.

## Objective

The publisher gives every Object a time of its own clock (LOC's Timestamp), and the viewer measures by its own when the Object came. Two machines' clocks run at speeds that differ by 10 to 100 parts in a million as a rule: 100 ppm is 360 ms in an hour. What the players do with that.

## The rule, read in the code before anything was run

- **The page** (`createPlayoutClock` and `createFastestWay` of `@xmoq/media`): for every Object, "how long it took" is when it came less the time it carries. **The least of those ever seen is kept, for as long as the page plays**, and taken for the way an Object needs at least; an Object is due at its time plus that least plus the buffer. Nothing forgets the least. The 10 seconds of the automatic buffer are how far back it looks at lateness, not at the least.
- An Object whose "took" is more than 10 s from the least is "not of the clock" and left out; after 30 such in a row the clock begins anew with the next.
- **The terminal viewer** (`crates/watch/src/clock.rs`): the same. The least is kept, 10 s (`MOST_FROM_THE_LEAST_US`), anew after as many in a row; `fixtures/playout/` has rows for the buffer and none for a least that moves.
- The demo's publisher times its video by the page's monotonic clock from when it began, and its sound by counting pieces of 20 ms; the watching page measures arrival by `Date.now()`. On two machines those are four clocks.

## Expected, written before the runs

"Took" changes by the difference of the two speeds, all the time.

**The viewer's clock is the faster one** (the publisher's times fall behind): every Object takes a little longer than the one before, the least stays what it was at the beginning, and **everything comes later and later by the least**, at the rate of the difference.

- A fixed buffer of 50 ms: once the difference has grown past the buffer, every frame comes after its turn and **every piece of sound is too late and left out. The sound stops, and stays stopped.** At 100 ppm that is after about 8 minutes (50 ms at 0.1 ms a second); with 150 ms after 25 minutes.
- The automatic buffer follows up to its limit: at 400 ms after about 67 minutes at 100 ppm, and then the same.
- When the difference has grown to 10 s (28 hours at 100 ppm) the Objects are not of the clock, and after 30 the clock begins anew: it plays again, and the same begins again.

**The publisher's clock is the faster one**: every Object takes a little less than the one before and is the new least. Nothing comes late. The sound's pieces are due a little closer together than they are long, so the player has to play them faster, all the time, by the difference.

**In the runs** the difference is made large so that minutes show what hours would: the publishing page multiplies the times it gives by 1 + N/1,000,000 (`/publish?clockSkewPpm=N`, a flag for this). With -10,000 (1%, 10 ms a second) the sound of a viewer with 50 ms stops after about 5 s, with 150 ms after about 15 s, and with the automatic buffer after about 40 s, at 400 ms; with -1,000 ten times later; with -100,000 (10%) the clock begins anew every 100 s or so. With +10,000 nothing is too late and pieces are played faster throughout. ±100 ppm show nothing in three minutes (18 ms).

## The first runs measured something else, and that is a finding of its own

`/publish?clockSkewPpm=N` multiplies the times the page gives its frames and its pieces of sound. **The sound's times did not move with it**: with -10,000 the frames came later and later (all of them after their turn from the tenth second on) and every piece of sound was in time for 100 s; with +10,000 it was the other way round, the frames in time and the sound lost. What fits is that the browser's Opus encoder gives its chunks times of its own, counted from the first one by the samples it has taken, and does not go by the time each `AudioData` carries (Chromium 156; not read in its source, and the decoder was found to do the same on 2026-10-11). So the flag made **the video's clock run at another speed than the sound's**, on one viewer's clock.

That is not what was to be measured, and it is what a real publisher has: a camera's frames are timed by the machine's clock and a microphone's sound by the sound card's count of samples, and the two differ by parts in a million.

| The video's times against the sound's | Buffer | What happened, read every 10 or 20 s |
|---|---|---|
| 1% slower (-10,000) | 50 ms, 150 ms | The sound whole throughout (500 of 500 pieces every 10 s). Every frame after its turn from second 10 and second 20 on, and so drawn the moment it comes |
| 1% slower | automatic | The buffer follows, 160, 260, 360 ms, with 100 pieces in 500 played at another speed, to its limit of 400 ms at second 40; from then on every frame after its turn |
| 1% faster (+10,000) | 50 ms | **The sound is gone within seconds**: 11 of 1,000 pieces sounded in the first 20 s, none after |
| 1% faster | 150 ms | 501 of 1,000 in the first 20 s, none after |
| 1% faster | automatic | Whole for 20 s while the buffer grows; at its limit of 400 ms the sound goes: 760 of 1,000 in the next 20 s, none after |
| 10% slower (-100,000) | 50 ms, automatic | As at 1% for 90 s. **Then the frames are 10 s from the least and are left out as not of the clock: no frame is shown any more**, to the end of the run 150 s later (606 left out every 20 s). The sound goes on, so the 30 in a row that would begin the clock anew never come |

- **The one least is shared by picture and sound.** Whichever track's times run faster sets it, again and again; the other track is then late by the difference, which grows without end. Where the other track is the sound, its pieces are too late and it stops. Where it is the picture, the frames are drawn as they come, and after 10 s of difference not at all.
- At the 10 to 100 ppm of real devices that is a buffer of 50 ms used up in 8 to 80 minutes, and the 10 s in 1 to 11 days. **This was not seen with a camera and a microphone; the demo's own picture and tone are timed by one clock.** It is what the rule gives, shown with a flag.
- **Beginning anew does not come where two tracks disagree**: one of them is always of the clock, so there are never 30 in a row that are not.

## The viewer's clock against the publisher's: expected again, before the runs

For what was to be measured the watching page gets the flag: `/watch?clockSkewPpm=N` has the page's `Date.now()` run N parts in a million faster (slower with a negative N). Both tracks then move together against the viewer's clock; the sound's own output, which goes by the sound card, is not touched, as on a real machine.

Read in `createPlayout` and `placePiece` before the runs:

- **The viewer's clock faster**: both tracks later and later against the least. The frames after their turn once the difference is past the buffer. A piece of sound follows the one before it while its own time is within 0.3 s of where the sound has got to, played up to 5% faster to catch up; **when the difference is past that, or the sound's queue has run dry once, a piece is placed by its own time, which is past: too late. From then on every piece, and the sound stays off.** With +10,000 that is expected between 5 and 40 s in, by the buffer. With the automatic buffer after it has reached 400 ms.
- **The viewer's clock slower**: every Object is a new least. Nothing late; nothing of the sound changed.

## Environment

macOS 14.4, arm64, one machine, no link in between; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag; the demo's clock and tone, 30 frames and 50 pieces of sound a second. One publishing page and three watching pages at once, with a buffer of 50 ms, of 150 ms, and the automatic one. The difference of the clocks is made by the flags above; nothing was run on two machines.

## Result: the viewer's clock against the publisher's

| The viewer's clock | Buffer | The sound stopped | The frames |
|---|---|---|---|
| 1% faster (+10,000) | 50 ms | within the first 10 s (113 of 500 pieces sounded in them, none after) | all after their turn from second 10 on |
| 1% faster | 150 ms | between second 10 and 20 (114 of 500 in that time, none after) | all after their turn from second 20 on |
| 1% faster | automatic | between second 30 and 40, once the buffer was at its limit of 400 ms (365 of 500, none after) | all after their turn from second 40 on |
| 0.1% faster (+1,000) | 50 ms | between second 40 and 60 (47 of 1,000 pieces in that time, none after) | all after their turn from then on |
| 0.1% faster | 150 ms | between second 140 and 160 | all after their turn from then on |
| 0.1% faster | automatic | not within the 200 s: the buffer grew 1 ms a second, 230 ms at the end | in time |
| 1% slower (-10,000) | all three | never: every piece sounded, none at another speed | all in time; the automatic buffer at 30 ms |

- **As expected, and it does not come back.** Where the viewer's clock is the faster one, the sound stops when the difference has grown past the buffer (and the 0.3 s the player makes up by speed), and from then on every piece is too late: 500 of 500 every 10 s, to the end of each run. The picture goes on, every frame drawn as it comes.
- **It goes by the difference and nothing else**: ten times slower at a tenth of the difference. At 100 ppm, which was not run (three minutes of it are 18 ms), that is **the sound gone after 7 to 10 minutes with the default buffer of 50 ms**, about 25 minutes with 150 ms, and a little over an hour with the automatic buffer, which is by then at 400 ms and has put the whole broadcast that far behind on the way.
- Where the viewer's clock is the slower one nothing happens, since every Object is a new least.
- **Which of the two a pair of machines is, is chance**; half of all viewers of a long broadcast are on the side that loses the sound, if their clocks differ at all. Whether they do depends on what the times are taken from: two clocks that the network keeps right (`Date.now()` on both) differ by what that correction leaves; the demo's publisher counts its frames by a clock that is not corrected, and its sound by pieces.

## What this says, and what it does not

- **The least way is kept for good, and it should not be.** It is right as a measure of the path for a while and wrong as soon as anything moves it for good: two clocks at different speeds, and also a path that got longer by a route, which was not run here.
- **Two things are wrong, and they need different mending**:
  - *The viewer's clock against the publisher's.* The least has to be able to rise again. The plain way is the least **of the last while** (a minute, say) in place of the least ever: at 100 ppm a minute is 6 ms, far inside any buffer, and the least follows the difference in both directions. It is a change to the shared rule (`fixtures/playout/` gets rows for a least that is forgotten, `createFastestWay` and `crates/watch/src/clock.rs` the same window). What it costs is in what a long bad spell does: when a link is bad for longer than the window, the least rises to the bad delay, and when the link is good again everything comes "sooner than the least" at once, which the players take for a new least, and the sound has that much to make up by speed or with a break. The automatic buffer is untouched by it but measures against the same least, so it would come down sooner after such a spell than it does now.
  - *The picture's clock against the sound's, within one broadcast.* No rule of the viewer mends that with one least for both: the two tracks' times do move apart. A least for each track would keep both playing, and would let picture and sound be apart by what the publisher's two clocks differ. The mending that keeps them together is on the publisher's side: the sound's times taken from the clock the frames are timed by, and not from the encoder's count of samples. Whether a real camera and microphone in a browser drift so, and by how much, is the first thing to measure; it was not.
- **The run in which the buffer stood at 370 ms** ([record](2026-10-13-the-run-in-which-the-buffer-stood-still.md)) is not a difference of clocks: it was one machine and 75 seconds. **Its numbers are what a least that is too low by about a third of a second gives, though**: an automatic buffer that stands at one value (the offset and 10 ms more, the same at every reading), few pieces at another speed, the sound too late as soon as the link adds 65 to 105 ms and the buffer's limit of 400 ms is passed, and in time again when the link is good. The runs here show that picture: with +1,000 the automatic buffer is the offset plus 20 ms at every reading. What would have put the least a third of a second too low in that run is not known; an Object that came with a time that far ahead, in the first seconds, would do it, and nothing would undo it. A least that is forgotten after a while would have.
- Not known: any of it on two machines; how far real clocks differ where the times come from the browser's clocks; whether the 0.3 s that the player makes up by speed are heard as it does.

## Next action

A report, and no change to the rule here: the mending touches `@xmoq/media`'s core, the shared table and the terminal viewer. The flags stay for measuring it: `/watch?clockSkewPpm=N` (the viewer's clock) and `/publish?clockSkewPpm=N` (the video's times against the sound's).
