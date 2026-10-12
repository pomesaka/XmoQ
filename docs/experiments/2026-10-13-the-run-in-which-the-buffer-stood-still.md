# The run in which the buffer stood still while the sound was too late

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2; before the open decision on the default of the playout buffer. One run of [the record of the automatic buffer](2026-10-13-the-automatic-buffer-what-raises-it-and-how-fast-it-comes-down.md) had the buffer at 370 ms for 65 s while 859 of 1,000 pieces of sound were too late, which no rule gives.

## What is left of that run

One line of numbers, and no log of the page:

- the buffer was 370 ms **at second 10, before the link was made narrow**, and 370 ms at every reading after it, to second 75;
- 859 pieces too late in the narrow 20 s, and **16 in the 45 s after**, with 16 breaks;
- 104 pieces played slower or faster in the whole run, where the other runs have 700 to 800;
- the watching page was opened with `buffer=auto&autoWindow=3000&autoDown=quarter`, and the script pressed "sound" 1.5 s after it asked for the page.

Read before anything was run:

- **A buffer that is the same number at 66 readings is not an automatic buffer that failed to move; it is one that is not automatic.** The rule moves it at every tick unless what it wants is exactly where it stands. The page has one way to turn the automatic buffer off: its slider, whose `input` sets the buffer to the slider's value and ends `auto`. The slider goes from 0 to 500 in steps of 10; 370 is a place on it.
- **With a fixed buffer of 370 ms the sound of this link is not too late** (it comes 65 to 105 ms after the fastest), so 859 pieces too late are not the buffer's doing either. Pieces count as too late when the sound is not being played.

## Expected, written before the runs

**The first suspect is the script and the page's controls, not the clock**: the press on "sound" landed on the slider, which the page puts next to that button after it has heard from the room. The sound was then never turned on, and the buffer was set by hand. If so:

- a run that goes wrong has `auto: false` in the page's report and the sound's state not "running", from the first reading on;
- it happens only at the press, so it shows in the first seconds or not at all, on any link and with any rule for the buffer;
- how often: it was 1 run of about 30 that pressed the button the same way that day, so 20 runs may well show none. 40 are run, short ones, since the first 10 seconds are enough.

Second, if the report says `auto: true`: the fastest way the clock goes by was set by an Object that came with a time ahead of the others (everything after it then counts as late by that much). Third: the page's timers, slowed as in a background tab.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag; `spikes/lossy-link/proxy.mjs` between relay and viewer, 50 ms each way and no limit; the busy picture at 2.5 Mbit/s. No code was changed for the runs: what the page's report already has (`auto`, the buffer, the state of the sound, the Objects that were not of the clock, what 99 of 100 came within) was read.

## Result

A run counts as odd when, 8 to 9 s after the press on "sound", the report says that the buffer is not automatic, or the sound is not running, or the buffer is 200 ms or more.

| How the viewer began | Runs | Odd |
|---|---|---|
| One publisher that goes on; a viewer opened, "sound" pressed 1.5 s after the page was asked for; the rule as it is and the short memory in turn | 40 | 0 |
| As the run that went wrong: a new publisher 2 s before each viewer, the short memory | 30 | 0 |

In every one of the 70 the buffer was automatic, the sound running, and the buffer between 30 and 100 ms. **It did not happen again.** With the 4 runs of the record before, the one that went wrong among them, that is 1 run in 74 that began the same way.

**The first suspect does not hold as it was written**:

- The slider cannot have taken the press as the page is laid out. Read from the page in every run: the button is 52 pixels wide at the left edge, and the slider begins 400 pixels to its right on the same line, before the press and after it. A press on the middle of the button is 370 pixels from the slider.
- A fixed buffer of 370 ms would not have made the sound too late either: with a fixed 300 ms no piece was too late on this link ([record](2026-10-13-the-sound-behind-the-video-on-a-narrow-link-and-priorities.md)), and a piece counts as too late by its own time and the buffer, whether the sound is on or not (`createPlayout`: "too-late", "unheard-too-late").

## What can be said, and what cannot

- **It began before the link was touched.** The buffer was at 370 ms at second 10. Whatever it was, the narrow link did not cause it; it made it show.
- **The run looks like one with a buffer that is not automatic**: the same number at 66 readings, and 104 pieces played at another speed where an automatic buffer has 700 to 800 in such a run and a fixed one few.
- **And the sound did come more than 370 ms late in the narrow 20 s of that run**, or was judged against a time that was off by that much: 859 pieces of 1,000, where every other run of that link has the sound 65 to 105 ms after the fastest. After the link was good again it was in time (16 too late in 45 s).
- Those two together are what a clock that is off would give: a fastest way that is too fast by some hundreds of milliseconds makes everything after it count as late by that much, and a buffer that goes by it stands at its limit or near it. Why the buffer would then stand at 370 and not at 400, and what would set the fastest way wrong in one run of 74, is not known. **Nothing that was run supports it; it is the reading that contradicts least.**
- **Not known**: the cause. Whether it is of the automatic buffer alone: the runs with a fixed buffer on the narrow link that day, more than ten, showed nothing like it, which is too few to say of something that came once in 74. Whether it is the page's or the machine's: the earlier run in which the buffer went to 230 ms on a link nobody touched was not found again either, in 6 minutes of looking.

## Next action

Closed without a cause. What would catch it the next time, at no cost: the measuring scripts read `auto`, the state of the sound and the count of Objects that were not of the clock at the end of every run, and a run in which the buffer is the same at every reading is kept with the page's whole report, not with its totals alone. The scripts of this record did that; the ones of the records before did not.
