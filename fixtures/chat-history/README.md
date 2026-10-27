# What a participant of the chat fetches of the lines before its subscription

A participant of a room subscribes to the track `lines` of `xmoq/chat/<room>` (`crates/chat-tracks/README.md`), and asks for lines said before by an Absolute Joining FETCH (§10.12.2.1), when it comes and each time its subscription is made again. Two implementations decide what to ask for by the same rule: `followPlan` of `@xmoq/tracks` (`packages/tracks/src/core/follow-plan.ts`, which `@xmoq/chat` and `@xmoq/transcript` follow their tracks by) and `past_to_fetch` of the terminal viewer and the reader for agents (`crates/subscriber/src/followed.rs`, with the chat's values of `crates/viewer/src/chat_track.rs`), where its reasons are. These rows pin it.

The rule, decided on 2026-10-24, with `catchUp` the most lines a participant catches up (its own default, 1 to 1,000):

- Nothing had: the newest 16 lines.
- Lines had up to `had`: from the line after it to the Group of the LARGEST_OBJECT; where they are more than `catchUp`, the newest `catchUp` and a gap before them. Nothing where `had` is that Group already.
- That Group below `had`, or no LARGEST_OBJECT where lines were had: the room was numbered anew; what was had is forgotten, and the newest 16 are fetched.

## Format

`history-to-fetch.tsv`, one row per case, eight columns separated by tabs, no header. Every row ends with one line feed. `-` for none.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The newest line had, by its number (its Group) |
| 3 | The Group of the LARGEST_OBJECT of the SUBSCRIBE_OK |
| 4 | `catchUp` |
| 5 | Whether what was had is forgotten: `yes` or `no` |
| 6 | The first Group of the FETCH; `-` where nothing is fetched |
| 7 | The gap before the lines fetched: the line after which lines are not fetched |
| 8 | The gap: the first line fetched after it |
