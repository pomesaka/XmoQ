# A cancelled subscription, and what the relay had not sent yet

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay

## Objective

A page that changed from one version of a video to another found the version it had left still coming down its link, and the new one late behind it. Whether the relay, when a subscription is cancelled, takes back what it has not delivered of it.

## What was found

It did not. The relay resets the streams of a cancelled subscription that are not done (§11.4.3: "If a sender closes the stream before delivering all such objects to the QUIC stream, it MUST reset the stream. This includes ... Early termination of subscription due to request cancellation"). But a stream it had written whole and finished was done for it at that moment, and gone from its books, while up to 4 MiB of such streams wait in the send buffer of QUIC for a slow subscriber. No reset reached those.

## What was changed

The writer of a subgroup stream keeps the stream after its FIN until the subscriber's transport has acknowledged all of it; a reset that comes meanwhile resets the QUIC stream, which drops what was not delivered. The codes are the ones the relay used before (`CANCELLED` for a cancelled request); none was chosen anew.

- A stream that is written whole is now "closed" for the relay when the subscriber has all of it, not when its FIN was handed over.
- What a subscriber may have waiting is counted as before for bytes and Objects (what is not written). **A stream that is written whole and not acknowledged still counts as one of the subscriber's streams** (256 for a session): a subscriber that takes streams and acknowledges nothing reaches that limit and loses the subscription that hit it, as one that is too slow. Before, such a stream stopped counting at its FIN.
- The connection in memory that the tests use holds nothing between a write and its peer, so a finished stream is a delivered one there. What the writer does with a stream that is not delivered is tested on a connection that delivers when the test says so; the reset itself on the real connection.

## Commands

The Rust client has `--stay-after-ms N` now: `--mode subscribe` keeps the session open that long after it has cancelled its subscription.

A publisher of the 2.5 Mbit/s video, the relay, `spikes/lossy-link/proxy.mjs` at 1,000 kbit/s with a queue of 100 ms and 50 ms each way, and behind it a viewer that takes 150 frames, cancels and stays 12 seconds (`--objects 150 --stay-after-ms 12000`). The proxy's log says what went down the link, by the millisecond.

## Observed results

One run each, one machine.

| | Before | After |
|---|---|---|
| Bytes down the link after the cancel, in 12 s | 1,511,532 | 10,635 |
| In each of those seconds | 119,064 to 132,132: the link was full to the end | 10,485 in the first, then 36 every third second |
| The last datagram of more than 200 bytes | 12,051 ms after the cancel, when the viewer left | 74 ms after |

- `mise run interop`: 123 of 123 pairings, three times with the change (once more it was 121, with two pairings stopped at their limit of 120 seconds right after other measurements on the machine; the two runs after that were 123).
- The lateness of a viewer behind a link of 2 Mbit/s is what it was (3,933 ms with no rule, 555 ms with `--delivery-patience-ms 500`, 331 ms with `--older-groups give-up`).

## Not measured

- A page as the viewer, and the change between two versions of a video that showed this.
- A subscriber that does not acknowledge at all: the streams stay with their writers until the session ends.

## Next action

None from this record.
