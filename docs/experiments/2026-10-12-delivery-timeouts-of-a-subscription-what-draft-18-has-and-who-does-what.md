# Delivery timeouts of a subscription: what draft 18 has, who does what, and the viewer's side made ready

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer whose link has less room than the video. The relay's experiments (`--delivery-patience-ms`, `--older-groups give-up`) hold for every track of every viewer, and what suits video breaks sound ([record](2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md)). Nothing is decided here.

## Objective

Does draft 18 have a way for a viewer to say, for one subscription, how long delivery may take, so that video and sound can have different rules without a flag of the relay? What does it say, what do the peers do with it, and can a page send it today?

## What the draft says

`draft-ietf-moq-transport-18`, read in full for "delivery timeout": §8, §10.2.3, §10.2.4, §10.9, §10.11, §11.4.3, §12.1, §12.2, §3.3.3.

- **Two timeouts for a subscription**, §8: "Each MOQT subscription has two timeout values associated with it: a SUBGROUP_DELIVERY_TIMEOUT and an OBJECT_DELIVERY_TIMEOUT. Both of those values are expressed in milliseconds; both are optional; a value of 0 means that there is no timeout set."
- **Who says them, and which holds**, §8: "The publisher communicates both timeout values as a Track Property; the subscriber communicates them as Message Parameters. For each type of timeout, if both the publisher and the subscriber have a non-zero value, the smaller of the two is used."
- **Where a subscriber may put them**, §10.2.3 and §10.2.4 (Parameter Types 0x06 and 0x02, each a varint): "It MAY appear in a PUBLISH_OK, SUBSCRIBE, or REQUEST_UPDATE message." Not in a FETCH. In another message it is a PROTOCOL_VIOLATION (§10.2.1), and so is one given twice (§10.2, "Senders MUST NOT repeat the same Parameter Type").
- **A relay does not pass them on**: "This parameter is intended to be specific to a subscription, so it SHOULD NOT be forwarded upstream by a relay that intends to serve multiple subscriptions for the same track."
- **The subgroup's timeout runs from the end of the Subgroup**, §8: the sender "MUST start a timer of SUBGROUP_DELIVERY_TIMEOUT duration once it becomes aware that all of the objects on the subgroup have been published (either by receiving a FIN from the upstream subscription, or, in case of the original publisher, through being notified of this fact by the application). If the timer expires before the underlying transport stream reaches "all data committed" state ..., the implementation MUST reset the stream."
- **The object's timeout runs from the Object's first byte and is looked at when it is handed to the transport**, §8: the sender "MUST check the time elapsed since the first byte of the object before attempting to pass it to the underlying transport for transmission; if the time elapsed exceeds OBJECT_DELIVERY_TIMEOUT, it MUST reset the underlying transport stream with the reset stream code DELIVERY_TIMEOUT ... and SHOULD NOT attempt to open a new stream to deliver additional Objects in that Subgroup." And: "any data buffered at this layer can no longer be timed out".
- **The code of the reset**: DELIVERY_TIMEOUT (0x2), §3.3.3; §11.4.3 has "Either of the delivery timeouts" among what makes a sender reset a stream it has not delivered whole.
- **It can be changed**, §10.9: a REQUEST_UPDATE carries parameters; "If a parameter previously set on the request is not present in REQUEST_UPDATE, its value remains unchanged. There is no mechanism to remove a parameter from a request." Setting it to 0 is "no timeout".
- **A publisher may give up sooner on its own**, §8: "Publishers can, at their discretion, discontinue forwarding Objects before either of the timeouts occurs, subject to stream closure and ordering constraints described in Section 11.4.3. However, if none of the timeouts are set to a non-zero value, all Objects in the track matching the subscription filter are delivered".
- The subscription goes on, §10.11: "some objects in the subscribed track might never be delivered, because a stream was reset, or never opened in the first place, due to the delivery timeouts".

Where the draft is silent, or leaves a choice:

- **Nothing says "give a Group up when the next one begins".** Both timeouts are lengths of time, from a Subgroup's end or an Object's first byte. The relay's `--delivery-patience-ms` is the subgroup's timeout with the relay's own number; `--older-groups give-up` is the publisher's discretion of §8, and no parameter names it. A parameter of one's own is not allowed without a Setup Option that negotiates it: "An endpoint that receives an unknown Message Parameter MUST close the session" (§10.2).
- What a relay's own number is against the two that are named: §8 has the smaller of the publisher's Track Property and the subscriber's parameter; a relay is a publisher to its subscriber, but its flag is no Track Property.
- Whether a timeout that is changed holds for streams that are already open.
- The subscriber is not told which timeout is in force, nor that the sender acts on one at all; it sees the publisher's Track Property, where one is sent.
- The object's timeout only bites where the sender holds Objects back before the transport. The own relay hands an Object to a send window of 4 MiB at once ([record](2026-10-12-the-relay-gives-up-a-group-a-viewer-does-not-get-in-time.md)), so it would not.

## Who does what

Read in the code that is installed for the interop run.

| | Sends it | Reads it | Acts on it |
|---|---|---|---|
| `moqtail` crate 0.15.0 | Has both parameters and both Track Properties; the clients of the interop run send neither parameter | Yes; allowed in PUBLISH_OK, REQUEST_OK, SUBSCRIBE and REQUEST_UPDATE | No use of the values was found in the library |
| moqtail relay 0.15.0 | Not forwarded upstream (seen: the bytes are in no SUBSCRIBE it sent) | Yes: it logged `ObjectDeliveryTimeout { timeout: 10000 }, SubgroupDeliveryTimeout ...` for the page's SUBSCRIBE | **No.** It resets a stream that does not finish in time with DELIVERY_TIMEOUT, by a number of its own configuration, the same for every subscriber |
| moqtail npm 0.13.0 | Has both classes; the client of the interop run sends neither | Yes | Not looked for |
| `moqtransport` 0.14.0 (moqlivemock) | Knows both types as varints | Yes | No use of the values was found outside its wire code |
| `@xmoq/moqt` | **Could already**: `subscribe({ parameters: { subgroupDeliveryTimeout, objectDeliveryTimeout } })`, and in PUBLISH_OK and REQUEST_UPDATE | Yes, with the scopes of the draft | Not as a publisher |
| own relay | Not forwarded upstream (seen) | Yes | **No**: ignored as a parameter and as a Track Property (implementation notes, known gaps) |

## What was done

- **The encoding was checked against a peer for the first time.** moqtail npm 0.13.0 was made to serialize a SUBSCRIBE with both timeouts (`Subscribe.newLatestObject(0n, name, [new ObjectDeliveryTimeout(250n), new SubgroupDeliveryTimeout(500n)])`): `03 00 1f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 03 02 80 fa 04 81 f4 1b 01 02`. `@xmoq/moqt` decodes it to those values and encodes the same bytes (`packages/moqt/test/core/delivery-timeouts.test.ts`). The bytes are in the test and not in `fixtures/moqt-18/`: no client sent them over a wire, and every file there is a capture that the Rust tests read as well.
- **`@xmoq/media`**: `playVideoTrack` and `playAudioTrack` take `deliveryTimeouts: { subgroupMs, objectMs }` and send them with their SUBSCRIBE (`deliveryTimeoutParameters`, with table tests). Nothing is sent without it.
- **The demo**: `/watch?videoTimeout=N` and `?audioTimeout=N` send SUBGROUP_DELIVERY_TIMEOUT for the one track; neither without the flag.
- **The interop run**, two pairings: a viewer page sends both timeouts (10,000 ms) with its SUBSCRIBE, through the own relay and through the moqtail relay, and plays the 50 frames as without. On the wire: `... 02 02 a7 10 04 a7 10`, two parameters, 0x02 and 0x06, each 10,000. Neither relay refused it, and neither sent it on to the publisher.

`mise run check` passed on the second run; on the first, one Rust test failed that this change does not touch (`contract::with_a_send_buffer::finish_until_resets_a_stream_that_is_given_up_before_it_is_delivered`, `Ok(())` where `Err(Reset(Some(2)))` was expected), and it passed in the four runs after. `mise run interop`: 125 of 125.

## To decide before the relay acts on it

For whoever builds the relay's side; nothing here is decided.

- **Which number holds where the relay has a flag and the subscriber sent a timeout.** §8 has the smaller of publisher and subscriber. The flag could be the relay's stand-in for a publisher's Track Property (then the smaller), a default for subscribers that say nothing, or a limit a subscriber cannot go over.
- **Whether a subscriber that says nothing gets nothing given up** (§8: "if none of the timeouts are set to a non-zero value, all Objects ... are delivered"), which is what the relay does today without a flag.
- **What "give older Groups up" becomes.** The draft's timeouts cannot say it. It can stay the relay's discretion with a flag; it could be applied only to subscriptions that sent a subgroup timeout (the viewer's sign that it wants late Subgroups dropped), with that timeout as the grace; or it could be left for video and never applied to a track whose Groups are shorter than the grace. For the last two the relay has to know nothing about codecs.
- **The Track Property of the publisher** (§12.1, §12.2): whether the relay reads it and takes the smaller, and whether the demo's publisher sends one (none today).
- **REQUEST_UPDATE**: whether a change holds for streams that are open, and whether the relay accepts it at all for a subscription (it answers one for a FETCH today).
- **OBJECT_DELIVERY_TIMEOUT**: whether to act on it, given that the relay's send window takes every Object at once; or to answer it by acting on the subgroup's alone.
- **Upstream**: the draft has "SHOULD NOT be forwarded upstream by a relay that intends to serve multiple subscriptions for the same track", and says nothing of a relay with one; neither relay here forwards it, and it is simplest never to.
- **What the viewer should ask for.** With a Group of video a stream, a subgroup timeout is the relay's patience by another name; the measurements have 500 ms for it, and say that "at the next Group, with 100 ms of grace" is better, which this parameter cannot ask for. For sound: none, or longer than a round trip.
- A shared fixture for the SUBSCRIBE above, read by both implementations, once the Rust side has a test for it.
