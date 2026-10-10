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
