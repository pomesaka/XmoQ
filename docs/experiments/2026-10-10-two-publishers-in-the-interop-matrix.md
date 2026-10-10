# Two publishers of one track in the interop matrix

- Date: 2026-10-10
- Commit SHA: the commit that adds the scenarios `same-group-twice` and `second-source` to `interop/disconnect.ts` together with this record. The runs were made on its working copy on top of the change that makes the relay accept a second PUBLISH for a track, which was not in `main` then: first at `a2fc1e9d`, then at `34fbe9f8` and at `a44c8854`, which are that change after the first and the second round of its review
- Roadmap item: Phase 0b, "Several publishers of one track in the relay, before Phase 1". This record covers the interop runs for the part of it that accepts a second PUBLISH; it closes no item

## Objective

Put runs with two publishers of one track into the interop matrix, so that CI checks what the relay does with them through real clients and a real Chromium: that both are accepted, that a Subgroup two of them send reaches a subscriber once, and that a subscription goes on when one of them leaves.

### What a run has to show

From [the implementation notes](../moqt-implementation-notes.md), section "Relay". The expectations were written from these rows and the relay's code before the runs were made, and none was changed afterwards.

| # | Requirement | Row of the notes | Checked by |
|---|---|---|---|
| 1 | A PUBLISH for a track that already has a source is accepted, whether or not the track has subscribers | "PUBLISH for a track that already has a source"; §9.3, §9.5 | every run: both publishers report that they published. `second-source --second-joins after`: with subscribers there |
| 2 | A Subgroup comes from the source that sent it first; the stream of another source for it is stopped with CANCELLED and nothing of it goes downstream | "A stream for a Subgroup from another source…"; §2.2 | `same-group-twice`: the relay prints `STOP_DATA` for the second publisher's session with `0x1` and one `OPEN_DATA` of the Group towards each subscriber, and every own subscriber has printed one Object for the Group when its subscription has ended, with the first publisher's payload where the two send different ones |
| 3 | A source that goes while another is left changes nothing for the subscriptions | "A source of a track that goes while another is left"; §9.4 | both scenarios: no PUBLISH_DONE was sent to a subscriber when the relay had closed the session of the first publisher, which it prints after everything else that session's end made it do. A page that ends its Track is closed for that. The line is seen once for each own subscriber when the track has ended, so that a change of its form is noticed |
| 4 | The next Subgroup comes from whichever source sends it | the same row | `second-source`: the two publishers send different payloads, and the Group after the first left has the second's |
| 5 | A stream the relay had open for a Subgroup of the source that went is reset with CANCELLED | the same row; §11.4.3 | `second-source --first kill-with-open-stream`: the own-rust subscriber sees one stream end without a FIN, with code `0x1` |
| 6 | The track ends with its last source: PUBLISH_DONE `TRACK_ENDED` | "What ends the upstream subscriptions of a track" | every run, at every own subscriber |
| 7 | Stream Count is the number of streams the relay opened for the subscription | "Stream Count in the PUBLISH_DONE the relay sends"; §10.11 | every run: 1 where two sources sent one Group; the number of Groups elsewhere, the reset stream among them |
| 8 | A killed source is noticed by the idle timeout | table of limits | `second-source --first kill`: fails above 10.5 seconds |

| 9 | With two sources, a stream for a Group older than the relay remembers a source for is stopped and not opened towards the subscribers a second time | "A stream for a Subgroup from another source…"; §2.2 (MUST NOT) | `same-group-twice --first-sends 10`: `STOP_DATA` for the second publisher, one `OPEN_DATA` of Group 0, and the subscriber has printed one Object for each of the ten Groups, each with the first publisher's payload |
| 10 | A publisher whose every counted stream the relay stopped still gets a FIN on its request stream after PUBLISH_DONE: a stream the relay stopped has ended for it | "A publisher that ended its PUBLISH with PUBLISH_DONE…"; fixed by a test of the relay core | `same-group-twice`: the second publisher, where it is an own client |

| 11 | The same on a track that had two sources and is back to one: what the subscribers have from the source that left is not opened towards them again | the same row; §2.2 (MUST NOT) | `second-source --first done --first-sends 10 --second-resends`: ten `STOP_DATA` for the second publisher, one `OPEN_DATA` for each of the Groups 0 to 9, one Object printed by the subscriber for each of them with the first publisher's payload, and one for Group 10 with the second's |

Requirement 11 was added with the second round of the review. Requirements 9 and 10 were added after the first runs: 9 with the review of the relay change, 10 when the project owner's side confirmed that the FIN the first runs had only recorded is intended.

Not in the matrix: the fifth publisher of a track (`EXCESSIVE_LOAD`), which needs five clients and is covered by the relay's tests.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Browser | Chromium 156.0.8078.4, new headless, `playwright-core` 1.64.0 |
| Relay and own-rust | `xmoq-relay` and `xmoq-moqt-client` 0.0.0, debug builds, at `a2fc1e9d` for the first runs, at `34fbe9f8` for the second and at `a44c8854` for the last; the client has `--first-group` since that change |
| moqtail peers | relay 0.15.0, Rust client and browser page as prepared by `interop/README.md` |
| MoQ Transport | draft 18 |

## Commands

From the repository root:

```sh
mise run check
mise run interop
cd interop && bun matrix.ts --relay own --only "leaving: " --base-port 47000   # 20 times in a row
```

Three checks were made with a temporary change that was taken back afterwards and is not in the tree:

- In `crates/relay-core/src/lib.rs`, a stream of another source for a Subgroup that was delivered whole was let pass instead of being stopped; then `same-group-twice` with two own-rust publishers.
- In `interop/disconnect.ts`, the Group of the second publisher was expected with the payload of the first; then `second-source --first done`.
- In `interop/disconnect.ts`, the check that the subscribers are still subscribed was put after the last publisher had left; then `second-source --first done`.

## Observed results

### The scenarios

Nine runs through the own relay, on two scenarios of `disconnect.ts`. The parts they need were added to the driver beforehand, without a change for the runs there were: a label, a first Group and a payload for each publisher, `--publisher` given twice, the number of a publisher's session, and a Stream Count a scenario can give.

| Run | Publishers | Subscribers | What happens |
|---|---|---|---|
| 1 | own-rust, own-rust | own-rust, own-browser | both send Group 0 with the same payload, one after the other |
| 2 | moqtail-rust, own-rust (at first the other way round, see below) | own-rust, moqtail-rust | the same |
| 3 | own-browser, own-rust | own-rust | the same |
| 4 | own-rust, own-rust | own-rust | the first sends Group 0 and leaves with PUBLISH_DONE; the second sends Group 1 |
| 5 | own-rust, own-rust | own-rust | the first sends Group 0 and is killed; the second sends Group 1 |
| 6 | own-browser, own-rust | own-rust, own-browser | the first sends Group 0, opens a stream for Group 1 and leaves it open, and its page is closed; the second sends Group 2 |
| 7 | own-rust, own-rust | own-rust | the first sends Group 0; then the second publishes and sends Group 1; then both leave |
| 8 | own-rust, own-rust | own-rust | the first sends Groups 0 to 9; then the second sends Group 0, with another payload |
| 9 | own-rust, own-rust | own-rust | the first sends Groups 0 to 9 and leaves with PUBLISH_DONE; then the second sends Groups 0 to 9 again, with another payload, and Group 10 |

In run 9 the second publisher sends all ten Groups again and not Group 0 alone: the Rust client numbers its Groups in a row from `--first-group`. So the run covers the two Groups the relay no longer remembers a source for (0 and 1, with ten sent and eight remembered) and the eight it does remember.

In runs 1 to 3 the second publisher sends only when the subscribers have the Group of the first, so that which source is first is not a race.

### Results

On the relay change as first written (`a2fc1e9d`), with runs 1 to 7:

| Run | Result |
|---|---|
| Each of the seven, once, as first written | passed |
| `mise run check` | passed, 392 TypeScript tests |
| `mise run interop` | 53 of 53 pairings in 33.6 seconds (46 before) |
| The 21 leaving scenarios, 20 rounds | 20 of 20 rounds, 16.0 to 25.4 seconds a round (13.2 to 13.9 with 14 scenarios) |
| Run 8, written then, before the relay was changed for it | **failed**, as it had to: `timed out waiting for relay to print /^STOP_DATA session=1 …/`. The relay's log had a second `OPEN_DATA session=2 … group=0`, and the subscriber printed Group 0 twice, the second time with the second publisher's payload |

On the relay change after the first round of its review (`34fbe9f8`), with runs 1 to 8:

| Run | Result |
|---|---|
| `mise run check` | passed, 392 TypeScript tests |
| `mise run interop` | 54 of 54 pairings in 35.1 seconds |
| The 22 leaving scenarios, 20 rounds | 20 of 20 rounds, 16.8 to 26.2 seconds a round |
| Run 8 | passed in every round |
| Run 9, written then, before the relay was changed for it | **failed**, as it had to: `the relay stopped 8 of the 10 streams pub2:own-rust sent again …`. The relay's log had a second `OPEN_DATA` towards the subscriber for Group 0 and for Group 1, the two Groups it no longer remembered a source for, and the subscriber printed both with the second publisher's payload. The other 22 scenarios passed beside it |

On the relay change after the second round (`a44c8854`), with runs 1 to 9 as committed:

| Run | Result |
|---|---|
| `mise run check` | passed, 392 TypeScript tests |
| `mise run interop` | 55 of 55 pairings in 35.1 seconds |
| The 23 leaving scenarios, 20 rounds | 20 of 20 rounds, 17.3 to 26.6 seconds a round |
| Run 9 | passed in every round; the relay printed ten `STOP_DATA` for the second publisher in each, 200 in all |

On `main` after the relay change was merged (`05dff21d`), with the checks as the review left them (the section on the review, below):

| Run | Result |
|---|---|
| `mise run check` | passed, 392 TypeScript tests |
| `mise run interop` | 55 of 55 pairings in 35.4 seconds |
| The 23 leaving scenarios, 20 rounds | 20 of 20 rounds, 17.3 to 35.9 seconds a round. A killed own-rust publisher was noticed after 9.0 seconds in 20 of 20 and after 9.002 to 9.007 in run 5; a killed own-rust subscriber after 6.0 to 6.1 |

Measured in the 20 rounds on `a44c8854`; the earlier ones gave the same within a few milliseconds, except that the slowest killed subscriber was noticed after 6.16 seconds here and 6.08 before:

| Measured | Value |
|---|---|
| Time from killing the first own-rust publisher to the relay dropping its session (run 5) | 9.002 to 9.007 seconds |
| Time from killing the one own-rust publisher to PUBLISH_DONE, in the run beside it | 9.0 seconds in 20 of 20; the bound is 10.5 |
| Time from killing an own-rust subscriber to the relay dropping its session, in the run beside it | 6.0 to 6.2 seconds; the bound is 7.5 |
| How a second own-rust publisher saw the relay end its request stream after PUBLISH_DONE | `FIN` every time, also where its one stream had been stopped (runs 1, 2, 3 and 8) |

Run 5 waits for the idle timeout and runs beside the four that already did; with five such runs side by side, the times of a killed client did not move.

### The moqtail Rust client is the first publisher of run 2, not the second

Run 2 first had own-rust send first and moqtail-rust second, so the relay stopped the moqtail client's stream. In the first 20 rounds that client sent, was stopped, and left with exit code 0 every time. It is the other way round now, for what the relay change's author found in the client's source and in runs of his own: `moqtail` 0.15.0 returns a transport error from `send_object` and `finish` on a stream that was stopped, so the client ends with an error when the STOP_SENDING arrives before it has written the stream out; he saw that in 2 of 24 runs and in 1 of 25. That is the peer's behaviour and not a fault of XmoQ, and a run that expects it either way would fail now and then. So no run has a moqtail client as the publisher whose stream is stopped: in run 2 it is the source the Group is taken from, and no other run has a moqtail publisher beside another one.

### The checks fail when what they check is broken

Each row is a temporary change that was taken back; `crates/` had no difference afterwards. The first three were made on the first state of the relay change (`a2fc1e9d`), the others on `main` after it was merged (`05dff21d`), with the checks as committed.

| Temporary change | Result |
|---|---|
| The relay lets the second source's stream pass (the relay core, first state) | run 1 fails: `timed out waiting for relay to print /^STOP_DATA session=1 …/` |
| The second publisher's Group expected with the first one's payload (the driver) | run 4 fails: `sub1:own-rust received … in Group 1, not the sent payload …` |
| "Still subscribed" checked after the last publisher left (the driver) | run 4 fails: `the relay ended the subscription of sub1:own-rust …, while the track had another source: SEND session=2 stream=1 PUBLISH_DONE …` |
| **The relay ends a track whenever one of its sources goes** (`source_gone` in the relay core) | runs 1, 2, 3 and 4 fail: `the relay ended the subscription of sub1:own-rust when pub1:… had left, while the track had another source`. Run 2 has the moqtail client as the first publisher and run 3 the own page |
| **The relay core of `a2fc1e9d`, and the two checks on the relay's log switched off in the driver** (no wait for `STOP_DATA`, no count of `OPEN_DATA`) | run 8 fails on what the subscriber printed: `sub1:own-rust got 2 Objects for Group 0, not the one of pub1:own-rust: [that of pub1:own-rust, 7365636f6e64…]` |
| **The relay core of `34fbe9f8`, and the same two checks off** | run 9 fails in the same way, for Group 0 |
| **A track that has ended stays in the relay with its sources** (`end_track` in the relay core) | the step after the scenario fails in `publisher-killed`: `timed out waiting for sub-end:own-rust to print /^SUBSCRIPTION_ENDED …/` |

### What a review changed in the checks

A review of the scenarios found two checks that passed where they should not, and a third was found beside them. All three are mended, and the rows in bold above were made to show it.

- **"No PUBLISH_DONE yet" was looked at too early** where the first publisher is a moqtail client, which only exits: the relay might not have dealt with its end. The driver now waits for the relay to close that publisher's session, which it prints after everything else the end of the session made it do. A page that ends its Track keeps its session, so the page is closed for it.
- **"The subscriber still holds the first publisher's Object" could not fail**: the driver asked for the first Object of the Group, which stays the first whatever comes after. It now counts, from what the subscriber itself printed when its subscription had ended, the Objects of each Group, and requires one, with the payload of the publisher it has to be from.
- **The step after every scenario relied on the relay refusing a second PUBLISH for a track**, which it has not done since the relay change: with a source left over from the scenario, the fresh publisher would have been taken as one more source and the Object would have passed. The step now has a second subscriber that stays and has to see PUBLISH_DONE when the fresh publisher leaves, which a relay with a source left over cannot send. The fresh publisher sends a Group far from those of the scenarios, so that such a relay does not stop the Object for another reason first.

The driver also refuses options that mean nothing where they are given: the options of these two scenarios in another one, a moqtail client as the second publisher, a moqtail subscriber in `second-source`, and `--second-resends` with another `--first` than `done`.

### A PUBLISH while the relay waits for another publisher's answer

Added on 2026-10-11, on `main` at `eff75aaa`, for a change to the relay that was merged in between (a PUBLISH from another session than the one the relay has asked is accepted and answers the subscribers). Scenario `publish-while-asked` of `disconnect.ts`; the expectations were written from the notes before the run and not changed after it.

| Requirement | Row of the notes | Checked by |
|---|---|---|
| Subscribers wait, without SUBSCRIBE_OK, while the publisher the relay asked has not answered | "Subscribers while the publisher has not answered the relay's SUBSCRIBE"; §9.4 | no SUBSCRIBE_OK line towards a subscriber when the relay has opened its request towards the publisher that announced the namespace |
| A PUBLISH from another session gives the track its source, and the subscribers that waited get SUBSCRIBE_OK | "PUBLISH for a track the relay knows already" | the subscribers are answered after the PUBLISH, and the publisher that was asked has not answered then (it answers 3 seconds late; the run fails if it had) |
| What the relay asked stays asked, and the answer makes that publisher a further source | the same row; "The publisher's answer to the relay's SUBSCRIBE" | Group 0 arrives with the payload of the publisher that sent PUBLISH, Group 1 with that of the one that was asked; one Object each at every subscriber; the subscriptions go on when the one that was asked leaves, and end with PUBLISH_DONE `TRACK_ENDED`, Stream Count 2, when both have left |

Run: two own-rust publishers, an own-rust and an own-browser subscriber. A spike of the relay change had the same flow with Rust clients only.

| Run | Result |
|---|---|
| The scenario, once, as first written | passed |
| `mise run check` | passed, 392 TypeScript tests |
| `mise run interop` | 56 of 56 pairings in 34.9 seconds |
| The 24 leaving scenarios, 20 rounds | 20 of 20 rounds, 17.5 to 26.6 seconds a round; this scenario took 3.8 to 4.7 seconds and runs beside the ones that wait. A killed own-rust publisher was noticed after 9.0 seconds in 20 of 20, a killed subscriber after 6.0 to 6.1 |
| Temporary change to the relay core, taken back: a PUBLISH no longer answers the subscribers that waited | the scenario fails: `timed out waiting for sub1:own-rust to print /^SUBSCRIBED /` |

Not run: a PUBLISH from the same session that was asked, where the relay's SUBSCRIBE gives way; the relay's tests cover it.

### A publisher that publishes its track anew on the same session

Added on 2026-10-11, on `main` at `07322d4c`, for a change to the relay merged in between (a PUBLISH from a session that has said PUBLISH_DONE for the track is accepted while its first subscription drains). Scenario `publish-anew` of `disconnect.ts`, without and with `--same-alias`. The expectations were written from the notes' row "PUBLISH for a track the relay knows already" before the runs and not changed after.

The own Rust client sends a Group, then PUBLISH_DONE with Stream Count 2^62 - 1 and a PUBLISH for the same track at once, then another Group. WHY that Stream Count: every stream of this client has ended when its PUBLISH_DONE goes out, so with the exact number the relay would be done with the first subscription before the PUBLISH; the value of a publisher that cannot count keeps the relay waiting, which is the state the row is about.

| Requirement, from the row | Checked by |
|---|---|
| "its PUBLISH for the track is accepted, also while the relay still waits for the streams the first subscription counted" | the client prints PUBLISHED for the new PUBLISH, under Track Alias 2, or 1 with `--same-alias` |
| "the PUBLISH is accepted for the track as it is, and the subscribers stay" | no PUBLISH_DONE towards a subscriber when the new PUBLISH is established; Group 0 and Group 1 arrive within one subscription, one Object each |
| Another alias: "the first drains as it would have" | the relay ends the first PUBLISH stream by a reset, and not before its wait of 2 seconds is nearly over |
| The same alias: "the first one is over at once: the relay stops waiting for what the first counted and resets its PUBLISH stream (`CANCELLED`)" | the reset has code `0x1` and comes at once |
| The track ends with its last source | when the publisher leaves, the own subscribers see PUBLISH_DONE `TRACK_ENDED`, Stream Count 2, no stream without a FIN; the relay prints one PUBLISH_DONE a subscriber; the track is served again afterwards |

That the reset after the wait has the code `0x1` under another alias as well was not derived beforehand; it was seen, and the scenario asks for a reset there and for the code under the same alias only.

| Run | Result |
|---|---|
| Another alias, once, as first written | passed: the first PUBLISH stream ended by "reset, code 0x1" after 2.0 seconds |
| The same alias, once, as first written | passed: "reset, code 0x1" after 0.0 seconds |
| Temporary change to the client, taken back: the PUBLISH_DONE has the exact Stream Count, 1 | both fail: "the relay ended the subscription of sub1:own-rust when pub:own-rust had published the track anew". That is the other side of the race the row names: the source is over at once, the track ends, and the PUBLISH makes a new track |

| `mise run check`, `mise run interop` | passed; 66 of 66 pairings in 40.3 seconds |
| The 26 leaving scenarios, 20 rounds, first attempts | not completed as a clean run. 16 rounds passed, 17.0 to 28.4 seconds each. The other 4 each lost pairings to "stopped after the limit of 120s" after a few seconds of running, and each ended at a moment the machine came back from sleep (01:15, 01:31, 01:49 and 02:06 in the power log; it slept for about 15 minutes each time). A second attempt with `caffeinate -i` slept all the same and was stopped after 3 rounds. |
| The 26 leaving scenarios, 20 rounds, repeated on `main` at `9888d6c2` while the machine stayed awake (no sleep in the power log between 02:50 and 02:57) | **20 of 20 rounds**, 18.3 to 28.5 seconds a round. The scenario under another alias took 3.9 to 4.9 seconds, the one under the same alias 0.6 to 0.8. A killed own-rust publisher was noticed after 9.0 seconds in 20 of 20. No round lost the last Object of the publisher that closes without PUBLISH_DONE |

Subscribers were own-rust and own-browser. Not run: a stream of the first subscription that is still open when the track is published anew (the client finishes every stream it opens), and a publisher that counts from 0 again.

### Which publishers the relay asks

Added on 2026-10-11, on `main` at `3dfb27c8`, for a change to the relay merged in between (the relay sends its own SUBSCRIBE to every publisher that announced a matching namespace, where it asked the first one only). Scenario `ask-all` of `disconnect.ts` with `--ask`. The expectations were written from the notes' row "A SUBSCRIBE for a track that has no source, and which publishers the relay asks" before the runs and not changed after. Subscribers were own-rust and then own-browser in every run; what the relay asked is counted from its `OPEN_REQUEST` lines, a session at a time.

| `--ask` | Who there is | Requirement, from the row | Checked by |
|---|---|---|---|
| `two-announcers` | two own-rust publishers that announce the namespace and send no PUBLISH | "The relay sends its own SUBSCRIBE to every publisher whose announced namespace matches [...] They are asked in the order they announced" | no SUBSCRIBE before a subscriber comes; then one to each, the first announcer's first; both accept; when the second subscriber has come, still one each. Group 0 comes from the first and Group 1 from the second, once each |
| `announcer-and-publisher` | a publisher with PUBLISH, and an own-rust one that announces before the subscribers come | "The same is done for a track that has a source, each time a subscriber comes [...] the publishers of the namespace that are not a source of the track are asked, and one that accepts is one more source" | one SUBSCRIBE to the announcer when the first subscriber comes and none more for the second; none to the publisher with PUBLISH, which announced the namespace as well; a Group of each arrives |
| `late-announcer` | the same, and the announcer comes after the subscribers | "When a PUBLISH_NAMESPACE arrives, the relay answers it and then does the same for every track in that namespace that has a subscriber" | no SUBSCRIBE while only the publisher with PUBLISH is there; one to the announcer when it has announced, without a subscriber coming; it accepts, and its Group arrives |
| `refusing-announcer` | an own-browser page that announces and sends no PUBLISH, and an own-rust publisher with PUBLISH | "Not asked are [...] one that was asked and did not become a source: it refused [...] A publisher is asked again once it announces a namespace of the track anew" | one SUBSCRIBE to the page when the first subscriber comes; still one when the second has come; a second one when the page has announced the same namespace again. The subscribers stay and get the publisher's Group |

In the first three, both publishers then leave one after the other: the subscriptions go on when the first has left, and end with PUBLISH_DONE `TRACK_ENDED`, Stream Count 2, when the second has, and the relay serves the track again.

**An own-browser publisher cannot answer the relay's SUBSCRIBE.** `@xmoq/moqt` answers every request the peer opens with REQUEST_ERROR NOT_SUPPORTED (`0x3`); its README lists that as not implemented. A page that announces a namespace without PUBLISH therefore refuses, and `refusing-announcer` uses exactly that: the page's log has the relay's SUBSCRIBE arrive and REQUEST_ERROR `0x3` go back, twice. A browser publisher that is to be asked for tracks needs that part of the library first. The own Rust client is the one client here that only announces and serves; the copies of the moqtail clients send PUBLISH (read in `interop/peers/`, not run).

| Run | Result |
|---|---|
| `two-announcers`; `announcer-and-publisher` and `late-announcer`, each with an own-rust and with an own-browser publisher with PUBLISH; `refusing-announcer` | all six passed at the first run, as first written |
| `mise run check`, `mise run interop` | passed; 70 of 70 pairings in 45.7 seconds. The matrix has four of the six: `two-announcers`, `announcer-and-publisher` with own-browser, `late-announcer` with own-rust, `refusing-announcer` |
| The 30 leaving scenarios, 20 rounds, 03:24 to 03:32 with no sleep in the power log | 20 of 20 rounds, 22.4 to 33.9 seconds a round |
| Temporary change to the relay core, taken back: only the first publisher that can be asked is asked | `two-announcers` fails: `timed out waiting for announcer2:own-rust to print /^SERVING /` |
| Temporary change, taken back: publishers that refused are not left out | `refusing-announcer` fails: "the relay had sent announcer:own-browser (session 0) 2 SUBSCRIBE(s) when every subscriber had come, not 1" |
| Temporary change, taken back: the sources of the track are not left out of those to ask | **`announcer-and-publisher` and `late-announcer` still pass.** The publisher with PUBLISH was not sent a SUBSCRIBE all the same: the relay's session with it refuses a second subscription to the track, a second guard (`ask` in `crates/relay-core/src/routing.rs`). So the check "none to the publisher with PUBLISH" was not seen to fail |

Not run: a publisher that refuses with another code or does not answer in 5 seconds, more publishers than a track has room for, a namespace that matches as a prefix only, and a refusal that ends a track without a source.

### Two failures in CI, and what they were

Added on 2026-10-11. The interop job of the pull request that added `ask-all` ended with 68 of 70 (run 38076391019). Neither failure was the relay's.

**1. `publish-anew` under the same Track Alias: the PUBLISH was read before the PUBLISH_DONE.** The relay answered the new PUBLISH with REQUEST_ERROR `0x19` (DUPLICATE_SUBSCRIPTION) and printed `DRAIN_TIMER` for the first subscription right after: it read the PUBLISH, on its stream, before the PUBLISH_DONE, on another. QUIC keeps no order between streams, and until the PUBLISH_DONE is read the first subscription is Established, so the refusal is what draft 18 §5.1 asks for ("If an endpoint receives a message attempting to establish a second subscription to a Track with the same role, it MUST fail that request with a DUPLICATE_SUBSCRIPTION error"). The check is in `crates/moqt/src/session/peer.rs` and does not look at the Track Alias, so the pairing under another alias can fail the same way. The implementation notes have this as the third outcome of a PUBLISH that follows a PUBLISH_DONE at once ("And there is a third outcome", row "PUBLISH for a track the relay knows already").

| | |
|---|---|
| Here, eight runs side by side, the two pairings in turn, as they were | 1 of 287 runs, under the same alias, with the same order of lines as in CI |
| The scenario as it is now | The client's step is two lines on its stdin: `publish-done`, then `publish-anew` or `publish-anew-same-alias`. Between them the driver waits for the relay's `DRAIN_TIMER session=N track_alias=1`, which the relay prints when it has read the PUBLISH_DONE and waits for the streams it counted. The state the scenario is about is then reached by what is observed, and not by which stream is read first |
| The same load, as it is now | **640 of 640 runs passed**, 320 of each pairing (04:17 to 04:21, no sleep in the power log) |
| What is still a matter of time | The relay waits 2 seconds from that line, and the PUBLISH has to be read within them. A run in which it is not fails with "the track was over when the relay read the new PUBLISH ... The run did not reach the state it is about" (seen by holding the PUBLISH back for 2.3 seconds, a temporary change) |
| A refusal all the same | The run fails at once with the client's own line: "pub:own-rust ended before it had published the track anew: PUBLISH rejected: error_code=0x19" (seen with a client changed, for one run, to send no PUBLISH_DONE and a driver that did not wait) |

**For a publisher outside these runs the race stays.** The driver can order the two because it reads the relay's log; a publisher cannot, and all it has is what the notes say there.

**2. `ask-all`, `late-announcer`: the driver counted the relay's lines too early.** "the relay had sent announcer:own-rust (session 3) 0 SUBSCRIBE(s) after the announcer came, not 1", while the relay's log of that run has the `OPEN_REQUEST` two lines later. The driver waited for the publisher's `SERVING` and then counted the relay's lines; the lines of two processes reach the driver in no order. Now it waits for the relay's `OPEN_REQUEST` towards that session as well, which is certain to come once the publisher serves, in all four ways. What the relay did not ask is counted once more after the line of the last publisher's session ending, which comes after every line of what went before; no time is waited for it.

**A fault of the driver that made the first failure look like a time limit.** Two waits added with these scenarios (`nextMatch` in `clients.ts`, `relayPrinted` in `disconnect.ts`) went on looking after their time was up: the run printed `FAILED` after 20 seconds and did not end, and the matrix stopped it at its limit of 120 seconds. Both now use `pollFor` in `harness.ts`, which stops at its limit. With a line that never comes, a run now ends 20 seconds in, with its own message.

### A Subgroup that comes again with another Publisher Priority

Added on 2026-10-11, on `main` at `55c0d370`, for a change to the relay merged in between (a Subgroup that comes again with another Publisher Priority ends the track as malformed). Scenario `subgroup-priorities` of `disconnect.ts` with `--priorities <first>,<second>`. The expectations were written from the notes' rows "A Subgroup that comes again with another Publisher Priority (relay)" and "A malformed track that has several sources" before the runs and not changed after. Subscribers were own-rust and own-browser.

In every run the first stream of the Group is delivered to the subscribers before the second is sent, so which one "came first" and "is taken as given" is not a race. The first stream has ended with a FIN by then.

| Requirement, from the rows | Checked by |
|---|---|
| "The stream that brought the other priority is stopped with `MALFORMED_TRACK`" | one `STOP_DATA` line for the session whose stream came second, code `0x12` |
| "every subscription ends with PUBLISH_DONE `MALFORMED_TRACK`" | the own subscribers see PUBLISH_DONE `0x12`, Stream Count 1, and have the Group once, with the payload of the stream that came first |
| "every source is cancelled: the one that sent what broke the object model with `MALFORMED_TRACK`, the others with `CANCELLED`" | the relay's lines for the request streams of each publisher's session carry `0x12` for the one whose stream came second and `0x1` for the other; an own-rust publisher ends with the same code in its last line |
| "one publisher that sends a Subgroup again with another priority ends its own track" | the same with one own-browser publisher, which sends Group 0 with priority 0 and again with 1 |
| "A Subgroup that two sources send with the same priority, one in the header and one by its default, is no difference" | the second stream is stopped with `0x1`, as any second stream of a Subgroup; no PUBLISH_DONE then; the track ends with `TRACK_ENDED` when both publishers have left |

| Run | Result |
|---|---|
| 0 and 1: own-rust and own-rust; own-browser and own-rust; own-rust and own-browser; own-browser alone | all four passed at the first run, as first written |
| 128 and left out: own-rust and own-rust; left out and 128: own-rust and own-browser | both passed at the first run. The headers on the wire: own-rust sent type `0x78` (the DEFAULT_PRIORITY bit, no priority byte), own-browser `5c 01 00 00 80` |
| `mise run check`, `mise run interop` | passed; 74 of 74 pairings in 62.7 seconds, after the correction below (73 of 74 before it). The matrix has four: 0 and 1 from two own-rust publishers, from own-browser and own-rust, and from one own-browser publisher; left out and 128 from own-rust and own-browser |
| The 34 leaving scenarios, 20 rounds | 20 of 20 rounds, 23.9 to 39.2 seconds a round (04:34 to 04:43, no sleep in the power log) |
| Temporary change to the relay core, taken back: a difference is never found | the runs with 0 and 1 fail: the relay does not stop the second stream at all (`timed out waiting for the relay to print /^STOP_DATA .../`) |
| Temporary change, taken back: a header without a priority counts as 0 and not as 128 | the run with 128 and left out fails: "the relay stopped the second stream of the Subgroup, from pub2:own-rust, with [0x12], not once with 0x1" |

- **The first form of the check on the own-rust publisher failed in the first run of the whole matrix**, and was the scenario's fault: it waited for the client's `STOP_SENDING_RECEIVED` line. The relay cancels a PUBLISH by resetting its direction of the request stream and stopping the publisher's, with the same code, and which of the two the client sees first differs from run to run: that run had the reset first and no STOP_SENDING line, where the six single runs before all had the STOP_SENDING. The check reads the code from the client's last line now (`Error: the subscriber ended the subscription: reset, code 0x12`, or `STOP_SENDING, code 0x12`). The client prints that line when its process ends, which is when its stdin does, so the driver closes the stdin once the relay has seen the client's session end. In 8 runs after the change the reset came first in 5 and the STOP_SENDING in 3, and all passed.
- **The own page reports the cancel as a reset**: its library's event is `request-closed`, reason `peer-reset`, reset code 18 (`0x12`). The scenario reads the relay's lines for a page and the client's own line for own-rust.
- **`@xmoq/moqt` can set the priority of a stream and can leave it out**, which the page of these runs now passes on; it could send priority 0 only before.
- Not run: a Subgroup that comes again while its first stream is still open (the clients finish every stream they send in this scenario); a DEFAULT PUBLISHER PRIORITY among the Track Properties (no client here sends one); a publisher that publishes anew with another default; a header that leaves the Subgroup ID to the first Object.

### A page whose write lost against the relay's STOP_SENDING

Added on 2026-10-11, on `main` at `cdd8245b` with two changes on top that touch neither the scenario nor a stream this endpoint writes. The run "own-rust leaves the Publisher Priority out and own-browser writes 128" failed once in another working copy: `[driver] FAILED evaluate: Error: NetworkError: The stream was aborted by the remote server`. The relay had done what the scenario expects (`STOP_DATA session=1 stream=3 code=0x1`); the page's call to send failed.

The relay decides on the Subgroup Header, so its STOP_SENDING can reach the page before the page has written the Object. What the write is rejected with then depends on how far the stop has got, seen in Chromium 156:

| Where the stop is when the page writes | The write is rejected with | Seen |
|---|---|---|
| `@xmoq/moqt` has taken it in | `SubgroupClosedError`, or `PublishEndedError` when the PUBLISH was cancelled with it | with the write held back 400 ms (temporary change to the page, taken back) |
| The browser has it, the library not yet | `WebTransportError`, `source` "stream", `streamErrorCode` 1, "Received STOP_SENDING." | with the write held back and the library's watch for a stop taken out (temporary change to `stream-io.ts`, taken back) |
| The data pipe of the stream closed before the browser reported the stop | `DOMException` named NetworkError, "The stream was aborted by the remote server", no code | the failed runs. `OutgoingStream::HandlePipeClosed` in Chromium's `third_party/blink/renderer/modules/webtransport/outgoing_stream.cc`, tip of the repository, read on 2026-10-11 |

The four `subgroup-priorities` runs of the matrix, each with an own-rust and an own-browser subscriber, six in parallel on six ports as load:

| State of the scenario | Rounds of each run | Failed |
|---|---|---|
| Before | 150 | left out and 128, own-rust then own-browser: 3, with the NetworkError. The other three: 0 |
| The page takes a write that was stopped for sent | 300 | the same run: 4, later, at the end of the track: "PUBLISH_DONE must not be sent while subgroup streams are open (§10.11)". The others: 0 |
| And the page resets a stream whose write failed | 300 | 0 in all four |

- **The correction is to the page and the scenario.** For the stream the relay is expected to stop, the page reports a write that was stopped as an outcome and not as a failure, as the own Rust client does (`STOP_SENDING_RECEIVED`, then "sent"). The scenario still requires one `STOP_DATA` line for that session with the expected code.
- **The second failure shows a gap in `@xmoq/moqt`, which is not mended here.** It learns that a stream was stopped from `writer.closed`, and takes only a `WebTransportError` with `source` "stream" for a stop (so that the end of the session is not taken for one). When Chromium rejects with the NetworkError instead, the library goes on counting the stream as open: `write` passes the browser's error on, and PUBLISH_DONE is refused until the application resets the stream. The page now resets a stream whose write failed (§11.4.3: "The publisher SHOULD respond with a reset"), which is what an application has to do today.
- The runs with 0 and 1 where a page sends the second stream have the same race and did not fail in 150 rounds; held back, their write is rejected with `PublishEndedError`, which the page takes the same way. Where own-rust sends the second stream nothing changes: the client has taken a stopped stream for sent since the relay change.
- How often the NetworkError came in the last 300 rounds was not counted (the logs of rounds that pass are not kept). At 3 of 150 and 4 of 300 before, about five were to be expected.
- `mise run check` and `mise run interop` passed after it: 74 of 74 pairings in 45.5 seconds.

## Limitations and blockers

- **The relay change was under review when the runs were made.** The last runs are on `a44c8854`; if it changes again, they have to be made again.
- The first three rows of "The checks fail when what they check is broken" were made on `a2fc1e9d` and not again later.
- **The path of the own Rust client that the relay change mended is not run here**: a data stream that is stopped while the client is still writing it. With these Objects the client had written the stream out before the stop arrived, in every run.
- A moqtail client whose stream the relay stops is not run (above), so what such a publisher does is known from another session's runs only.
- Whether the own Rust client prints `STOP_SENDING_RECEIVED` for a data stream the relay stopped is not judged: with these small Objects it had written the stream out before the stop arrived, and printed nothing.
- The moqtail clients send Group 0 only, so they can be a second source of the same Group and not of a next one. The moqtail browser page was not used as a publisher here.
- The two sources of a Group send the same payload, as the relay assumes; what it would do with different payloads is a known gap of the notes and is not run.
- Chromium only, loopback only.

## Open questions raised

- None that is open. The runs with two publishers stay in `disconnect.ts`; they would get a driver of their own when more are added.
