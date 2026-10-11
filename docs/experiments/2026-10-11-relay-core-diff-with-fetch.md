# The tool that compares two relay cores gets worlds with FETCH

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

`spikes/relay-core-diff/` runs two versions of the relay core through the same random events and reports where they differ. Its events had no FETCH in them, so a change to how the relay answers a FETCH showed nothing: PR #73 changed what the relay does with a message that follows a FETCH it has answered to its end, and the tool, run over that change, found no difference and could not say which sequences differ. Give the tool events about FETCH, have it count the differences by kind instead of stopping at the first, and run it over that step of PR #73 to see whether the differences are the ones the change was meant to make.

## Environment

The working copy this record is part of, macOS. The two versions compared in runs 3 to 5 are commits of the change that became PR #73, before it was squashed: `98aeb613` (before the step) and `6bfae37b` (after it). The step is the one that tells the session when the relay has ended its direction of the request stream of an answered FETCH (`Session::on_fetch_answer_ended`).

## Commands

```sh
T="python3 spikes/relay-core-diff/compare.py"
# 1: the same core twice, in the three new worlds (2,000 sequences each)
XMOQ_FUZZ_RUNS=2000 $T @- disk --world fetch --kinds          # also split-fetch, shared-fetch
# 2: one decision about a FETCH changed in the new core (20,000 sequences)
$T @- disk --world fetch --kinds --touch fetch --mutate-fetch  # also split-fetch, full
# 3 to 5: over the step of PR #73 (20,000 sequences each)
$T 98aeb613 6bfae37b --world fetch --kinds --touch fetch
$T 98aeb613 6bfae37b --world split-fetch --kinds --touch fetch
$T 98aeb613 6bfae37b --world shared-fetch --kinds --touch fetch
# 6: the same step in a world without FETCH
$T 98aeb613 6bfae37b --world full --kinds
```

`--kinds` lets the sequences run to their end and counts, for every sequence in which the two versions differ, the first difference under its kind: the kind of the event with the message it brings, and the kinds of the actions each version answered with.

## Observed results

**Run 1: the same core twice differs nowhere, and the sequences get to FETCH.** 0 sequences differ in each world. What the relay did in 2,000 sequences of 80 events:

| World | FETCH requests sent to a publisher | Fetch streams passed on | Fetch streams written by the relay | Joining FETCH requests held for a SUBSCRIBE to come | FETCH_OK sent | Sequences with a stream passed on |
|---|---|---|---|---|---|---|
| `fetch` | 2,116 | 479 | 565 | 782 | 975 | 419 |
| `split-fetch` | 1,457 | 335 | 1,208 | 737 | 1,509 | 297 |
| `shared-fetch` | 1,457 | 335 | 1,208 | 737 | 1,509 | 297 |

Before the first track of a sequence was given a Largest Object, the relay wrote 20 streams in 2,000 sequences of `fetch`: it answers a FETCH with FILL_TIMEOUT 0 itself only for a track it knows a largest Location of, and few sequences got an Object through first.

**Run 2: a changed decision about a FETCH is found in the worlds with FETCH, and not in `full`.** The new core refused a FETCH whose publisher did not answer in time with `DOES_NOT_EXIST` (16) where it refuses it with `TIMEOUT` (2).

| World | Sequences that differ | First difference |
|---|---|---|
| `fetch` | 2,360 of 20,000 | all at `UpstreamTimeout`: `SendRequest(RequestError 2)` before, `SendRequest(RequestError 16)` after |
| `split-fetch` | 1,588 | the same; the bystanders (sessions 3 and 4) were treated alike in all 304,966 actions for them |
| `full` | 0 | none: no sequence has a FETCH |

**Runs 3 to 5: over the step of PR #73, every sequence that differs first differs at a message on the stream of a FETCH whose answer the relay has ended.**

| First difference: the event, what the old core did, what the new one does | `fetch` | `split-fetch` | `shared-fetch` |
|---|---|---|---|
| REQUEST_UPDATE with no parameter: `SendRequest(RequestOk)`, then nothing | 107 | 154 | 154 |
| REQUEST_UPDATE with SUBSCRIBER_PRIORITY: `SendRequest(RequestOk)`, then nothing | 44 | 77 | 77 |
| REQUEST_UPDATE with FORWARD: `CloseSession(3)`, then nothing | 54 | 68 | 68 |
| **PUBLISH_DONE: `CloseSession(3)`, then nothing** | 63 | 75 | 75 |
| REQUEST_UPDATE, of any of the three, whose Request ID a held Joining FETCH names: as above, and `SendRequest(RequestError 50)` with `FinishRequest` after it in both versions, or in the new one alone where the old one closed the session | 1 | 3 | 3 |
| Sequences that differ, of 20,000 | 269 | 377 | 377 |
| Bystanders treated differently | - | 0 of 316,877 actions | 0 of 274,414 actions |

- The first three rows are what the change was reported to do: a REQUEST_UPDATE the relay can no longer answer gets no REQUEST_OK, and one with a parameter it may not carry no longer closes the session.
- **The fourth row was not in the report of the change.** A message that is no REQUEST_UPDATE on such a stream closed the session before (`PROTOCOL_VIOLATION`, "message not allowed in this request state") and is ignored now, as on the stream of a request that was refused. The implementation notes have it ("The same holds for a FETCH that was answered"); the report named only the REQUEST_UPDATE. A subscriber never has a reason to send PUBLISH_DONE on the stream of its FETCH, before or after the answer.
- The fifth row is the first three again, in sequences where the REQUEST_UPDATE also uses the Request ID that a Joining FETCH is held for: that FETCH is refused with `INVALID_JOINING_REQUEST_ID` (50) in both versions, unless the old one closed the session for the FORWARD. Seen in the full list of the first differences of `split-fetch` (`SendRequest(RequestOk) SendRequest(RequestError 50) FinishRequest` before, `SendRequest(RequestError 50) FinishRequest` after).
- In no sequence did the versions differ before an event about a FETCH (`differ before such an event: 0`, with `--touch fetch`).

**Run 6: in `full`, 0 of 20,000 sequences differ.**

## Limitations and blockers

- **"The versions differ in these ways" means: in every sequence that differs, they first differ in one of them.** Two versions that have parted hold different things, and what follows differs for that reason alone; the table says nothing about it. The tool prints this under the table.
- The two versions of runs 3 to 6 are commits that are no longer on a branch: the change was squashed into PR #73. Over the whole of PR #73 (`6ad5240c` to `3be3f4fa`) the tool cannot compare: that change also removed an action the old core returns, and the two cores do not build together.
- The numbering of the relay's own requests needed no translation between the versions: a fetch stream names the Request ID the relay gave its FETCH, and the two versions give the same one as long as they sent the same requests to that publisher. Once they have not, the sequence has differed already. In the worlds with two halves the publisher of the second half gets requests for that half only.
- In `split-fetch` and `shared-fetch` only sessions 1 and 2 send a FETCH. Session 4 of `shared-fetch`, which subscribes to the tracks of both halves, sends none: a FETCH of its own that closes its session in one version and not in the other would be the change itself, and would count as a bystander treated differently.
- That the comparison of the bystanders can fail in the new worlds was not checked (the tool has `--raw-numbers` for that in `split`); the changed decision of run 2 is of the first half and leaves them alone, as it should.
- Half of the Standalone FETCH requests are for the whole of Group 2. That is the last Group the streams of these worlds have, and not always the last one the relay has seen in a sequence.
- A fetch stream the generator opens is not followed by Objects: the relay does not look at what it passes on, and the events have none.
- `--same` (one version three times, for the order of its actions) was not run with the new events.
- Run 1 has 2,000 sequences a world, the others 20,000.
