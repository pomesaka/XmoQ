# A viewer changes the delivery timeout of its subscription with REQUEST_UPDATE, and the relay follows

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link that is narrow or loses packets. [The record of a timeout that follows the sound](2026-10-12-a-timeout-and-a-buffer-that-follow-the-sound.md) found that changing a timeout cost a subscription made anew, because the relay refused every REQUEST_UPDATE for a subscription.

## Objective

That a subscriber can change the SUBGROUP_DELIVERY_TIMEOUT of a subscription it has, without ending it, and that the own relay acts on the new time. In both implementations, and with a peer's relay for the bytes that are sent.

## What the draft says (draft-ietf-moq-transport-18)

| Question | The text |
|---|---|
| Who sends it, and where | §10.9: "The sender of a request (SUBSCRIBE, PUBLISH, FETCH, PUBLISH_NAMESPACE, SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS) can later send a REQUEST_UPDATE on the same bidi stream as the request to modify it. A subscriber can also send REQUEST_UPDATE to modify parameters of a subscription established with PUBLISH." §5.1: only once the subscription is Established |
| The answer | §10.9: "MUST respond with exactly one REQUEST_OK or REQUEST_ERROR". No Request ID is in either, so the answers are told apart by their order. §10.5: Track Properties are empty in REQUEST_UPDATE_OK |
| Its Request ID | §10.1: a REQUEST_UPDATE "consumes a Request ID" of its sender |
| What is left out | §10.9: "its value remains unchanged. There is no mechanism to remove a parameter from a request" |
| What it may carry for a subscription | AUTHORIZATION_TOKEN (§10.2.2), SUBGROUP_DELIVERY_TIMEOUT and OBJECT_DELIVERY_TIMEOUT (§10.2.3, §10.2.4), SUBSCRIBER_PRIORITY (§10.2.7), SUBSCRIPTION_FILTER (§10.2.9), FORWARD (§10.2.12), NEW_GROUP_REQUEST (§10.2.13) |
| When it fails | §10.9.1: "the publisher MUST also terminate the subscription by sending a PUBLISH_DONE with error code UPDATE_FAILED". Several updates may be answered by one REQUEST_ERROR |
| A relay and its upstream | §10.2.3: the timeout "SHOULD NOT be forwarded upstream by a relay that intends to serve multiple subscriptions for the same track". FORWARD and NEW_GROUP_REQUEST are to go upstream (§9.5, §10.2.13) |
| With no parameter | §13.6.1 names "REQUEST_UPDATE with no modified Message Parameters" as a way to keep a session open |

**Where it is silent**:

- **From when a changed delivery timeout holds.** §8 starts a timer "once it becomes aware that all of the objects on the subgroup have been published" and has nothing of a timeout that changes while a timer runs, or of streams that are open. For a filter §10.9.1 has "it might still receive Objects outside the new range if the publisher sent them before the update was processed".
- **What a publisher does with a parameter it cannot apply**: fail the update, which ends the subscription, or accept and leave it.

## What the peers do

- `moqtail`'s relay (tag `relay@0.15.0`, read with `gh api` on 2026-10-12: `apps/relay/src/server/message_handlers/subscribe_handler.rs`, `subscription.rs`): takes a REQUEST_UPDATE for a subscription, writes its parameters over the ones it has (`apply_message_parameter_update`), acts on the filter, the Forward State and the priority, and answers REQUEST_OK. Nothing there reads a delivery timeout.
- `moqtail`'s TypeScript library: the npm package 0.13.0 has `RequestUpdate` and serializes it. In the source at the same tag (`libs/moqtail-ts/src/client/handler/request_update.ts`), a publisher applies an update of a subscription (`SubscribePublication.update`) and answers REQUEST_OK. No client of `moqtail` in `interop/peers/` sends one.
- `moqlivemock`: a code search of its repository for `RequestUpdate` finds nothing (GitHub searches the default branch, not the tag v0.16.1); its library `Eyevinn/moqtransport` v0.14.0 was not read.

## What was built

- **The rule, in both sessions**: a REQUEST_UPDATE for an Established subscription this endpoint publishes is answered with REQUEST_OK when it carries nothing but delivery timeouts (SUBGROUP_DELIVERY_TIMEOUT, and OBJECT_DELIVERY_TIMEOUT, which nobody acts on, as in a SUBSCRIBE). With any other parameter it is refused as before (NOT_SUPPORTED, then PUBLISH_DONE UPDATE_FAILED).
- **The relay** takes the new time for the subscription, and for every stream towards the subscriber that it learns the end of after the update; a stream whose timer runs keeps its time (**provisional**; [the implementation notes](../moqt-implementation-notes.md) have the row). 0 takes the timeout away. `--delivery-patience-ms` holds as before, the smaller of the two.
- **Sending**: `Subscription.update` of `@xmoq/moqt`, `setDeliveryTimeouts` of the playback of `playVideoTrack`, `Session::update_subscription` of `xmoq-moqt`, `--update-subgroup-timeout-ms` of the Rust client. At most 16 updates of a subscription wait for their answer.
- **Fixtures**: `fixtures/moqt-18-from-libraries/` has two REQUEST_UPDATEs as `moqtail`'s npm package serializes them (a timeout of 250 ms, and of 0); both implementations decode them and encode the same bytes, and the page's own update is those bytes.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag; `spikes/lossy-link/proxy.mjs` between viewer and relay at 2,000 kbit/s, 50 ms each way, queue 100 ms; the busy picture at 2.5 Mbit/s; buffer 150 ms.

## Result

### The relay follows a timeout that changes

`/watch?videoTimeout=A&videoTimeoutThen=B&videoTimeoutAfter=15`: the page subscribes with a timeout of A ms and asks for B with a REQUEST_UPDATE 15 s later. Counted: the 10 s from the fifth second ("before"), and the 10 s from two seconds after the update ("after"); 300 frames are made in each. "Reset" is the relay's lines `DELIVERY_TIMED_OUT` in those 10 s; the picture has one Group, and one stream, a second.

Expected (not written down before the first run, which was a trial of the script): with no timeout no stream is reset and few frames are shown; with 150 ms about one stream a second is reset and about 210 frames are shown, as in [the record of the timeout](2026-10-12-which-delivery-timeout-a-viewer-asks-for-on-its-video.md); each half is as its timeout, whichever came first.

| First, then | Run | Before: frames shown, streams reset | After: frames shown, streams reset | The relay's answer |
|---|---|---|---|---|
| none, then 150 ms | 1 | 62, 0 | 140, 10 | REQUEST_OK |
| | 2 | 61, 0 | 143, 10 | REQUEST_OK |
| | 3 | 56, 0 | 143, 10 | REQUEST_OK |
| 150 ms, then none | 1 | 211, 10 | 75, 0 | REQUEST_OK |
| | 2 | 208, 10 | 80, 0 | REQUEST_OK |
| | 3 | 213, 9 | 80, 0 | REQUEST_OK |
| 150 ms, then 150 ms | 1 | 220, 10 | 209, 9 | REQUEST_OK |
| | 2 | 214, 10 | 210, 9 | REQUEST_OK |

- **As expected for the streams**: none is reset while no timeout is asked, and one a second from the update on; the other way round, none from the update on. The subscription is the same throughout (one session, no SUBSCRIBE anew).
- **Not as expected: 140 frames, not 210, in the 10 s after a timeout is first asked.** The viewer is then about 680 ms behind, and comes nearer over those seconds (the picture came 430 ms after it was made at their end, where the runs that had the timeout from the start are at 365). A longer count would say whether it gets to 210; that was not run.
- An update to the time that is there changes nothing that was counted.

### Both implementations send it, and two relays take it

| Sender | Relay | What happened |
|---|---|---|
| The page (`setDeliveryTimeouts`), while it plays a video of 50 frames | own | REQUEST_OK; all 50 frames played |
| The page | `moqtail` 0.15.0 | REQUEST_OK; all 50 frames played |
| The Rust client (`--update-subgroup-timeout-ms 5000`) | own | `UPDATE_ACCEPTED`; the Object came after it |
| The Rust client | `moqtail` 0.15.0 | `UPDATE_ACCEPTED`; the Object came after it |

The four are pairings of `mise run interop` from here on. **That a relay acts on the new time is not in them**: nothing is late on loopback. The own relay's tests have it with a subscriber that takes nothing (a table of seven rows: the timeout of the SUBSCRIBE, that of the update, and whether the update came before or after the end of the stream).

`mise run check` and `mise run interop` passed (the numbers are in the pull request).

## What this says, and what it does not

- A viewer can change the timeout on its video without subscribing anew, and the relay follows from the next Group that ends.
- Not built: a page that changes the timeout by itself. The rule of the record before (the sound's lateness) showed no gain worth a pause at each change; without the pause it could be looked at again.
- Not known: how long a viewer that asks for a timeout late takes to catch up; any of it on a real network.
- Not done: updates of a filter, the Forward State, the priority and NEW_GROUP_REQUEST (refused, and the subscription ends); acting on OBJECT_DELIVERY_TIMEOUT; updates of a FETCH, of a PUBLISH_NAMESPACE, and of a subscription a PUBLISH made, from a subscriber of ours.
- **Not run**: an update that gets no answer. Both clients give the answer the time a request gets and then leave the subscription as it is; on loopback every answer came within the 1 and 2 ms that were tried, so the path was not seen with a real peer.
