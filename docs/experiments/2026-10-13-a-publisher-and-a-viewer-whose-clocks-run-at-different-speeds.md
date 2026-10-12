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

## Afterwards: the least way is of the last minute

Decided on the report above: the rule itself is changed, in both players. **The least is the least of the Objects that came in this second of the viewer's clock and the 59 before it**, and no longer the least ever seen. What follows was written before the change was run.

**The 60 seconds, provisional, from both sides**:

- *How fast clocks move apart.* The viewer is behind the least by what the clocks have moved in the window: 6 ms at 100 ppm, which any buffer holds. A shorter window would hold more; this one already holds what real clocks do.
- *How long a bad spell is believed.* A link that is bad for longer than the window has its delay taken for the way itself. The longer the window, the longer a spell has to last for that. A minute is longer than the narrow spells and the stretches of loss that were run (20 s and less), and short enough that a path that really got longer is followed within the minute.

**What the 10 seconds and the 30 in a row are for now**: unchanged, and for what they were for, a time that is not of the clock at all. An Object more than 10 s from the least of the window is left out, and 30 such in a row begin the clock anew: a publisher whose clock jumped is followed after 30 Objects, not after a minute. New: where nothing of the clock came for a minute the window is empty, and the next Object is the clock's first, whatever its time.

**Expected**:

| Run | Before | With the change |
|---|---|---|
| The viewer's clock 1% faster | The sound stops for good after 10, 20, 40 s | **Not mended, and not meant to be**: the viewer is 600 ms behind the least of the minute, more than any of the three buffers. The sound stops as before |
| 0.1% faster (1,000 ppm) | Stops after 40 to 60 s with 50 ms, 140 to 160 s with 150 ms | Behind by 60 ms, and no more, from the first minute on. 150 ms and the automatic buffer (which stands at 70 to 80 ms): whole, for as long as it runs. 50 ms: 60 ms is more than the buffer, so pieces go on being lost |
| 300 ppm faster | Would stop after about 3 minutes with 50 ms | Behind by 18 ms: whole with every buffer, to the end of a run of 5 minutes |
| 100 ppm faster, 13 minutes | Would stop after 7 to 10 minutes with 50 ms | Behind by 6 ms: whole |
| 1% slower | Nothing | Nothing |
| A link that is narrow for 90 s and good again, the clocks alike | The least stays; the automatic buffer at 400 ms, down 27 s after | After a minute of the narrow spell the least rises by what the sound has been late (65 to 105 ms), and the lateness the buffer sees falls by as much; nothing is lost by that, the automatic buffer may come down a step or two while the link is still bad. **When the link is good, everything comes sooner than the least at once**: the sound has 65 to 105 ms to make up, played faster for about 2 seconds (100 pieces), with no break and none too late |
| The terminal viewer, the same clocks | (not run before) | As the page, by the same table |

And in the core's tests: a least that was put a third of a second too low by one Object in the first seconds is back where it belongs a minute later.

### Result with the change

The page, the same runs as above (`/watch?clockSkewPpm`, one machine):

| The viewer's clock | Buffer | Before | With the least of the last minute |
|---|---|---|---|
| 1% faster, 80 s | 50, 150, automatic | The sound stops for good within 10, 20, 40 s | **The same**: stops within 20 s (50 ms), within 40 s (150 ms), within 60 s (automatic, at 400 ms). The lateness stands at 604 to 608 ms from the first minute on, and no longer grows |
| 0.1% faster, 220 s | 50 ms | Stops between 40 and 60 s | Stops between 60 and 80 s, and for good: the lateness stands at 72 to 73 ms, over the buffer |
| 0.1% faster | 150 ms | Stops between 140 and 160 s | **Whole to the end**: 1,000 of 1,000 pieces in each 20 s, none too late, none at another speed after the first minute |
| 0.1% faster | automatic | Rises 1 ms a second, 230 ms after 200 s | **Stands at 90 ms from 60 s on**, whole |
| 300 ppm faster, 300 s | all three | (not run; 50 ms would stop after about 3 minutes) | **Whole**: no piece too late in 5 minutes, the lateness at 31 ms throughout, the automatic buffer at 50 ms |
| 1% slower, 60 s | all three | Nothing | Nothing |

The terminal viewer (`crates/watch/scripts/try.sh 200 --buffer-ms … --clock-skew-ppm 1000`, three at once; "before" is the same build with the window set to 600,000 s for the run):

| Buffer | Before: sound played of 200 s, packets left out as late | With the change |
|---|---|---|
| 50 ms | 58 s, 6,793 of 9,993 | 135 s, 1,066 (and 3,167 cut at their beginning): 60 ms is over this buffer too |
| 150 ms | 162 s, 1,636 | **199.7 s, 0** |
| automatic | 199.8 s, 0; the buffer up to 220 ms | 199.8 s, 0; **the buffer up to 90 ms** |

A link that is narrow (2 Mbit/s under a picture of 2.5) for 90 s and good again, the clocks alike, 150 ms and the automatic buffer; one run of each rule:

| | The least ever | The least of the last minute |
|---|---|---|
| While narrow: lateness of 99 in 100 | 790 to 910 ms | 770 to 950 ms, **and no lower after the first minute of it** |
| While narrow, 150 ms: pieces too late | 16 to 53 in every 5 s | 0 to 60 in every 5 s |
| While narrow, automatic | 400 ms throughout, no piece too late after the first 5 s | the same |
| From 60 s into the narrow spell: pieces at another speed | 0 | 17 (150 ms) and 21 (automatic) in the 5 s after the minute, a few after |
| The 5 s in which the link is good again | no piece too late, none at another speed | no piece too late, no break but 2 in the first 5 s (150 ms); **24 and 18 pieces at another speed** |
| Automatic buffer back at 30 ms | 30 s after | 30 s after |

- **What real clocks do is held, and what was held is as counted**: the viewer stays behind by what the clocks move in a minute (18 ms at 300 ppm, 60 ms at 1,000, 600 ms at 10,000), and the sound is whole where the buffer is more than that. 100 ppm was not run; 300 ppm, three times that, lost nothing in 5 minutes with the default buffer.
- **Wrong in the expectation**: with 50 ms and 1,000 ppm the page's sound does not "go on losing pieces", it stops; the terminal viewer plays two thirds. And **the narrow link did not raise the least by the 65 to 105 ms expected**: enough Objects still come by the fast way in every minute of it. What the change did there is some twenty pieces played at another speed a minute into the spell and again when it ends, which says the least moved a little, by how much was not read. A link whose every Object is late for more than a minute (a longer route, not a full queue) was not run, and is where the cost would show.
- No harm was seen from the least rising while the link is bad: the automatic buffer was at its limit throughout under both rules.
- One run each; the numbers of the narrow spell differ from run to run by more than the two columns do.

## For a person with a camera and a microphone: do the two tracks' clocks move apart?

Not run. What it asks is in the watching page already, with no flag:

1. `mise run demo`, then <http://localhost:8080/publish?source=camera&audio=mic> in one tab and <http://localhost:8080/watch> in another, both left in front (two windows side by side), for 30 minutes.
2. Under the picture, the lines beginning `video:` and `audio:` each say `came median N ms after it was made`: when it came, by this machine's clock, less the time the publisher gave it, over the last 200 of the track. Either number alone means little (a camera's times count from where the browser likes). **Write down the video's less the audio's after 1 minute, and again every 10 minutes.**
3. If that difference stays within a few milliseconds, the two are timed by one clock and there is nothing to mend. If it moves one way steadily, they are not: 6 ms a minute are 100 ppm. The track whose number grows against the other is the one with the slower clock. Once the difference has moved by more than the buffer, that track shows it: `N too late` in the `audio:` line counts up and the sound stops, or (the video the slower one) the picture is still in time but drawn the moment it comes.

Both pages are on one machine here, so the viewer's clock against the publisher's plays no part; what is read is the camera's clock against the microphone's alone. A reading once a second, for a script, would need the two medians in the page's report (`document.body.dataset.report`), where they are not.
