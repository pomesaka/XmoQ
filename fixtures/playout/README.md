# When picture and sound are played

What the two players of this repository agree on: `@xmoq/media` (`packages/media/src/core/playout-clock.ts`, `audio-seam.ts`), which the watching page plays by, and `crates/watch` (`src/clock.rs`, `src/playout.rs`), the viewer for the terminal. No draft says any of it. Both read these tables in their tests; a row is changed for both or for neither.

The rows are those of the tests `@xmoq/media` had before the table was shared (`packages/media/test/playout.test.ts`), written out in whole numbers. **Every value in them is provisional**, as it is where it is defined.

## `auto-buffer.tsv`: the buffer that follows how late things come

Tab-separated, a newline after each row.

| Column | |
|---|---|
| 1 | A name, unique in the table |
| 2 | The buffer the clock begins with, in milliseconds |
| 3 | `yes`: the buffer is automatic; `no`: it is not |
| 4 | The most the automatic buffer may become, in milliseconds; `-` for the default, 400 |
| 5 | How many Objects come |
| 6 | How many milliseconds after the fastest way every Object but the first comes |
| 7 | How many ticks follow |
| 8 | The buffer after the ticks, in milliseconds |

The Objects come 10 ms apart on the publisher's clock: Object `n` has the time `n * 10` ms, and comes at `1000 + n * 10` ms of the player's clock, plus column 6 for every `n` but 0. The ticks come 1 ms after the last Object.

The rule the rows pin: the buffer wanted is what 99 of 100 of the Objects of the last 10 seconds came within, after the fastest, and 10 ms more, rounded up to 10 ms and no more than the limit. At a tick an automatic buffer goes to it at once where that is more, and 10 ms towards it where it is less. Fewer than 50 Objects move nothing.

## `least.tsv`: the fastest way is that of the last minute

Tab-separated, a newline after each row.

| Column | |
|---|---|
| 1 | A name, unique in the table |
| 2 | The Objects in the order they come, a space between them: `when:took`, when it came on the player's clock and how long it took (when it came less the time it carries), both in milliseconds |
| 3 | The fastest way after each, in milliseconds; `-` for an Object that is left out |

The rule the rows pin: the fastest way is the smallest `took` among the Objects that came in this second of the player's clock and the 59 before it (seconds are whole: `when / 1000`, rounded down). An Object more than 10 seconds from it, either way, is left out and is not kept; where the 60 seconds hold nothing, the Object is the first. An Object that came before the newest second (a clock that was set back) counts for the newest. The 30 in a row after which the clock begins anew are in each implementation's own tests.

## `seam.tsv`: where a piece of sound begins

Tab-separated, a newline after each row. Every piece is 20 ms long. Times are microseconds of the clock the sound is played by.

| Column | |
|---|---|
| 1 | A name, unique in the table |
| 2 | The piece's own time: when it is due, with the buffer |
| 3 | Where the piece before it ends; `-` for the first |
| 4 | The clock now |
| 5 | `sound`, or `too-late` for a piece that is not sounded |
| 6 | Where it begins |
| 7 | Where it ends: 20 ms after it begins, or more or less where it is played slower or faster |
| 8 | `yes`: it follows the piece before it to the sample |
| 9 | `yes`: it is played slower or faster |

Columns 6 to 9 are `-` for a piece that is not sounded.

The rule the rows pin: a piece follows the one before it, to the sample, as long as that one still sounds and ends within 300 ms of the piece's own time; otherwise it begins at its own time, and is not sounded where that is over. A piece that follows and is more than 8 ms from its own time is played up to a twentieth slower or faster, so that the sound comes back to its own time without a gap.
