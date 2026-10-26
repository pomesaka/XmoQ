# moqtail's relay and the Joining FETCH of a page's catalog

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings (`interop/meeting-pages.ts`), and every run that reads a page's MSF catalog through `moqtail`'s relay.

## Objective

Through `moqtail`'s relay 0.15.0 a page gets no catalog of another page: the run of a [meeting of pages](2026-10-25-a-meeting-of-pages.md) failed for it, as the terminal viewer had in the [GOAWAY runs](2026-10-24-goaway-and-moqtail.md). Is it the relay that asks for the wrong range, or the page that answers wrongly?

This is a reading of draft 18 and of the relay's code beside the logs of that run, and no new run.

## Expected

Written before the code was read: the Joining FETCH is answered from the Joining Location of the subscription (§10.12.2.1), which the relay told the subscriber in its SUBSCRIBE_OK. If the relay counts from anything else, the error is the relay's.

## What the draft has

- §5.1 (line 1950 of the text): "A publisher MUST save the Largest Location communicated in SUBSCRIBE_OK ... This value is called the Joining Location and can be used in a Joining FETCH".
- §10.12.2.1: "The publisher receiving a Joining Fetch sets the End Location to {Joining Location.Group, Joining Location.Object + 1}", and for a Relative Joining Fetch "the Start Location to {Joining Location.Group - Joining Start, 0}".
- The relay is the publisher of the subscriber's subscription, and the Joining Location is the LARGEST_OBJECT of the SUBSCRIBE_OK it sent.

## What happened in the run

From the log of `moqtail relay: meeting: ...`, 2026-10-25, page p1 reading the catalog of p2:

1. The relay forwarded p1's SUBSCRIBE for `xmoq/meet/r/p2` `catalog`. p2 answered SUBSCRIBE_OK with LARGEST_OBJECT {1792907901367, 0}: it numbers its Groups by its clock, and its one catalog Object was sent before anybody subscribed. The relay sent p1 a SUBSCRIBE_OK with the same LARGEST_OBJECT.
2. p1 sent a Relative Joining FETCH with Joining Start 0. By §10.12.2.1 that is {1792907901367, 0} to {1792907901367, 1}.
3. The relay answered p1 FETCH_OK with End Location {0, 1}, asked p2 for {0, 0} to {0, 0}, got FETCH_OK and an empty stream (p2 has no Group 0), and sent p1 an empty stream. No catalog came, and none came live, since the catalog was not published anew.

## What the relay's code does

Read with `gh api repos/moqtail/moqtail/contents/...?ref=relay@0.15.0` on 2026-10-25:

- `apps/relay/src/server/message_handlers/fetch_handler.rs`, lines 286 to 311: for a Joining FETCH it reads `track.largest_location` and makes Start {that Group - Joining Start, 0} and End {that Group, that Object + 1}. The subscription is looked up (line 253) only for its Forward State.
- `apps/relay/src/server/track.rs`: `largest_location` starts at {0, 0} (line 271) and moves only when an Object (lines 596 to 602) or a datagram (lines 683 to 689) comes through the relay. `largest_object()` says None until one has (lines 381 to 389); the Joining FETCH does not use it.
- `apps/relay/src/server/message_handlers/parameters.rs`, `downstream_subscribe_ok`: the SUBSCRIBE_OK sent down carries the upstream LARGEST_OBJECT where the relay has seen none, and `subscribe_handler.rs` (line 833) calls it with `largest_object()`.

## Result

**The relay's**, as expected. It tells the subscriber one Joining Location and counts the FETCH from another, its own {0, 0}, which §5.1 and §10.12.2.1 do not allow. The page answered what it was asked: §10.12.1 has an End Location {0, 0} as all of Group 0, and the page has none. Numbering Groups by the clock is the page's to choose; the draft puts no start on Group IDs.

The own relay answers the same FETCH from the Joining Location (row on a Joining FETCH in the relay's table of the [implementation notes](../moqt-implementation-notes.md)), and the same meeting runs through it.

## What is not done

- Nothing is changed in the pages. Two ways around it were looked at and left. A page could number its Groups from 0, which would match only while its catalog is in Group 0. Or it could publish the catalog anew for every subscription, which works around the relay with Objects nobody else needs. Either one fits the pages to one relay's mistake.
- Not reported to `moqtail`: it is outside this project's operations here.
- A Joining FETCH through `moqtail`'s relay after an Object has passed through it, where the relay's Largest Location is right, was not run for a catalog.
