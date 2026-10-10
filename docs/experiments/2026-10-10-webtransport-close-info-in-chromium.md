# The code and reason of a session close from `@xmoq/moqt` in Chromium

- Date: 2026-10-10
- Commit SHA: the commit that adds `packages/moqt/src/webtransport/close-info.ts` together with this record. The runs were made on its working copy
- Roadmap item: Phase 0b, own TypeScript browser client. Follows a review of [the stream error record](2026-10-10-webtransport-error-in-chromium.md): `MoqtConnection.close(code, reason)` handed both to the browser unchecked

## Objective

Find out what Chromium sends for a `closeCode` outside 32 bits and for a `reason` longer than 1024 bytes, and make `@xmoq/moqt` report in `closed` what the peer was told.

### What the specifications ask for

| # | Text | Where |
|---|---|---|
| 1 | `dictionary WebTransportCloseInfo { unsigned long closeCode = 0; USVString reason = ""; };` The code has no `[EnforceRange]` and no `[Clamp]`, so a value outside its range is converted by WebIDL, not refused | W3C WebTransport, Candidate Recommendation Snapshot of 30 July 2026, §6.10 |
| 2 | `close()`: "Let reasonString be the maximal code unit prefix of closeInfo.reason where the length of the UTF-8 encoded prefix doesn't exceed 1024." | the same, §6.4 |
| 3 | The Application Error Message of WT_CLOSE_SESSION "MUST NOT exceed 1024 bytes", and a sender that truncates "MUST do so at a UTF-8 character boundary" | `draft-ietf-webtrans-http3-16` §6, as quoted at `MAX_CLOSE_REASON_LEN` in `crates/moqt-io/src/lib.rs`; the draft was not fetched again for this record |
| 4 | A session is terminated with a relevant code; the codes are small numbers | `draft-ietf-moq-transport-18` §3.5 |

The Rust implementation cuts the reason to at most 1024 bytes at a character boundary (`close_reason` in `crates/moqt-io/src/lib.rs`, read on 2026-10-10) and sends a `u32` code.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Browser | Chromium 156.0.8078.4, new headless, launched by `playwright-core` 1.64.0 with `channel: "chromium"` |
| Relay | `xmoq-relay` 0.0.0 on `web-transport-quinn` 0.13.2, debug build. It prints the code and reason a peer closed with |

## Commands

From the repository root, after `cargo build --locked -p xmoq-relay` and one interop run for the certificate:

```sh
bun spikes/wt-close-info/run.ts
bun spikes/wt-close-info/run.ts --write-fixtures
mise run check
```

`run.ts` opens a session from a page with the browser's `WebTransport`, closes it with a given `closeCode` and `reason`, and prints what `closed` resolved with in the page and what the relay reported. At the end it does the same once through `@xmoq/moqt`.

## Observed results

### What Chromium sends

| `closeCode` given | `closed` in the page | The relay got |
|---|---|---|
| 0, 255, 256, 4294967295 | the same | the same |
| 4294967296 | 0 | 0 |
| -1 | 4294967295 | 4294967295 |
| 1.5 | 1 | 1 |
| NaN | 0 | 0 |

The code is sent in 32 bits, and a value outside is wrapped, not refused. Chromium treats the two codes a page can give differently:

| | Code of a stream error (`WebTransportError`, [record](2026-10-10-webtransport-error-in-chromium.md)) | Code of a session close (`WebTransportCloseInfo`) |
|---|---|---|
| Specification | `[Clamp] unsigned long?`: 32 bits, clamped | `unsigned long`: 32 bits, converted by WebIDL |
| Chromium 156 keeps | 8 bits | 32 bits |
| 256 | 255 | 256 |
| 2^32 | 255 | 0 |
| -1 | 0 | 2^32 - 1 |
| 1.5 | 1 | 1 |
| NaN | not measured | 0 |
| `@xmoq/moqt` accepts | whole numbers from 0 to 255 | whole numbers from 0 to 2^32 - 1 |

| `reason` given | Bytes of UTF-8 | `closed` in the page | The relay got |
|---|---|---|---|
| 1024 times "a" | 1024 | 1024 code units | 1024 bytes |
| 1025 times "a" | 1025 | 1025 code units | 1024 bytes |
| 1023 times "a", "é" | 1025 | all of it | 1023 bytes |
| 1022 times "a", "あ" | 1025 | all of it | 1022 bytes |
| 1021 times "a", "😀" | 1025 | all of it | 1021 bytes |
| 1023 times "a", "😀" | 1027 | all of it | 1023 bytes |
| 1020 times "a", "😀" | 1024 | all of it | 1024 bytes, ending in "😀" |
| U+FEFF, "abc" | 6 | all of it | 6 bytes: the U+FEFF is sent |
| "a", U+D83D (half of a surrogate pair) | not valid text | all of it | "a", U+FFFD in the text the relay had (see the limitations for where it was replaced) |

Chromium cuts the reason between two characters to at most 1024 bytes, as rows 2 and 3 ask. **`closed` resolves with the reason as it was given, not as it was sent.**

### What `@xmoq/moqt` does now

- `MoqtConnection.close(code, reason)` throws a `RangeError` for a code that is not a whole number from 0 to 2^32 - 1, before it touches the session. The session stays open and can be closed with a valid code.
- The reason is cut to at most 1024 bytes of UTF-8 between two characters before it is given to the browser, and `closed` reports the cut reason. The same is done where the session is closed for an error, with the error's message as the reason, and where the session core asks for the close.
- Through the library in Chromium: `close(5, 1023 times "a" + "é")` resolved `closed` with code 5 and a reason of 1023 bytes, and the relay printed code 5 and the same 1023 bytes.

**The bytes Chromium sent are now a shared fixture**, `fixtures/webtransport-close/reasons.tsv`: 14 reasons and what the relay received for each, written by `bun spikes/wt-close-info/run.ts --write-fixtures`. The TypeScript tests read it for the function that cuts the reason and for the transport the tests play. The Rust tests do not read it yet. Before the fixture, the Rust function and this one had been compared by reading only, and a review that ran them found a difference: the first version here dropped a leading U+FEFF, which Rust and Chromium keep. With that put back, the two rows with a U+FEFF fail.

The function that reads the fixture refuses a row that is not four columns, whose digits are not whole bytes in lower-case hexadecimal, or whose UTF-16 column has half a code unit, and refuses a file without rows; with the file emptied, the tests fail.

The transport the package's tests play now treats a close as Chromium does: it wraps the code to 32 bits and hands the peer the cut reason while `closed` gets the whole one. With the check and the cut taken out of `close()`, 5 of the tests fail.

- A session the library closes for an error: where the session core asks for the close, the error in `closed` carries the reason as it was sent. Where another error ends the session, a hook that throws for one, `closed` has that error as it is, and the peer gets its message cut.
- -0 is accepted and sent and reported as 0.

`mise run check` passed.

## Limitations and blockers

- **Chromium only.**
- **The Rust tests do not read the fixture yet**, so that the Rust function gives these bytes is still known from reading it.
- **The fixture's bytes passed through the relay's text of the reason.** The relay has the reason as text, not as bytes. For the 12 rows whose reason is valid text that is what was on the wire. For the 2 rows with half of a surrogate pair, the capture cannot tell whether Chromium sent the U+FFFD or something on the way replaced other bytes with it. The specifications make the browser replace it (`USVString`, Web IDL §3.2.12 and Infra §4.7), and the capsule decoder of the library under the relay refuses invalid UTF-8 instead of replacing it, but a reason read from the close of the QUIC connection is converted with replacement, and which path the relay took was not recorded. The raw bytes were not captured. The README of the fixture has the details.
- `spikes/wt-close-info/` is outside lint and review.

## Open questions raised

- The codes an application may close with stay any whole number of 32 bits. §3.5 of draft 18 lists the Session Termination Error Codes and says the application "SHOULD use a relevant code, as defined below"; neither it nor the registry (§15.10.1) sets a range aside for applications, as §2.5 does for Property types. So an application's own code can collide with one a later draft defines.
