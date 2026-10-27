# What a publisher answers a FETCH range with, from what it published

A publisher that answers a Standalone FETCH for its own track (§10.12.3) from the Objects it published and still keeps. Two implementations do: the Rust client with `--answer-fetch`, which keeps the newest Objects within a limit, and the publisher of a catalog in `@xmoq/msf`, which keeps the newest catalog alone. For the same range both give the same answer, and these rows pin it: the End Location of FETCH_OK and what the fetch stream carries.

**The rows hold for a publisher that publishes Object 0 of one Group after the other, without a gap, and nothing else.** Both publishers do. A publisher with several Objects a Group is not in the table.

Where a Location the publisher published and let go of lies in the range, the stream carries one End of Unknown Range indicator (§11.4.4.2); where it stands and which Location it names are column 10 of the rows, and what is provisional is the item "Where an End of Unknown Range indicator stands in a Descending fetch" under "Other observations about the draft" in [the implementation notes](../../docs/moqt-implementation-notes.md). With one Object a Group that Location is Object 0 of a Group.

## Format

`answers.tsv`, one row per FETCH, ten columns separated by tabs, no header. Every row ends with one line feed, the last one too; there are no empty rows.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The first Group the publisher published |
| 3 | The newest Group it published. The Largest Object is Object 0 of it |
| 4 | The first Group it still keeps. It keeps every Group from there to the newest; the Groups from column 2 to the one before were published and let go of |
| 5 | Start Location of the FETCH, `group,object` |
| 6 | End Location of the FETCH, `group,object`: the last Object plus 1, and the whole Group with Object 0 (§10.12.1) |
| 7 | Group Order of the FETCH: `ascending` or `descending` |
| 8 | `ok`: FETCH_OK. `invalid_range`: REQUEST_ERROR `INVALID_RANGE` (`0x11`) |
| 9 | End Location of FETCH_OK, `group,object`; `-` in a row with `invalid_range` |
| 10 | What the fetch stream carries after its header, in order, separated by spaces: a number for Object 0 of that Group, `u<group>,<object>` for an End of Unknown Range indicator with that Location. `-` for nothing, and in a row with `invalid_range` |

The rows whose name begins with `catalog-` have the newest Group alone kept (column 4 equals column 3), which is all the publisher of a catalog can be.

## What is not in the table

- A publisher that has published nothing: `INVALID_RANGE`. The row has no first Group to name.
- Which limit made a publisher let go of a Group.
- A FETCH the publisher refuses for another reason: a Joining FETCH, another track, more requests at once than it answers.
- The payloads, Publisher Priorities and Subgroup IDs of the Objects.

## Source

**The table is a decision, not an observation.** §10.12.3 has `INVALID_RANGE` and the End Location; where an indicator for a range a publisher no longer keeps stands in a Descending fetch, and which Location it names, the draft does not say, and no other implementation was asked.

The answers of columns 8 to 10 are what the Rust client gave when the rows were written; the expectations of the `catalog-` rows were written for `@xmoq/msf` first, from the rule, and came out the same in the Rust client. The Rust client, the gatherer of the chat and the transcriber plan their answers with one calculation, `plan_kept` of `crates/publisher/src/fetch.rs`, so the table does not check them against each other: the one implementation of its own that reads it is `@xmoq/msf`, which reads the rows a publisher of a catalog can be, those that keep the newest Group alone.
