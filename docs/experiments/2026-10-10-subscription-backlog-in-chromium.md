# A subscription nobody reads, in `@xmoq/moqt` and in Chromium

- Date: 2026-10-10
- Commit SHA: the commit that adds `maxPeerUniStreams` to `packages/moqt` together with this record. The runs were made on its working copy on top of the commit that fixes `WebTransportError` (pull request 15)
- Roadmap item: Phase 0b, own TypeScript browser client. Follows the defect found in [the disconnect record](2026-10-10-disconnects-through-own-relay.md): a page that did not read its subscription held every Object sent to it

## Objective

Bound what `@xmoq/moqt` holds for a subscription whose application does not read, and find out what the browser and the own relay do then: whether the relay ends the subscription with `TOO_FAR_BEHIND`, as it does for a Rust client that takes nothing, and what the page is told.

### What has to hold

| # | What has to hold | Basis |
|---|---|---|
| 1 | What a subscription holds unread is bounded | Project rule: receive queues are bounded. **Met for the number of streams only**: neither Objects nor bytes are counted, see below |
| 2 | TypeScript and Rust behave alike. Rust reads 64 unidirectional streams of the peer at once and leaves further ones in the transport (`max_uni_streams` in `crates/moqt-io`), and hands Objects on through a queue of 16 | Project rule; `crates/moqt-io/src/streams.rs`, `crates/moqt-client/src/io.rs` |
| 3 | Nothing is dropped: Objects are not discarded one by one | [Architecture](../architecture.md), slow subscribers |
| 4 | A slow subscriber has its subscription ended by the publisher, `TOO_FAR_BEHIND` | §8 of draft 18 (MAY); architecture |
| 5 | When the subscription ends or the session closes while streams wait, no stream and no reader stays behind | Clean-up |
| 6 | Requests are not held up by a subscription nobody reads | PUBLISH_DONE and new requests travel on bidirectional streams. **Met only in part**: the connection goes on taking request streams, but in one shape of traffic their answers did not arrive, see below |

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Browser | Chromium 156.0.8078.4, new headless, launched by `playwright-core` 1.64.0 with `channel: "chromium"` |
| Relay | `xmoq-relay` 0.0.0 on `web-transport-quinn` 0.13.2, debug build. Its limits for one subscriber: 4 MiB, 4096 Objects, 256 streams waiting to be written (provisional) |
| Library | `packages/moqt` at the working copy: queue of 16 Objects, 64 streams of the peer read at once (both provisional defaults) |

## What was changed

The connection takes a unidirectional stream of the peer from the transport only while fewer than `maxPeerUniStreams` are being read (64 by default, the peer's control stream among them). Nothing else changed: a stream still hands over the Objects of the chunk it read and then waits for room in the queue.

Before, every stream was taken at once, and each queued its first Objects before it waited, so the queue grew with the number of streams.

What the limit bounds is the number of streams that feed a queue, not its Objects. A stream hands over every Object of the chunk it read and then waits, so the queue holds up to `subscriptionHighWaterMark` - 1 Objects plus one chunk's worth from each of `maxPeerUniStreams` - 1 streams. With one Object a stream that is 15 + 63 = 78 Objects by default. Where Objects are small and a chunk carries many, it is 63 chunks' worth, and an Object can be as large as the session's limit for one, so the bytes are bounded only by 63 times that. `maxPeerUniStreams` has to be a whole number from 2: one place is the peer's control stream.

## Commands

From the repository root, after `cargo build --locked -p xmoq-relay` and one interop run for the certificate:

```sh
mise run check
mise run interop
bun spikes/subscription-backlog/run.ts --objects-per-group 1  --object-bytes 57     --seconds 15
bun spikes/subscription-backlog/run.ts --objects-per-group 1  --object-bytes 100000 --seconds 15
bun spikes/subscription-backlog/run.ts --objects-per-group 30 --object-bytes 100000 --seconds 20
bun spikes/subscription-backlog/run.ts --objects-per-group 1  --object-bytes 100000 --seconds 15 --then-subscribe
```

The first three were also run with `--max-streams 1000000`, which is the library without the limit, and with `--then-subscribe` in both versions. Without the limit the runs were stopped after 5 seconds (`--seconds 5`), because the page's memory grows all the while. Where the text below says the page was not told for 20 seconds, the driver was made to wait that long after the relay had ended the subscription (`WAIT_MS=20000`). `run.ts` has one page publish Groups as fast as it can, each Group on one stream, and another page subscribe through the own relay and never read. It watches the relay's log for PUBLISH_DONE with status `0x5` to that subscriber.

## Observed results

### In the tests

A test sends 40 subgroup streams of one Object each to a subscription nobody reads, with a queue of 16 and 8 streams read at once. The session hands over 22 Objects: 15 that the queue takes without making their stream wait, and one from each of the 7 streams that then wait (the eighth is the control stream). With the limit switched off, 40 are handed over and the test fails. Reading afterwards delivers all 40 (row 3).

Three more tests end the subscription while 18 streams wait in the transport: by `cancel()`, by PUBLISH_DONE, and by closing the session. In the first two, the 18 are taken and a stream among them is stopped with CANCELLED; in the third, the 18 stay untaken (row 5). The tests fail with the limit switched off, and the third fails when closing does not stop the loop that takes streams. Another test shows that a stream hands over all five Objects of one chunk to a queue of two. A request made while a subscription is not read was covered by a test already (row 6).

### In Chromium, through the own relay

| Traffic | Limit | Relay ended the subscription | Objects sent by then | Objects in the queue | What the page was told |
|---|---|---|---|---|---|
| 1 Object of 57 bytes a stream | 64 | after 3.6 to 3.8 s | 889 to 932 | 78 | `publish-done`, status 5, when its wait for late streams was over |
| The same | none | not in 15 s | 2119 | 2119 | nothing |
| 1 Object of 100,000 bytes a stream | 64 | after 0.4 to 0.5 s | 377 to 467 (36 to 45 MiB) | 78 | **nothing in 20 s**. `publish-done`, status 5, once it read |
| The same | none | not in 15 s | 10,599 (1011 MiB) | 10,586 | nothing |
| 30 Objects of 100,000 bytes a stream | 64 | after 0.1 s | 98 to 272 (9 to 26 MiB) | 18 to 19 | `publish-done`, status 5 |
| The same | none | after 0.2 s | 272 (26 MiB) | 20 | `publish-done`, status 5 |

78 is 15 plus 63: the queue's 15 and one Object from each stream read at once, without the control stream. The publisher was never held up: it was still sending at the end of every run.

**With the limit, the relay ended the subscription in every shape of traffic** (rows 1 and 4). Without it, only the long streams got it ended, because a stream waits for the queue between its own Objects in either version.

**Short streams are held back as well.** The expectation, from [the disconnect record](2026-10-10-disconnects-through-own-relay.md), was that Chromium takes short streams off the connection whether or not the page reads. It did so there because the library took every stream. A stream the page has not taken from `incomingUnidirectionalStreams` stayed with the browser, the relay ran out of streams it could open, and 256 of them waited: `TOO_FAR_BEHIND`.

**The memory of the browser** (resident size of all its processes) grew by 39 to 46 MiB with 57-byte Objects, by 144 to 184 MiB with one 100,000-byte Object a stream, and by 53 to 105 MiB with long streams, until the relay ended the subscription. Without the limit it grew by 650 MiB in 15 seconds with the 100,000-byte Objects and had not stopped. The numbers are from single runs and include both pages.

**In one shape the page is not told that its subscription ended.** With one Object of 100,000 bytes a stream, `Subscription.ended` had not resolved 20 seconds after the relay sent PUBLISH_DONE, in three of three runs. When the page then started to read, it read 314 Objects, `objects` closed, and `ended` resolved with `publish-done`, status 5, 5.0 seconds later, the library's wait for late streams. A reading that fits and was not verified with a capture: about 300 streams of 100,000 bytes that the page has not taken hold the connection's flow-control credit, so the bytes of PUBLISH_DONE on the request stream cannot be sent until the page takes those streams. With 57-byte Objects and with long streams the page was told.

So a page that stops reading and never reads again can stay unaware that the relay has dropped its subscription. The relay has let go of it by then, and the browser holds what was in flight.

**In the same shape, another request of the session gets no answer.** After the relay had ended the unread subscription, the page subscribed to a second track that the publisher offers and never writes to:

| Traffic on the unread subscription | `subscribe` to the second track |
|---|---|
| 1 Object of 57 bytes a stream | answered after 2 ms, in two of two runs |
| 30 Objects of 100,000 bytes a stream | answered after 1 ms, in two of two runs |
| 1 Object of 100,000 bytes a stream | **no answer in 10 s**, in three of three runs. The relay's log has the SUBSCRIBE_OK it sent |

The same without the limit, where the relay does not end the first subscription in the first two shapes, so the second request was made after 5 seconds of traffic, while the first subscription was still served:

| Traffic on the unread subscription | Objects the library had taken | `subscribe` to the second track |
|---|---|---|
| 1 Object of 57 bytes a stream | 1059, 1169 | answered after 1 ms, in two of two runs |
| 1 Object of 100,000 bytes a stream | 3301, 3599 (316, 344 MiB) | answered after 1 ms and 39 ms, in two of two runs |
| 30 Objects of 100,000 bytes a stream | 18 (the relay ended it after 0.1 s) | answered after 1 ms, in two of two runs |

§11.4.2 of the draft names a deadlock of this kind: "endpoints MUST allocate connection flow control to the control streams before allocating it to any data streams. Otherwise, a receiver might wait for a control message [...] to release flow control, while the sender waits for flow control to send the message." The draft speaks of a control message that carries a Track Alias; an answer on a request stream that does not arrive is the same pattern applied more widely. A page cannot say how the browser gives out its credit. Whether the relay could keep credit for its request streams was not looked at.

**Without the limit it did not happen** in the same shape of traffic (table above): the library read every stream into memory, at the cost shown in the first table. When this was first reported it was an inference from that; the runs without the limit were made afterwards. The Rust implementation has a related `FIXME:` at `read_uni_streams` in `crates/moqt-io/src/streams.rs`, about a server that bounds its receive window; the library has one at `uniStreamSlot` in `stream-io.ts`.

## Limitations and blockers

- **One browser, one machine, loopback, single runs** for most rows. The times and counts vary between runs as shown.
- **Why PUBLISH_DONE and SUBSCRIBE_OK did not arrive is an inference**; no packet was captured, and the size of Chromium's flow-control window was not looked up. That the relay sent them is in its log.
- **With the limit, the second subscription was made after the relay had ended the first.** Whether it also fails while the first is still being served was not run. Without the limit it was made while the first was served, in two of the three shapes, so the two tables do not compare the same moment.
- **A subscription nobody reads holds up the streams of every other subscription of the session**, once 64 streams wait on it: the limit counts streams of the session, not of a subscription. The Rust implementation has the same property. Not run here.
- **The peer's control stream uses one of the 64**, and so does a stream that is slow to say what it is. A peer that opens 64 such streams before its control stream is not served; nothing here tries it.
- Bytes and Objects are not counted, as in Rust (above).
- A stream holds one of the 64 places until it has ended: a subgroup that stays open for a long time, and a stream of no subscription that the peer does not close.
- `spikes/subscription-backlog/` is outside lint and review.

## Open questions raised

- Can the relay alone prevent the deadlock? To be verified: when the second request was made, the relay had sent and finished the streams of the first subscription, so it may have had nothing left to reset, and the credit comes back only when the page reads those streams or stops them. If so, the client has to stop the unread streams, and the relay could at most hold back what it sends before the peer's window is used up, for which it would have to know that window.
- Should the client end the subscription itself when its application does not read? It is the one way to tell an application that never reads, in the shape of traffic where PUBLISH_DONE does not arrive. Recorded in [open questions](../open-questions.md).
- Should the interop matrix get a run with a page that does not read, now that the relay ends its subscription? It was left out of the disconnect runs because a page could not be made slow.
