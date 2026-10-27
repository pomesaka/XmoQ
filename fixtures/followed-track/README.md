# The limits and waits of a followed track

A followed track is a kept track of one numbered item a Group, read with what came before (`docs/architecture.md`, "The roles of an endpoint"): the chat's `lines` and a transcript's `speech`. `limits.tsv` has what its readers wait for and hold, in Rust and in TypeScript. All the values are **provisional**, with their reasons at the constants. Each row is read by the tests of the parts that hold its value, so that a value changed on one side alone fails a test; why a row has `-` on one side is at the test that reads it.

## Format

`limits.tsv`, one row per limit, three columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the limit; unique |
| 2 | The Rust value; `-` where it has none |
| 3 | The TypeScript value; `-` where it has none |

Bytes are bytes, `-ms` is milliseconds, and a count is a count.
