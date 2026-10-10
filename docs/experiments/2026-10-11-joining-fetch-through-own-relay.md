# A Joining FETCH through the own relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

The own relay now answers a Joining FETCH: it works out the range from the Joining Location of the subscription the FETCH names, and asks the publisher for it with a Standalone FETCH. See that the range ends where the subscription begins, with a publisher written by someone else.

## Environment

As in [the record of the Standalone FETCH](2026-10-11-standalone-fetch-through-own-relay.md): the own relay built from the working copy this record is part of, moqtail's test client 0.15.0 as the publisher, `spikes/fetch-capture` as the subscriber, loopback, port 48600.

## Commands

The relay and the publisher as in that record (12 Groups of 3 Objects, 250 ms apart). Four seconds after the publisher started, each against a relay and a publisher started anew: the SUBSCRIBE of `subscribe-moqtail-rs` on one request stream, and 300 ms later a Joining FETCH on another.

```sh
S="$(cat fixtures/moqt-18/subscribe-moqtail-rs.hex)"
$C --request "$S" --request "$(cat fixtures/moqt-18/fetch-relative-joining-moqtail-rs.hex)"   # 1: Relative, Joining Start 0
$C --request "$S" --request "16 00 07 02 02 00 01 01 20 c8"                                   # 2: Relative, Joining Start 1
$C --request "$S" --request "16 00 07 02 03 00 02 01 20 c8"                                   # 3: Absolute, Joining Start 2

python3 spikes/relay-core-diff/compare.py 0b1da296 disk --world <world> --random-only
```

The requests of 2 and 3 were written by hand; a test of `crates/moqt` decodes them to what was meant.

## Observed results

**In each run the fetch stream ended with the Object at the Joining Location, and the subscription began with the Group after it.** The three runs were made twice, before and after the relay's FETCH was changed for its reviews, with the same results; the table is of the second time.

| Run | SUBSCRIBE_OK | FETCH_OK | Objects on the fetch stream | First subgroup streams of the subscription |
|---|---|---|---|---|
| 1 | LARGEST_OBJECT {5, 2} | End Location {5, 3} | {5, 0}, {5, 1}, {5, 2} | Groups 6, 7, 8 |
| 2 | {5, 2} | {5, 3} | Groups 4 and 5, three Objects each | Groups 6, 7, 8 |
| 3 | {5, 2} | {5, 3} | Groups 2 to 5, three Objects each | Groups 6, 7, 8 |

The relay log of each run: `OPEN_REQUEST session=0 own=0 FETCH`, `RECV session=0 own=0 FETCH_OK`, `SEND session=1 stream=2 FETCH_OK`, `PIPE_FETCH session=1 stream=2 request_id=2 from_session=0 from_stream=9`, `FETCH_PIPE_ENDED session=1 stream=2 finished`. The FETCH_HEADER the subscriber got has its own Request ID, 2.

The moqtail relay, asked the same in [the record of the captures](2026-10-11-moqtail-relay-fetch.md) (a), answered run 2's request with End Location {6, 1} and sent {6, 0} on both the fetch stream and the subscription.

**The relay core treats what has no FETCH as the revision before did.** `spikes/relay-core-diff`, `0b1da296` (the relay with a Standalone FETCH alone) against the working copy, 20,000 random sequences of 80 events in each of five worlds (`full`, `one-publisher`, `split-one`, `shared`, `no-announce`): 0 sequences in which the versions differ, 0 bystanders treated differently in the two worlds that count them, 0 events of a kind the old core does not know.

## Limitations and blockers

No blocker.

- **Two runs each, on one machine.** That the two did not overlap is what these runs showed, not what the relay guarantees: an Object at or before the Joining Location that reaches the relay after the SUBSCRIBE_OK goes out on the subscription as well (implementation notes, known gaps). The publisher here sends its Groups in order on one connection.
- A FETCH for a SUBSCRIBE that is not answered yet, a subscription with Forward State 0, one without a LARGEST_OBJECT, a subscription that ends while its FETCH is served, and a Joining Start beyond Group 0 were not run against a real peer; they are tests of the relay core.
- The FETCH was sent 300 ms after the SUBSCRIBE. A FETCH that the relay reads before its SUBSCRIBE is refused (known gaps); that was not provoked.
- `moqlivemock`'s viewer and publisher, the moqtail npm package and a browser were not run through the own relay here.
- The comparison of the relay core says nothing about a FETCH: no world of the tool has one.
