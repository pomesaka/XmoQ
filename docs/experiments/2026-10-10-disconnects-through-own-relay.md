# Clients that leave, through the own relay (MOQT draft 18)

- Date: 2026-10-10
- Commit SHA: the commit that adds `interop/disconnect.ts` together with this record. The runs were made on its working copy twice: on top of `ef0d72b`, where the relay and own-rust were on `wtransport` 0.7.2, and again on top of `68fa616`, where they are on `web-transport-quinn` 0.13.2. Results are those of the second unless "before the change of library" is said
- Roadmap item: Phase 0b, "Put the runs in which a publisher or a subscriber disconnects mid-transfer into the interop matrix, so that CI runs them"

## Objective

Run, from the interop matrix and so in CI, what happens when a client leaves while Objects are moving: a publisher that is killed, a publisher that closes right after its last Object, a subscriber that leaves while others stay, and a subscriber that takes nothing. Until now these were a script under `spikes/` that was run by hand, with the own Rust client on both sides.

Names used below: **own-browser** is `interop/page.ts` on `@xmoq/moqt`, **own-rust** is `xmoq-moqt-client`, **moqtail-browser** is `interop/peers/moqtail/browser/page.ts` on the `moqtail` npm package, and **moqtail-rust** is `interop/peers/moqtail/rust-client` on the `moqtail` crate. The relay is always the own relay, `xmoq-relay`.

### What has to hold

Listed before the scenarios were written, from `draft-ietf-moq-transport-18` (§3.3.2, §3.3.3, §5.1, §5.1.1, §8, §9.4, §9.5, §10.11, §11.4.1, §11.4.3), the rows on PUBLISH_DONE, STOP_SENDING, resets and slow subscribers in [the implementation notes](../moqt-implementation-notes.md), and the paragraph on slow subscribers in [the architecture](../architecture.md). "Scenario" names the runs below; "not checked" gives the reason.

| # | What has to hold | Basis | Checked in |
|---|---|---|---|
| 1 | When a publisher goes, with or without PUBLISH_DONE, the relay ends each subscription to its track with PUBLISH_DONE, not by breaking off the request stream | §11.4.1 (SHOULD), §10.11 | publisher killed; last Object then close |
| 2 | The status of that PUBLISH_DONE is `TRACK_ENDED` (`0x2`) | Implementation notes, "What ends the upstream subscription" | the same |
| 3 | PUBLISH_DONE is sent only after every stream of the subscription is closed, and its Stream Count is the number of streams opened, or 2^62 - 1 | §10.11 (MUST NOT, MUST), §5.1.1 | The count: every scenario, where both own subscribers compare it with the streams they got, and the stopped subscriber, which has to get 2^62 - 1. The order is not checked: a subscriber cannot tell it. own-rust reports only that the counted streams had ended before its wait of 2 seconds was over. Covered by the relay's tests |
| 4 | A subgroup stream that got all its Objects ends with a FIN; one that did not ends with a reset | §11.4.3 (MUST) | publisher killed, where own-browser and moqtail-browser leave a stream open; own-rust reports every stream that ended without a FIN |
| 5 | A stream the publisher had finished is delivered whole to the subscribers when the publisher's connection goes right after | Implementation notes, "A publisher whose connection goes" | last Object then close |
| 6 | The relay notices a peer that vanished without closing, within its idle timeout and one keep-alive interval (6 and 3 seconds, provisional) | Table of limits in the implementation notes | publisher killed (own-rust); subscribers killed. The scenarios fail above 10.5 seconds for a publisher and 7.5 for a subscriber. Measured: 9.0 seconds for a killed publisher, 6.0 to 6.1 for a killed subscriber. While the idle timeout was 9 seconds, a killed publisher was noticed after 12.0, later than the row said then |
| 7 | After a publisher is gone, the relay holds nothing of it: the track can be published again | Clean-up of a long-running server; the relay refuses a second PUBLISH for a track | every scenario ends with a fresh publisher and subscriber on the same track |
| 8 | When a subscriber leaves, the publisher and the other subscribers are not affected | §9.4, §11.4.1; architecture | subscribers leave |
| 9 | The relay does not wait for a subscriber it cannot reach: what is sent right after a subscriber was killed arrives at the others before the relay knows | Architecture: "The publisher is not made to wait" | subscribers killed: an Object is sent at once, and when the others have it the relay must not have dropped the session of the killed process yet. Another Object is sent after it has |
| 10 | Ending a request stream ends the subscription, not the session | §11.4.1 | subscribers leave in order, and the scenario with the one Object of moqtail-rust: after moqtail-browser cancelled, the relay opens no stream towards it for the Groups sent later, and has not dropped its session |
| 11 | A subscriber that takes nothing loses its subscription with PUBLISH_DONE `TOO_FAR_BEHIND` (`0x5`); its session, the other subscribers and the publisher go on | §8 (MAY), §10.11; architecture; decided for XmoQ | stopped subscriber |
| 12 | A subscriber takes the Objects that arrive after PUBLISH_DONE | §10.11 (SHOULD set a timer) | not checked: no run makes an Object arrive after PUBLISH_DONE, and the order of arrival is not looked at |
| 13 | A subscriber cancels by STOP_SENDING or a reset on the request stream, and the session stays | §5.1, §3.3.2 | not shown for an own client in these runs. own-browser cancels: **fails**, see the defect below. own-rust closes its session right after it cancels, and what it sent on the request stream is not looked at. moqtail-browser cancels with a FIN and a STOP_SENDING as read in its code; no frame was captured, and the relay's log shows only that it ended the request (row 10). Since the defect is fixed and the relay prints the STOP_SENDING it gets: own-browser cancels and the run passes. The relay ends the request, opens no stream for what is sent later and keeps the session; where it printed the STOP_SENDING, the code has to be CANCELLED (`0x1`), and for moqtail-browser the line has to be there with `0x0` |
| 14 | The publisher resets the open streams of a subscription that was cancelled | §5.1.1 (MUST) | not checked: no client reports a reset after it cancelled. own-rust leaves right after it cancels, and own-browser cannot cancel yet (it can since the defect below is fixed; its page does not report a reset after it cancelled, so this is still not checked). Covered by the relay's tests on the in-memory connection |
| 15 | The streams of a subscription that fell behind are reset with `TOO_FAR_BEHIND` (`0x5`) | §3.3.3 (SHOULD); implementation notes | not observed: the stopped subscriber saw no reset, because the streams the relay reset were the ones it had not been able to open. Covered by the relay's tests |
| 16 | The relay closes the request stream with a FIN right after PUBLISH_DONE | §10.11 (SHOULD) | not checked: neither own client reports how the stream closed after PUBLISH_DONE |
| 17 | Ending one data stream early does not affect the subscription | §11.4.1 | not checked: no scenario stops a single data stream |
| 18 | A message or Object for which a session was closed is not passed on to another session | §9.5 (MUST NOT) | not checked: no client here can send an invalid message |
| 19 | A subscriber sends STOP_SENDING on the streams of a subscription when it drops the subscription's state | §10.11 (SHOULD) | not checked. own-browser cannot send one in Chromium (the defect below; it can since that is fixed, and no run has looked at a STOP_SENDING on a data stream) |
| 20 | A control stream is not closed while the session lasts | §3.3 (MUST NOT) | every scenario fails if the relay has printed that it closed a session itself, looked at once, at the end of the run. The relay does so for a client that stops its control stream, unless the client is gone within 200 ms, which the relay takes for leaving; own-rust did so on `wtransport`. Nothing in the runs makes the relay print that line, so a change of its form would go unnoticed |
| 21 | A session is closed with CLOSE_WEBTRANSPORT_SESSION, with a relevant code | §3.5 (the capsule is how a WebTransport session is closed; SHOULD for the code) | not checked: what a client that closes sent is not looked at. The relay printed `ClosedByPeer { code: 0 }` for them |
| 22 | The reset of a stream whose publisher went has a relevant code | §3.3.3 (SHOULD) | publisher killed: own-rust has to see CANCELLED (`0x1`), the code of the notes. While the relay gave either `0x0` or `0x1`, both were accepted |
| 23 | A subscription for which no stream was opened ends with Stream Count 0 | §10.11 (MUST) | not checked: no run ends a subscription that got no Object |
| 24 | A stream reset with RESET_STREAM_AT keeps its header reliable, so that the subscriber can count it | §11.4.3 (SHOULD) | not checked. Whether the relay uses RESET_STREAM_AT was not looked at |

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Relay | `xmoq-relay` 0.0.0 at `68fa616`, `web-transport-quinn` 0.13.2 (`quinn` 0.11.12), `cargo build --locked` (debug). Before the change of library: at `ef0d72b`, `wtransport` 0.7.2, the same `quinn` |
| own-rust | `xmoq-moqt-client` 0.0.0, same commits and builds |
| own-browser | `packages/moqt`, unchanged between the two commits, bundled by `bun` 1.3.12 |
| Browser | Chromium 156.0.8078.4, new headless, launched by `playwright-core` 1.64.0 with `channel: "chromium"` |
| moqtail-rust | `moqtail` 0.15.0, `wtransport` 0.7.2 |
| moqtail-browser | `moqtail` 0.13.0 from npm |
| Toolchain | `rustc` 1.99.0, and the tools of `mise.toml` |
| Wire | MOQT draft 18, WebTransport subprotocol `moqt-18`. The draft text was fetched from datatracker on 2026-10-10 |
| Certificate | Self-signed ECDSA P-256, 13 days, generated by the driver and pinned by its SHA-256 in every client |

Provisional values the runs depend on, none of them project decisions: the relay's idle timeout of 9 seconds and keep-alive of 3 seconds, its limit of 256 streams waiting to be written to one subscriber, and the limits of this driver (30 seconds for a subscriber to learn that a killed peer is gone, 100 seconds for a Rust client).

## Commands

From the repository root:

```sh
mise run interop
```

The runs of this record alone, from `interop/`, and one of them by hand:

```sh
bun matrix.ts --relay own --only "leaving: "
bun disconnect.ts --scenario publisher-killed --publisher own-rust --subscriber own-browser --subscriber own-rust
```

## Observed results

### The runs

`matrix.ts` runs 13 scenarios through the own relay, after the 32 pairings that move one Object. Twelve have to pass and one is listed as failing. This is its table from the run of `mise run interop`:

| Run (own relay) | Result | Seconds |
|---|---|---|
| own-rust publisher is killed; own-browser and own-rust subscribe | the subscribers saw PUBLISH_DONE 12.0s after the kill | 13.3 |
| own-browser publisher's page is closed with a stream open; all four subscribe | the subscribers saw PUBLISH_DONE 0.0s after the kill | 1.8 |
| moqtail-browser publisher's page is closed; own-browser and own-rust subscribe | the subscribers saw PUBLISH_DONE 0.0s after the kill | 0.8 |
| own-browser publisher closes right after its last Object; all four subscribe | every subscriber received the last Object | 1.0 |
| own-browser publisher closes right after its last Object and PUBLISH_DONE; all four subscribe | every subscriber received the last Object | 1.0 |
| own-rust publisher closes right after its last Object and PUBLISH_DONE; all four subscribe | every subscriber received the last Object | 0.9 |
| moqtail-rust publisher closes right after its last Object; own-browser and own-rust subscribe | every subscriber received the last Object | 1.4 |
| subscribers leave in order (own-browser and own-rust close, moqtail-browser cancels); own-rust publishes on | the stayers received 2 Objects sent after the leavers left | 1.1 |
| subscribers are killed (own-browser, own-rust, moqtail-browser); own-browser publishes on | the stayers received 2 Objects sent after the leavers left | 11.0 |
| subscribers leave before the one Object of moqtail-browser (own-browser closes, own-rust and moqtail-rust are killed) | the stayers received 1 Objects sent after the leavers left | 1.0 |
| subscribers leave before the one Object of moqtail-rust (own-browser is killed, moqtail-browser cancels) | the stayers received 1 Objects sent after the leavers left | 1.1 |
| own-browser cancels its subscription; own-rust publishes on | failed as expected (Failed to construct 'WebTransportError') | 0.6 |
| a stopped own-rust subscriber is cut off (TOO_FAR_BEHIND); own-browser and own-rust get everything | the stopped subscriber was cut off with TOO_FAR_BEHIND after 385 Objects; the others received all 386 | 3.2 |

Three of them wait for the relay to give up on a client that vanished: 9 to 13 seconds when this table was made, 6 to 9 since the relay's idle timeout is 6 seconds. They start together, after the 32 pairings, and go on beside the ten others. With everything built, `mise run interop` took 30.0 to 44.6 seconds for the 45 pairings in four runs; it took 17.7 seconds for the 32 before this change. The longer runs are those in which moqtail-rust left without the relay hearing of it (below), which adds 13 seconds to a pairing.

The 13 scenarios were run 20 times in a row with `bun matrix.ts --relay own --only "leaving: "`, 15 times over (sets 1 to 12 with 13 scenarios, sets 13 to 15 with 14):

| Set of 20 rounds | Library of the relay and own-rust | Driver | Rounds in which all 13 ended as expected | Seconds a round |
|---|---|---|---|---|
| 1 | `wtransport` 0.7.2 | Before the time to drop a session was measured per subscriber | 20 | 13.1 to 13.7 |
| 2 | `wtransport` 0.7.2 | Before the wait for a subscription's end was raised from 20 to 30 seconds everywhere | 20 | 13.3 to 14.2 |
| 3 | `wtransport` 0.7.2 | Before the checks for rows 9, 10 and 20 and the bound of row 6 | 19 | 13.3 to 20.3 |
| 4 | `web-transport-quinn` 0.13.2 | The same | 19 | 13.3 to 31.4 |
| 5 | `web-transport-quinn` 0.13.2 | The same | 20 | 13.3 to 13.7 |
| 6 | `web-transport-quinn` 0.13.2 | With those checks, and a bound of 13 seconds, which later sets widened to 15 | 19 | 13.1 to 21.1 |
| 7 | `web-transport-quinn` 0.13.2 | As committed | 20 | 13.1 to 13.5 |
| 8 | The same, and own-rust binds an IPv4 socket for an IPv4 server (`7f98929`) | As committed | 18 | 13.1 to 24.7 |
| 9 | The same | As committed | 20 | 13.1 to 14.9 |
| 10 | The relay with an idle timeout of 6 seconds and CANCELLED for every stream it cuts short (`main` after pull request 17) | With a bound of 12 seconds and CANCELLED alone accepted, and a wait for the relay to print the STOP_SENDING of a page that cancels | 18 | 12.4 to 33.6 |
| 11 | The same | The STOP_SENDING is looked at where it was printed | 20 | 12.3 to 24.3 |
| 12 | The same | Bounds of 10.5 seconds for a killed publisher and 7.5 for a killed subscriber, and the STOP_SENDING of moqtail-browser has to be printed | 20 | 12.5 to 21.4 |
| 13 | The same, and `@xmoq/moqt` with its bound on the streams it takes (64). Run on the driver as it was before set 12: a bound of 12 seconds for a killed client | 14 scenarios, with the page that does not read, one after the other | 20 | 19.2 to 24.9 |
| 14 | The same, on the driver of set 12 | The page that does not read runs beside the three runs that wait | 20 | 13.3 to 24.0 |
| 15 | The same | As committed: also the checks that no stream is opened after the cut-off and that the streams cut short carry TOO_FAR_BEHIND, and at most 833 Objects for the page | 20 | 13.2 to 17.8 |

**Three runs failed, one of 780 before the change of library and two of 780 after it, all because an own-rust client could not connect to the relay.** Nothing of the scenario itself had started in the first, and the scenario was over in the two others.

- Set 3, round 11, scenario with moqtail-rust as the publisher of one Object: the own-rust subscriber that was to stay printed `Error: connection lost: connection timed out` and exited with code 1. The relay printed no `SESSION_ACCEPTED` and no `HANDSHAKE_FAILED` for it, and had accepted four sessions in the same run just before. The run ended 9.9 seconds after it began.
- Set 4, round 13, scenario with moqtail-browser as the publisher of one Object: the own-rust subscriber of the last step, on the same track with a fresh publisher, printed nothing for 20 seconds, not even `CONNECTED`, and the driver gave up. The relay printed `HANDSHAKE_FAILED timed out`, its limit of 10 seconds for the WebTransport handshake. The fresh publisher had connected just before.
- Set 6, round 4, scenario with moqtail-rust closing right after its Object: the own-rust publisher of the last step printed nothing for 20 seconds, and the relay printed `HANDSHAKE_FAILED timed out`.

**Two more runs failed in set 8, in another way: the relay could not start.** In rounds 16 and 18, in two different scenarios, it printed `Error: connection lost: Address already in use (os error 48)` for its own UDP port (44603 and 44612) and exited. No relay of an earlier run was left: every run stops its relay and waits for it to exit. Set 9 ran with a watcher that listed, 20 times a second, every other process holding one of those ports; it saw none, and no run failed. What held the ports is not known. It did not happen in the 1820 runs before.

No client failed to connect in sets 7 to 9. Seven of the 13 scenarios connect a moqtail-rust client, eight connections a round; that peer still binds a socket for IPv4 and IPv6 and can fail as described below.

Up to four other relays and their clients were running on the machine each time. A cause was found afterwards, in [its own record](2026-10-10-client-handshake-timeout.md): the client binds a socket for IPv4 and IPv6, and when the kernel gives it a port that another process holds with an IPv4 socket, the relay's answers go to that process. It reproduced with one relay and no load, twice in 38,000 connections. That these three runs failed for it is not shown, as their ports were not recorded. The relay went on accepting connections. Until the client binds differently, the same can fail the CI job.

Every scenario ends with a fresh own-rust publisher and subscriber moving one Object over the same track. Where they connected, it passed in every run: the relay had let go of the publisher that left (row 7).

### What the relay's shorter idle timeout changed

Sets 10 and 11, 40 rounds, on `main` after the relay's idle timeout went from 9 to 6 seconds and it began to print the STOP_SENDING it receives.

| Measured | Before | Now |
|---|---|---|
| Time from killing an own-rust publisher to PUBLISH_DONE at the subscribers | 12.0 seconds | 9.0 seconds in 40 of 40 |
| Time from killing an own-rust subscriber to the relay dropping its session | 9.0 to 9.1 seconds | 6.0 to 6.1 seconds |
| Reset code own-rust sees on a stream its publisher left open | `0x0` and `0x1`, about half each | `0x1` on 240 of 240 streams |
| The relay learns that moqtail-rust closed its session by its idle timeout only | 13 of 40 runs, 12 seconds later | 13 of 40 runs, 9 seconds later |
| A client that could not connect; a relay that could not start; a session the relay closed itself | 3 of 1560; 2 of 520; none since `web-transport-quinn` | none in 520 runs |

**The code of a STOP_SENDING sent from a browser reaches the relay.** When the own page cancelled its subscription, the relay printed `STOP_SENDING_RECEIVED session=1 stream=1 code=0x1` in 37 of the 40 rounds: CANCELLED, the code `@xmoq/moqt` puts into the `WebTransportError` it cancels the reader with. In the other 3 it printed nothing for that stream. It prints the line only where it sees the STOP_SENDING before the reset of the other direction, which a client that cancels sends as well; in those rounds it had ended the request for the reset. The two failed rounds of set 10 are those: the driver waited for the line. It now compares the code where the line is there, and no other code than `0x1` was printed. For moqtail-browser, which cancels its reader without a reason, the relay printed `code=0x0` in 20 of 20 rounds.

Set 12, 20 more rounds, with the bounds for a killed client put between what is measured now and what was measured with the idle timeout of 9 seconds (10.5 seconds for a publisher, where 9.0 and 12.0 were measured; 7.5 for a subscriber, where 6.0 to 6.1 and 9.0 to 9.1 were): 9.0 seconds in 20 of 20 and 6.0 to 6.1 seconds, no round failed. The 12 seconds of sets 10 and 11 would have let the old timeout pass. The own page's STOP_SENDING was printed in 16 of these 20 rounds, 53 of 60 in all. The line of moqtail-browser was printed with `code=0x0` in both runs in which it cancels, 40 of 40, and the driver now fails without it: that client ends its direction with a FIN and sends no reset, so nothing overtakes its STOP_SENDING.

**A reset with another code than CANCELLED is possible and was not seen.** The relay gives CANCELLED to the streams of a publisher whose session ended. Where the publisher resets a stream itself, the relay hands on the publisher's code (`0x0` if there is none). A browser that reset its streams before it closed the session of a page that is closed would get another code to the subscribers, and the runs would fail. In the 20 rounds of set 12 the relay printed the end of the publisher's session before every one of the 111 resets it sent for a closed page (71 for own-browser, 40 for moqtail-browser), all with `0x1`. Whether Chromium can send the resets first was not found out.

That is for a request stream. A STOP_SENDING on a data stream was not looked at here: the relay notices one only at its next write to that stream, and no scenario has a subscriber stop a stream the relay is still writing.

The three runs that wait for the idle timeout wait 6 to 9 seconds now. They run beside the others, whose ten runs take about 12 seconds together, so what the shorter timeout took off the 13 scenarios is small: the fastest round went from 13.1 to 12.3 seconds. A still shorter timeout, set with the relay's `--idle-timeout-ms`, would take off nothing more, and the scenarios run with the relay's default.

### What the change of library changed

| Measured | `wtransport` 0.7.2 | `web-transport-quinn` 0.13.2 |
|---|---|---|
| The relay learns that own-rust closed its session | At once in 36 of 71 runs, 9 seconds later in 35 | At once in 40 of 40 runs (5 to 48 ms) |
| The relay closes a session with `the peer stopped the relay's control stream` | In some runs, for own-rust clients that had just finished | In none of 40 rounds |
| Time from killing an own-rust publisher to PUBLISH_DONE at the subscribers | 12.0 seconds in 59 of 60 runs, 11.8 in one | 12.0 seconds in 40 of 40 |
| Time from killing an own-rust subscriber to the relay dropping its session | 9.0 seconds | 9.0 to 9.1 seconds |
| Reset code own-rust sees on a stream its publisher left open | `0x1` in most runs, `0x0` in one that was noticed | `0x0` in 121 streams, `0x1` in 118 |
| The relay learns that moqtail-rust closed its session, in the scenario where it closes right after its Object | 12 seconds later in 20 of 71 runs | 12 seconds later in 13 of 40 |
| The stopped subscriber is cut off | After 385 Objects, 0.6 to 0.7 seconds | After 385 Objects, 0.4 to 0.8 seconds |
| `Subscription.ended` in own-browser | `publish-done` with status 2, or `session-closed` after its own `close()` | The same |

The first two rows were defects of own-rust that the change of library removed. The scenario in which own-rust leaves in order took 2 or 11 seconds before and takes 2 seconds now, so it no longer runs beside the others.

### A publisher that is killed

| Publisher | How it went | Subscribers that report | What they saw |
|---|---|---|---|
| own-rust | `SIGKILL` after its Object arrived | own-browser, own-rust | PUBLISH_DONE `0x2`, Stream Count 1, 12.0 seconds after the kill |
| own-browser | Page closed, with one stream finished and one left open | own-browser, own-rust | PUBLISH_DONE `0x2`, Stream Count 2, 20 ms after the page was closed in the run looked at. Both Objects arrived. own-rust saw the open stream end with a reset |
| moqtail-browser | Page closed after its Object | own-browser, own-rust | PUBLISH_DONE `0x2`, Stream Count 1. own-rust saw the stream end with a reset: moqtail keeps the stream of a Group open |

The relay sent PUBLISH_DONE `TRACK_ENDED` in every case (rows 1, 2 and 4), with a Stream Count equal to the streams each own subscriber got (row 3). own-rust reported `SUBSCRIPTION_ENDED why=PUBLISH_DONE, all streams closed` each time: every counted stream had ended before its wait of 2 seconds was over. That does not show that the streams were closed before PUBLISH_DONE was sent.

**The kill is noticed after 12 seconds, not 9**, with either library. The relay's idle timeout is 9 seconds and its keep-alive 3 seconds. A reading that fits, not verified in the relay: by RFC 9000 §10.1 the idle timer also restarts when an endpoint sends its first ack-eliciting packet after receiving one, and the relay's first keep-alive goes out 3 seconds after the publisher's last packet. Where the relay had sent something right after the last packet it received, the wait was 9 seconds (below).

**The reset code of an open stream is not always the same.** The relay reset the subscribers' stream with CANCELLED (`0x1`) where it had seen the PUBLISH end first, and with INTERNAL_ERROR (`0x0`) where it had seen the publisher's stream break off first: its log then has no `STOP_DATA` for that stream, and it passes on a reset without a code. With `web-transport-quinn` each happened in about half of the runs, for own-browser and for moqtail-browser alike; with `wtransport` the second was rare. The scenario accepts both codes. What a moqtail subscriber sees of these resets is not known: the moqtail clients do not report it.

### A publisher that closes right after its last Object

| Publisher | What it does | Result |
|---|---|---|
| own-browser | Writes the Object, closes the stream, closes the session | All four subscribers received the Object; own-browser and own-rust then saw PUBLISH_DONE `0x2`, Stream Count 1 |
| own-browser | The same, with PUBLISH_DONE before closing the session | The same |
| own-rust | Reads "send" and the end of its input together: Object, PUBLISH_DONE, session closed | The same |
| moqtail-rust | The same input; it has no PUBLISH_DONE and calls `connection.close()` | Both own subscribers received the Object and saw PUBLISH_DONE `0x2` |

The Object arrived in every run (row 5). Whether it arrived before or after PUBLISH_DONE was not looked at, so row 12 is not shown. With moqtail-rust the run took either about 1.5 seconds or about 13.5, the latter in 13 of 40 runs (20 of 71 before the change of library, which did not touch this peer): in those the relay learned that the publisher was gone by its idle timeout only. The same happened once in a pairing that moves one Object, `moqtail-rust to own-browser, then PUBLISH_DONE`, which took 13.4 seconds where it takes 0.6; the wait of that driver for the subscription's end is 30 seconds now, as here. The peer returns from `main` right after `connection.close()`, so the likely cause is that the process ends before the close is sent; this was not verified with a packet capture.

moqtail-browser is not a publisher here. Its `sendObject` queues the Object, and its `disconnect()` ends the PUBLISH before the Object is written: the relay sent PUBLISH_DONE with Stream Count 0 and was never given the Object.

### Subscribers that leave

| Publisher | Who left, and how | Who stayed | Result |
|---|---|---|---|
| own-rust | own-browser closed its session; own-rust cancelled and closed its session; moqtail-browser cancelled | own-browser, own-rust | The stayers received Group 1, sent right after, and Group 2, sent after the relay had dropped the two sessions, and then PUBLISH_DONE `0x2` with Stream Count 3. The relay opened no stream towards moqtail-browser for Groups 1 and 2 and had not dropped its session (row 10) |
| own-browser | own-rust killed with `SIGKILL`; the pages of own-browser and moqtail-browser closed | own-browser, own-rust | The same, and the relay had not dropped the session of own-rust when the stayers had Group 1 (row 9). It dropped the pages' sessions 60 to 90 ms after they were closed and the session of own-rust 9.0 seconds after it was killed. The publisher's own end was `{"type":"done"}` |
| moqtail-browser | own-browser closed its session, own-rust and moqtail-rust were killed, all before the one Object | own-browser, own-rust, moqtail-rust | Every stayer received the Object, before the relay had dropped the sessions of the two killed processes |
| moqtail-rust | own-browser's page was closed and moqtail-browser cancelled, both before the one Object | own-browser, own-rust, moqtail-browser | Every stayer received the Object |

Rows 8 and 9 held in every run. The publishers were not disturbed: own-rust sent on and exited with code 0, and own-browser's subscription ended by its own PUBLISH_DONE.

Three things the clients did when they left, found here:

- **own-rust closed its session, and the relay often did not hear of it. Gone with the change of library.** With `wtransport`, in 35 of 71 runs the relay dropped the session of the own-rust subscriber 9 seconds after it had left, by the idle timeout. The client now waits until its close is done, and the relay dropped the session within 48 ms in 40 of 40 runs.
- **own-rust stopped the relay's control stream before it closed its session. Gone with the change of library.** With `wtransport`, in some runs the relay printed `CLOSE_SESSION session=N code=0x3 reason="the peer stopped the relay's control stream"` for a client that had just finished, against §3.3: "A control stream MUST NOT be closed at the underlying transport layer during the session's lifetime." The line did not appear in 40 rounds since.
- **`disconnect()` of moqtail 0.13.0 does not close the session.** It closes its request streams and then tests `webTransport.closed`, a promise, before calling `close()`, so the call is never made (`dist/client-*.mjs` of the npm package, read on 2026-10-10). The relay kept the session of a page that had called it. The scenarios use it as a way to cancel: on each request stream it closes its own direction, a FIN, and cancels the reader, which a browser sends as STOP_SENDING (`RequestStream.close` in the same file; the frames were not captured). The relay answered by finishing its direction (`FINISH_REQUEST`). §3.3.2 and §5.1 name STOP_SENDING and a reset as the way to cancel, not a FIN.

### A browser subscriber that cancels: fails

Fixed since, in [the record of the stream errors](2026-10-10-webtransport-error-in-chromium.md): the run passes and is no longer listed as failing. What follows is what was found then.

`Subscription.cancel()` of `@xmoq/moqt` rejected in Chromium 156 with `TypeError: Failed to construct 'WebTransportError': The provided value is not of type 'WebTransportErrorInit'`, and the relay then dropped the session. `packages/moqt/src/webtransport/connect.ts` builds the error for every reset and every STOP_SENDING as `new WebTransportError(message, { streamErrorCode })`. In Chromium 156 the constructor takes one dictionary:

| Call, in a page of Chromium 156.0.8078.4 | Result |
|---|---|
| `new WebTransportError("m", { streamErrorCode: 1 })` | throws the TypeError above |
| `new WebTransportError({ message: "m", streamErrorCode: 1 })` | an error with `source` "stream" and `streamErrorCode` 1 |
| `new WebTransportError("m")` | throws |

So in a real browser the library cannot cancel a subscription, reset a subgroup stream, or send STOP_SENDING with a code. The package's tests play the transport themselves and do not build this error. The scenario is in the matrix as a run that has to fail with this message (row 13), with a `FIXME:` to turn it into a passing one.

### A subscriber that takes nothing

An own-rust subscriber was stopped with `SIGSTOP` after it had received one Object. own-browser then published Groups of one Object each.

- After 385 Objects, 0.4 to 0.8 seconds after the stop, the relay sent the stopped subscriber PUBLISH_DONE `TOO_FAR_BEHIND` (`0x5`) with Stream Count 2^62 - 1, and did not drop its session.
- The two other subscribers, own-browser and own-rust, received all 386 Objects and then PUBLISH_DONE `0x2` with Stream Count 386 (row 11).
- When it was let run again, the stopped subscriber printed `PUBLISH_DONE status_code=0x5` and had taken 98 Objects, which had reached it before the relay ran out of the stream credit it had given. It saw no reset (row 15) and gave up waiting for further streams after its 2 seconds, as the count told it nothing.

With own-rust as the publisher the cut-off came after 8.1 seconds (measured before the change of library, not again), too close to the 9 seconds after which the relay drops a silent session, so the matrix uses own-browser. own-rust waits for the acknowledgement of each stream before it sends the next and sent about 48 Objects a second.

**A browser subscriber could not be made slow.** This was tried first, with temporary variants of the page that are not in the tree:

| Subscriber | Publisher | Result |
|---|---|---|
| own-browser, subscribed and never reading `Subscription.objects` | own-rust, 57-byte Objects | Not cut off. The page's session logged all 1428 Objects sent in 30 seconds |
| The same | own-browser, 300,000-byte Objects | Not cut off. The session logged all 936 Objects, about 280 MB |
| moqtail-browser, whose page reads one Object and no more | own-rust, 57-byte Objects | Not cut off after 1362 Objects |

Two things are behind it. Chromium takes a short stream off the connection whether or not the page reads it, so the relay never runs out of stream credit. And `@xmoq/moqt` holds more than its `subscriptionHighWaterMark` of 16 Objects: every subgroup stream reads up to its first whole Object and only then waits for room, so with one Object per stream the queue grew by one Object per stream without a bound. The second is a defect of the package against the rule that receive queues are bounded.

**Since `@xmoq/moqt` takes a bounded number of the peer's streams at once (64 by default), a page that does not read is cut off**, and the matrix has it as a 14th scenario (`--slow unread-own-browser`). The page subscribes and never reads `Subscription.objects`; an own-browser publisher sends 57-byte Objects, one a stream; an own-browser and an own-rust subscriber read. 20 rounds each:

| Measured | Set 13, one after the other | Sets 14 and 15, beside the three runs that wait |
|---|---|---|
| Rounds in which the page got PUBLISH_DONE `TOO_FAR_BEHIND` with Stream Count 2^62 - 1, its session stayed, and the two readers got every Object | 20 of 20 | 40 of 40 |
| Objects the publisher had sent when the driver saw the cut-off (it looks after every 64) | 577 or 641 | 577 in 37 rounds, 641 in 3 |
| Time from the first Object the page did not take to the cut-off | 0.8 to 1.6 seconds | 1.3 to 3.5 seconds |
| Time of the scenario | 6.7 to 7.8 seconds | 8.3 to 10.9 seconds |
| Time from killing an own-rust publisher to PUBLISH_DONE, in the run beside it | 9.0 seconds, 20 of 20 (set 12, without this scenario) | 9.0 seconds, 40 of 40 |
| Time from killing an own-rust subscriber to the relay dropping its session, in the run beside it | 6.0 to 6.1 seconds (set 12) | 6.0 to 6.1 seconds |

**The scenario runs beside the three that wait.** Its first seconds are busy, and they are the first seconds of the runs that have a bound of 10.5 and 7.5 seconds on how long the relay takes to notice a killed client. Those times did not move in 40 rounds, and no round failed. A round of the leaving scenarios takes 13.2 seconds at best with it, where it took 12.5 without and 19.2 with the scenario run alone. The scenario itself is slower beside the others; what it is judged on is counted, not timed.

What the numbers count, in one run looked at line by line. The publisher had sent 641 Objects when the driver saw the cut-off. The relay had taken 603 of them by then and printed `OPEN_DATA` for 603 streams towards the page, which it prints when it books a stream and not when the stream is open. It reset 257 of the 603, the 256 that waited and the one that made it cut the subscription off, with `RESET_DATA … code=0x5`, TOO_FAR_BEHIND. So 346 streams reached the page's browser, of which the library took 64. In sets 14 and 15 every run reset 257 streams, and every code was `0x5`; the stopped own-rust subscriber of the other scenario was cut off after 385 Objects in 20 of 20, also with 257 resets of `0x5`.

The driver checks these since set 15: the relay opens no stream towards the page for the Group sent after the cut-off (§10.11), the session is still there after the page's wait, every stream the relay reset has code `0x5` and there is at least one (§3.3.3), and the publisher needs at most 833 Objects, so that a library that takes a few hundred streams more fails. The code is read from the relay's log: the page never sees those streams, as they were not opened. With the expected code changed, the run failed.

The page reports the end about 5 seconds after the cut-off: the Stream Count says nothing, so the library waits for late streams (`DEFAULT_PUBLISH_DONE_TIMEOUT_MS` in `packages/moqt/src/webtransport/connection.ts`).

Not in the matrix: a page that does not read while each stream carries one large Object. There the page was not told that its subscription had ended ([record](2026-10-10-subscription-backlog-in-chromium.md)), a known gap.

**A STOP_SENDING on a data stream was not produced by this scenario either.** The Objects are small, so the relay had finished every stream it opened before the page stopped anything, and it printed no `STOP_SENDING_RECEIVED … out=` line. A stream the relay is still writing when the subscriber stops it needs an Object too large for the flow control window, which is the case above that is not in the matrix.

### How `Subscription.ended` resolved in own-browser

| Scenario | `ended` | `objects` |
|---|---|---|
| The publisher ends its Track with PUBLISH_DONE (the 32 pairings) | `publish-done`, status 2 | closed |
| The publisher is killed, or its page closed | `publish-done`, status 2 | Delivered what had arrived, also the Object of the stream that was reset, then closed |
| The publisher closes right after its last Object, with or without PUBLISH_DONE | `publish-done`, status 2 | Delivered the Object, then closed |
| Another subscriber leaves, in any way | Not resolved until the track ends, then `publish-done`, status 2 | Every Object, then closed |
| The page closes its own session (`close()`) | `session-closed` | Closed at once |
| The page cancels (`cancel()`) | Not reached: `cancel()` rejects with a TypeError (above) | |
| The relay cuts the subscription off (`TOO_FAR_BEHIND`) | `publish-done` with status `0x5`, after the library's wait for late streams, since the page that does not read is cut off. When this table was made: Not produced: no page could be made slow (above) | |

`peer-closed` and `peer-reset` did not occur. The own relay ends a subscription it serves with PUBLISH_DONE in every case reached here, which is what §11.4.1 asks for, so `publish-done` is the expected reason in all of them and none differs from the draft. `session-closed` by the relay did not occur either: no scenario makes the relay close the session of a browser subscriber, so none of the expected values here depends on what a browser sees of a session the relay closes.

## Limitations and blockers

- **Not run on GitHub.** The runs were made on macOS only. The time the CI job takes with them is not measured.
- **Three runs of 1560 failed because a client could not connect to the relay** (above), all with an own-rust client that bound a socket for IPv4 and IPv6. own-rust binds differently since `7f98929`; moqtail-rust does not, so the same can still fail the CI job.
- **Two runs of 520 after that failed because the relay's port was in use**, for a reason that is not known (above).
- Fixed since, see the section on the defect; kept as it was written. **One run is expected to fail**, the browser subscriber that cancels, until `@xmoq/moqt` builds `WebTransportError` the way Chromium takes it. Until then rows 13 and 14 are not shown for the browser client.
- When this was written: **The slow subscriber is a stopped process, not a slow reader.** It takes nothing at all and acknowledges nothing. A reader that is merely slower than the publisher is not covered, and neither is a browser. Since set 13 a browser is covered, by a page that does not read; it too takes nothing at all, and a reader that is merely slower is still not covered.
- **Three runs depend on the relay's idle timeout** and so on a provisional value. They run beside the others to keep the matrix short, which puts up to four Chromium instances on the machine at its start.
- **The driver reads six kinds of lines of the own relay's log**: `SESSION_OPENED`, `SESSION_CLOSED`, `FINISH_REQUEST` or `RESET_REQUEST`, `OPEN_DATA`, the `SEND` of a PUBLISH_DONE with status `0x5`, and `CLOSE_SESSION`. A killed client says nothing and a stopped one reads nothing, so there is no other place to wait. A change to the form of the first five fails the runs, because each is waited for at least once in every run. A change to `CLOSE_SESSION` does not: that line only has to be absent.
- **Which session belongs to which client is taken from the order in which the clients connect.** The relay prints no address. A client that connected out of turn would be mistaken for another.
- **That own-rust saw no stream end without a FIN is checked with nothing in the same run that makes it print one**, in the runs where the publisher finishes every stream. The runs where a publisher leaves a stream open do make it print the line.
- **Only the own relay.** The scenarios were not run through the moqtail relay.
- Rows 12 to 19, 21, 23 and 24 of the table at the top are not checked by any run, and of row 3 only the count is.
- **Sets 1 to 5 of 20 rounds were run before the checks for rows 9, 10, 20 and the bound of row 6 were added**, and with moqtail-rust among the killed subscribers of the scenario that sends several Objects. It had left by itself by then, so it was taken out there and is killed in the scenario where the leavers go before the one Object.
- `spikes/moqtail/browser/relay-scenarios.mjs` is kept as it was. Its three disconnect scenarios are covered by the runs above; its fourth, a SUBSCRIBE for a track nobody published, was in the matrix already.

## Open questions raised

- Settled since: the relay gives CANCELLED. The question was: Which Stream Reset Error Code should the relay give a subscriber's stream when the publisher vanishes: CANCELLED, INTERNAL_ERROR as passed on now in some runs, or SESSION_CLOSED? It gives two different ones for the same event today.
- It is 9 seconds now. The question stands for that value: Is 12 seconds to notice a killed publisher acceptable for a live broadcast, where every viewer waits that long for the end? The value follows from the provisional idle timeout and keep-alive.
- How should a slow browser viewer be bounded, when Chromium accepts short streams without the page reading them? The relay's limit does not see such a viewer; the queue in `@xmoq/moqt` is the only place left.
