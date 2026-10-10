# A stream of the peer that does not say what it is, stopped by a page after a time

- Date: 2026-10-11
- Commit SHA: the commit that adds `firstBytesTimeoutMs` to `@xmoq/moqt` together with this record. The runs were made on its working copy
- Roadmap item: groundwork for Phase 1 (limits on what is received). It closes no item

## Objective

`@xmoq/moqt` reads 64 unidirectional and 64 bidirectional streams of the peer at once, and a stream held its place until it ended. One whose peer sent a first byte and stopped held it for the whole session, and 64 of them kept every later stream from being read. The Rust implementation stops such a stream after 10 seconds. Give the TypeScript side the same time, at the same points and with the same codes, see it in a real browser, and find out how late a browser can run the timer.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless, the page in the foreground |
| Relay | the own relay, **with a temporary change that is not committed**, the one of [the record on the streams a page takes](2026-10-11-streams-a-page-takes-from-a-peer.md): with `XMOQ_PROBE_STREAMS` set it opens up to 400 streams towards every session, writes to each the first bytes of a FETCH (bidirectional) or the type of a Subgroup Header (unidirectional), and keeps them open. Taken back, the diff of `crates/` empty |
| Page | the own page of `interop/`, **with `firstBytesTimeoutMs: 2000` for these runs** (a temporary change, taken back), so that a run of some seconds sees the time run out twice |

## Commands

```sh
cd interop
XMOQ_PROBE_STREAMS=bidi bun disconnect.ts --scenario publisher-killed --publisher own-rust --subscriber own-browser --verbose
XMOQ_PROBE_STREAMS=uni  bun disconnect.ts --scenario publisher-killed --publisher own-rust --subscriber own-browser --verbose
```

Counted in the log: the relay's `PROBE_OPENED` lines for the page's session, and the page's `peer-stream-timed-out` events.

## Observed results

| | Streams the relay opened towards the page | Stopped by the page for the time |
|---|---|---|
| Bidirectional | 100 | 100: 64, and then the other 36 |
| Unidirectional | 100, beside its control stream | 100: the ones the page read first, and then the rest |

- **Every stream that said nothing was stopped, and the ones that had waited for a place were read then and stopped in their turn.** In the log, the first events alternate with the first bytes of a stream that had waited: each stream that is stopped frees its place for the next. The time of a stream that waits in the browser starts when the page takes it.
- **The scenario passed in both runs**, with its subscription and the PUBLISH_DONE after the kill: the session stays open, and the control stream and the subgroup streams of the subscription are not touched.
- **What was expected and not seen: the relay opening more than 100 streams.** It opened 100 and no more in both runs. A stream the page stops is not closed for QUIC while the relay's side of it is open, and the probe keeps its side open whatever the page does; a peer that follows the draft resets a stream that is stopped (§11.4.3). So the limit of 100 that Chromium gives a peer was not seen to move on. One run each.
- The page's timer was not measured against a clock; the log of a page has no times. That the first 64 ran out together and the rest later is the order of the lines.

The tests with the played transport and a clock that stands still (`packages/moqt/test/webtransport/first-bytes.test.ts`; `jest.useFakeTimers` of bun 1.3.12 drives `setTimeout`): a stream is not stopped at 9,999 ms and is at 10,000; the codes are 0x0 (`INTERNAL_ERROR`) for a unidirectional stream and 0x1 (`CANCELLED`) for both directions of a request, as in the Rust reader; a stream that waits for a place has no time yet; a stream that has said what it is, is not stopped after an hour. Nine temporary changes to the implementation, each taken back, made tests fail: among them no timer, a subgroup stream that counts as having said it with its type alone, a request that counts on its Request ID, a timer that starts when the peer opens the stream.

### How late a browser runs the timer

Read, not measured: Chrome's own description of its timer throttling ([developer.chrome.com, "Heavy throttling of chained JS timers beginning in Chrome 88"](https://developer.chrome.com/blog/timer-throttling-in-chrome-88), fetched on 2026-10-11).

| The page | Timers are checked | So 10 seconds are at most about |
|---|---|---|
| visible, or made sound in the last 30 seconds | as asked | 10 seconds |
| hidden | once a second | 11 seconds |
| hidden for more than 5 minutes, silent for 30 seconds, no WebRTC, **and the timer has a chain count of 5 or more** | once a minute | 70 seconds |

- A timer of this kind is set when a stream is taken. A stream taken because a timer just stopped another one is set from a timer's task, which is what the chain count counts; after five such generations in a page hidden for long, the third row can apply. A stream taken for another reason starts a chain anew.
- **Late is the harmless direction.** A stream that says nothing holds its place for longer; none is stopped early. A page that is frozen reads nothing either, so its peer waits whatever the timer does. Freezing is not in the text that was read.

## Limitations and blockers

No blocker.

- **A stream that has said what it is and then stays open still holds its place**, for as long as its peer likes: a request that was answered and is not ended, a subgroup stream after its header. The Rust implementation has the same. Nothing was changed there.
- The 2 seconds of the runs are not the default; the default of 10 seconds was run against the played transport only.
- A page in the background was not run. The table above is what Chrome's text says.
- One browser, one version, loopback.

## Open questions raised

- What ends a stream that has said what it is and stays open, before `@xmoq/moqt` serves a request of the peer.
