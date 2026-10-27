# Reasons of a WebTransport session close

Reasons an application closes a session with, and the bytes a browser sent for each. XmoQ's implementations cut the reason themselves before they hand it to the transport, so that what an endpoint reports about its own close is what the peer was told; these rows pin that cut to what a browser does. Both implementations read the file in their tests.

## Format

`reasons.tsv`, one row per reason, four columns separated by tabs, no header. Every row ends with one line feed, the last one too; there are no empty rows and no carriage returns. Hexadecimal digits are lower case. Both readers refuse a table that is written otherwise:

| Column | Content |
|---|---|
| 1 | Name of the row |
| 2 | `utf8` or `utf16`: how column 3 encodes the reason |
| 3 | The reason an application gives, in hexadecimal digits. With `utf8`, its UTF-8 bytes. With `utf16`, its UTF-16 code units, two bytes each, high byte first; these rows hold half of a surrogate pair, which is not valid text |
| 4 | The reason as it was sent, in hexadecimal digits of UTF-8. Empty for an empty reason |

An implementation whose strings are always valid text, as Rust's, cannot build the input of a `utf16` row: the Rust test checks the cut on the 12 `utf8` rows, and reads the 2 `utf16` rows only for their form, counting both kinds.

## Source

Column 4 comes from Chromium 156.0.8078.4 and not from an XmoQ function. Captured on 2026-10-10 on macOS 14.4 arm64 over loopback: a page called `WebTransport.close({ closeCode: 0, reason })`, and the own relay (`xmoq-relay` on `web-transport-quinn` 0.13.2) printed the reason it received.

`reasons.tsv` is written by `bun spikes/wt-close-info/run.ts --write-fixtures`. Do not edit it by hand.

### What the capture can and cannot show

The relay does not hand on the bytes of the reason. It has the reason as text and prints it in Rust's debug form; the capture undoes that form and encodes the text as UTF-8. So column 4 is the bytes of the text the relay had, and the two kinds of rows rest on different grounds.

- **The 12 `utf8` rows.** The reason is valid text all the way, so the text the relay had is what was on the wire, byte for byte. These rows are what Chromium sent.
- **The 2 `utf16` rows**, whose reason holds half of a surrogate pair. Column 4 has U+FFFD (`ef bf bd`) in its place. **The capture cannot tell whether Chromium sent U+FFFD, or sent other bytes that were replaced on the way to the relay's text.** What speaks for Chromium:
  - The specification makes the browser replace it. `reason` is a `USVString` (W3C WebTransport §6.10), and a value becomes one by "converting string to a sequence of scalar values" (Web IDL §3.2.12, Living Standard of 6 October 2026), which is "replace any surrogates with U+FFFD" (Infra §4.7, Living Standard of 5 October 2026).
  - The library under the relay has two ways to a reason. Its decoder of the WT_CLOSE_SESSION capsule refuses a reason that is not UTF-8 and does not replace anything (`String::from_utf8` in `web-transport-proto` 0.6.2, `src/capsule.rs`). A reason taken from the close of the QUIC connection is converted with replacement (`String::from_utf8_lossy` in `web-transport-quinn` 0.13.2, `src/error.rs`). Which of the two the relay took for these rows was not recorded.

  The raw bytes were not captured: with the capsule decoded inside the library, the relay never has them, and that was not changed for this fixture.

## What the rows show

| Row | Reason | Sent |
|---|---|---|
| `empty`, `short` | "", "bye" | the same |
| `1023-bytes`, `1024-bytes` | "a" 1023 and 1024 times | the same |
| `1025-bytes` | "a" 1025 times | the first 1024 bytes |
| `2-byte-character-across-the-limit` | "a" 1023 times, "é" (1025 bytes) | the 1023 "a" |
| `3-byte-character-across-the-limit` | "a" 1022 times, "あ" (1025 bytes) | the 1022 "a" |
| `4-byte-character-across-the-limit` | "a" 1021 times, "😀" (1025 bytes) | the 1021 "a" |
| `4-byte-character-one-byte-over` | "a" 1023 times, "😀" (1027 bytes) | the 1023 "a" |
| `4-byte-character-ending-at-the-limit` | "a" 1020 times, "😀" (1024 bytes) | the same |
| `leading-bom`, `bom-only` | U+FEFF "abc", U+FEFF | the same: the U+FEFF is kept |
| `half-of-a-pair-at-the-end` | "a", U+D83D | "a", U+FFFD |
| `only-halves-of-pairs` | U+D83D, U+D83D | U+FFFD, U+FFFD |

The limit is the 1024 bytes of `draft-ietf-webtrans-http3-16` §6, and the cut is between two characters.
