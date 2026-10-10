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

### Objects of the size of a video frame, in real time

Added later the same day, on `main` at `7c7a4033`, for the question of how Phase 1 puts frames on streams: `draft-ietf-moq-msf-01` §6 asks for a new stream for every Object, and the runs above had only 57-byte and 100,000-byte Objects, sent as fast as the page could.

`spikes/frame-streams/run.ts`: an own page publishes Objects of 10,000 bytes, 30 a second (it kept 30.0 a second in every run and was never a frame late), 30 to a Group, which is the shape of a one-second group of pictures at about 2.4 Mbit/s. Shape A puts every Object on a stream of its own, each a Subgroup named by its Object ID; shape B puts a Group on one stream. Three pages on `@xmoq/moqt` subscribe through the own relay: one never reads, one reads 15 Objects a second, one reads everything. After the time of the run, the page that never reads subscribes to a second track of the same publisher, which never carries an Object.

```sh
bun spikes/frame-streams/run.ts --one-per-stream --seconds 90     # twice, and twice with --seconds 180
bun spikes/frame-streams/run.ts --seconds 90                      # twice, and once with --seconds 300
```

Runs of the same kind gave the same numbers, to the tenth of a second.

| | A: one Object a stream | B: a Group a stream |
|---|---|---|
| The page that never reads: cut off by the relay (PUBLISH_DONE `TOO_FAR_BEHIND`, Stream Count 2^62 - 1) | after 60.9 s | after 171.2 s; not within 90 s |
| Streams the relay had booked towards it by then | 1828, of which it reset the 257 it could not open, with `0x5` | 172. The relay reset 30 streams in that run, for the two pages it cut off; how many for this one was not counted |
| Objects its library had taken | 78 | 78 |
| Is that page told? (`Subscription.ended`, and whether it saw PUBLISH_DONE) | **no**: `ended` had not resolved and no PUBLISH_DONE had been seen 39 s after the cut (the 90-second runs). In the 180-second runs it ended 119 s after the cut, with the session and not with PUBLISH_DONE (below) | **no**, 139 s after the cut |
| Its second SUBSCRIBE, made after the cut | **no answer in 10 s** (at 90 s). At 180 s the browser refused the stream ("Failed to create bidirectional stream."), and the library closed the session for it, with code 1 | **no answer in 10 s** (at 300 s) |
| The same second SUBSCRIBE before the cut | | answered after 2 ms (at 90 s) |
| The page that reads 15 a second: cut off | after 87.1 s | after 197.5 s; not within 90 s |
| Is that page told? | yes: it read what had arrived (1607 Objects), `objects` closed, `ended` is `publish-done` with status 5 | yes, after 4535 Objects |
| The page that reads everything | read every Object that was sent; not cut off | the same |
| Sessions the relay closed itself | none | none |

- **The page that never reads is not told that it was cut off, with Objects of the size of a frame too, and in both shapes.** The requests it makes afterwards get no answer, and two minutes later the session is lost. So what the first table of this record showed for one 100,000-byte Object a stream is not a matter of that size or of that shape. It is a matter of time: shape B reaches the same state after 171 seconds where shape A takes 61.
- **"With 30 such Objects a stream … it did not happen" above was for 100,000-byte Objects sent as fast as the page could**, where the relay cut the subscription off after 0.1 seconds and the page was told. With a Group of 30 frame-sized Objects on a stream, at 30 a second, the relay cuts it off after 171 seconds, and then it does happen.
- **A page that only reads too slowly is told.** It reads on, reaches the end of what had arrived, and gets PUBLISH_DONE. It is cut off after 87 seconds in shape A at half the rate of the Objects.
- **The relay's limit is reached much later than its own number suggests.** The relay ends a subscription at 256 streams waiting to be written, which 30 streams a second would fill in 8.5 seconds. It took 61, because the streams do not wait at the relay: Chromium takes them off the connection although the page does not read. From the relay's log, about 1570 streams (15.7 MB) had gone to the browser in shape A before 256 waited, and, if 15 of the 30 resets were this page's, about 157 streams (47 MB) in shape B. How much Chromium takes that way was not looked up; these are counts from the relay's log, not a measurement in the browser.
- The page that reads everything was not held back in any run.

**A stream that cannot be opened ends the whole session of the library.** In the 180-second runs of shape A, the page's second SUBSCRIBE failed in the browser, and the relay printed `CONNECTION_ENDED session=1 ClosedByPeer { code: 1, reason: "Failed to create bidirectional stream." }`. That is the library's doing, as read in `packages/moqt/src/webtransport/`: the session state hands the connection an output to open a request stream, `stream-io.ts` calls `createBidirectionalStream()`, and when any output fails, `enqueue` in `connection.ts` calls `fatal`, which ends everything the application holds and closes the transport with `INTERNAL_ERROR`. So one request that the browser could not start took the other subscriptions of the page with it. Three ways it could be instead, none of them tried:

| | What it would take | What it leaves open |
|---|---|---|
| As it is: the session fails | nothing | a page loses subscriptions that still worked |
| Only that request fails, and the session stays | the session state has given the request a Request ID and a stream number before the stream is opened, so it would need an input that says "this request stream was never opened", which ends the request for the application. The peer would see a Request ID that was never used, which both implementations take as no fault (the notes, "Request IDs from the peer that arrive out of order or leave gaps") | whether a browser that refuses a stream is in a state where the session is worth keeping. Here it was not: the page was already cut off from the answers to its requests |
| The request waits for the browser to let a stream through | a bound on that wait | in the 90-second runs the stream was opened and the answer did not come, so waiting would not have helped there |

What made Chromium refuse the stream after two minutes and accept it 29 seconds after the cut was not looked at.

What this means for the open question "whether a browser client should end a subscription itself when its application does not read": the constraint written there, that one large Object a stream is to be avoided, is not enough. With frames of 10,000 bytes the same state is reached in both shapes; the choice of shape moves when (61 seconds or 171) and how much the browser holds by then (about 16 MB or about 47 MB, from the relay's counts), not whether.

## Limitations and blockers

- **In the frame-sized runs every Object has 10,000 bytes.** Real frames differ in size and a key frame is larger. One track, one subscription a page, loopback, Chromium 156 alone. Whether a page that starts to read after it was cut off learns of the end, as it did in the earlier runs, was not looked at in these. Whether other work on the machine ran beside them (23:26 to 23:45) is not known; the publisher kept its rate.
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
