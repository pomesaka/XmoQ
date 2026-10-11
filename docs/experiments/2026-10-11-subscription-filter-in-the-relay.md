# The relay keeps to the Start Location of a Subscription Filter

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

The relay sent every subscriber whatever reached it after the SUBSCRIBE, whatever the Subscription Filter of that SUBSCRIBE said (§5.1.2: "A publisher MUST NOT send objects from outside the requested range"). It now sends a subscriber the Objects from the Start Location of its filter on. Run the four filters and an unfiltered subscription against Objects that reach the relay late, with a Location before the Largest Object, on the relay as it was and as it is; see that a Joining FETCH ends where a subscription with the Largest Object filter starts; and run the two versions of the relay core through random events, to see that they differ in nothing but what a filtered subscriber is sent.

## Environment

The own relay and the Rust client built from the working copy this record is part of, on both ends. Loopback, port 48600, macOS. "The relay as it was" is the same build with one line changed so that the filter of a SUBSCRIBE is not read; the line was put back after the two runs.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"

# P1, at 1 s: Groups 5 to 8 from 3 s on, and Groups 9 and 10 at about 11 s.
$C --mode publish --payload-hex 6162 --first-group 5 --answer-fetch
# The subscriber, at 5.5 s: the Largest Object is {8, 0}.
$C --mode subscribe --objects 9 --timeout-secs 11 --subscription-filter <filter>   # 1 to 5
$C --mode subscribe --objects 9 --timeout-secs 11 --join relative:1                # 6
# P2, at 7 s: Groups 0 to 2 from 8.5 s on, which are before the Largest Object.
$C --mode publish --payload-hex 6162

# The two cores, 20,000 sequences of 80 random events a world:
T="python3 spikes/relay-core-diff/compare.py"
$T @- disk --world filters --kinds --touch filters     # also split-filters, shared-filters
$T @- disk --world full --kinds --touch filters        # also fetch
```

`--subscription-filter` is new: `none`, `largest-object` (what the client sends without the flag), `next-group-start`, `absolute-start:<group>,<object>`.

## Observed results

**The subscriber got the Groups its filter passes.** It was told no LARGEST_OBJECT it prints, so the Start Location here is worked out from the Largest Object {8, 0} the relay had.

| Run | Filter | Start Location | Groups on the subscription | As it should be |
|---|---|---|---|---|
| 1 | Largest Object | {8, 1} | 9, 10 | yes |
| 2 | none | {0, 0} | 1, 2, 9, 10 | yes: Group 0 is one the relay passes on to nobody (below) |
| 3 | Next Group Start | {9, 0} | 9, 10 | yes |
| 4 | `AbsoluteStart` {1, 0} | {1, 0} | 1, 2, 9, 10 | yes |
| 5 | `AbsoluteStart` {9, 0} | {9, 0} | 9, 10 | yes |

**Group 0 of the second publisher reached no subscriber, in any run, on either relay, and that is not the filter.** The relay's log has `STOP_DATA session=2 stream=3 code=0x1` for it. The first publisher had sent Group 8, and on a track that has had two sources the relay stops a stream of a Group that is 8 or more behind the newest (implementation notes, row on how long the relay remembers); Groups 1 and 2 are 7 and 6 behind and pass. Run 2 again with the first publisher starting at Group 3 (`--first-group 3`: Groups 3 to 6, then 7 and 8), so that Group 0 is 6 behind: the subscriber got the Groups 0, 1, 2, 7 and 8, and the log has no `STOP_DATA`. One run. An unfiltered subscription does not get everything, then; the notes name the two exceptions.

**On the relay as it was, the Largest Object filter kept nothing out.**

| Relay | Filter | Groups on the subscription |
|---|---|---|
| as it was | Largest Object | 1, 2, 9, 10 |
| as it is | Largest Object | 9, 10 |

**Run 6: a Joining FETCH ended where the subscription started.** The subscriber's Joining FETCH (Relative, Joining Start 1) got `FETCH_OK ... end_group=8 end_object=1` and the Objects of Groups 7 and 8; the publisher printed `FETCH_ASKED ... start_group=7 start_object=0 end_group=8 end_object=1`. The subscription brought Groups 9 and 10. On the relay as it was, the same subscriber got Groups 7 and 8 on the fetch stream and Groups 1, 2, 9 and 10 on the subscription.

**The two cores differ only in what a subscriber with a filter is sent.** "Before" is the commit below the working copy.

| World | SUBSCRIBEs with a filter (Largest Object, Next Group Start, `AbsoluteStart`, `AbsoluteRange`) | Sequences that differ, of 20,000 | Bystanders treated differently |
|---|---|---|---|
| `filters` | 36,206; 12,159; 12,011; 12,159 | 863 | - |
| `split-filters` | 18,263; 6,119; 6,063; 6,029 | 461 | 0 of 331,776 actions |
| `shared-filters` | 18,263; 6,119; 6,063; 6,029 | 464 | 0 of 295,447 actions |
| `full` | none | 0 | - |
| `fetch` | none | 0 | - |

In every sequence that differs, the first difference is that the new core opens fewer streams to subscribers or sends fewer Objects, and does everything else as the old one did: at `DataStreamOpened` (1,516 sequences over the three worlds), at `Object` (209), at a PUBLISH that brings streams the relay held for it (44) and at a SUBSCRIBE_OK of a publisher that does (12). In 7 of them the old core also reset or finished, in the same step, a stream the new one did not open. In no sequence do the versions differ before a SUBSCRIBE with a filter.

## Limitations and blockers

- **One pairing of `mise run interop` failed with this change as its scenario was, and the scenario changed with it**: "own-browser fetches Objects the own page has and never sent; the relay asks the page". The page answered the relay's SUBSCRIBE with LARGEST_OBJECT {102, 0} and then sent Groups 0 to 2; the Rust client that takes them subscribes with the Largest Object filter, whose Start Location is then {102, 1}, and the relay sent it nothing, as §5.1.2 has it. The other 101 pairings ended as expected. The page now goes on with Group 103 ([record](2026-10-11-a-page-answers-a-fetch.md)), and with that all 102 pairings end as expected on this relay; one run.
- **An Object that both a Joining FETCH and its subscription deliver was not produced, before or after.** The relay learns of an Object when it comes on a subscription, and passes on a second copy of a Subgroup from no publisher; a publisher that holds an Object back and sends it after it was fetched is none the Rust client can be. What the runs show is the cause the notes named: an Object before the Largest Object that reaches the relay after the SUBSCRIBE went out on the subscription, and does not any more.
- "The first difference" is all the comparison of the cores says of a sequence; see [the record of that tool](2026-10-11-relay-core-diff-with-fetch.md).
- A subscription that starts inside a Subgroup, a SUBSCRIBE that waited for a publisher's answer, and the Start Location where a Group or an Object ID has none after it are tests of the relay core and were not run against a peer: the Rust publisher sends one Object a Group.
- `AbsoluteRange` was not run against a peer: the Rust client has no flag for it. The relay keeps to its Start Location and does not look at its End Group (implementation notes, known gaps).
- One run each.
