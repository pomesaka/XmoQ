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
