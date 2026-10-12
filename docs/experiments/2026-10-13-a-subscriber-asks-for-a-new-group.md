# A subscriber asks for a new Group

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer (the relay's handling of NEW_GROUP_REQUEST)

## Objective

The own relay ignored NEW_GROUP_REQUEST, which §10.2.13 has a relay pass on (MUST). A viewer that comes in the middle of a Group waits for the next one to begin, or fetches the beginning of this one with a Joining FETCH, as the viewers of this repository do. This is the first of two steps: the request travels from the subscriber through the relay to the publisher and breaks nothing. **No publisher begins a Group for it yet**, and no viewer of this repository sends it; that is the second step, and what a viewer gains by it is not measured here.

## What the draft says

§10.2.13, and §12.6 for the Track Property.

- Where it may be: "It MAY appear in PUBLISH_OK, SUBSCRIBE or REQUEST_UPDATE for a subscription."
- Its value: "It represents the largest Group ID in the Track known by the subscriber, plus 1. A value of 0 indicates that the subscriber has no Group information for the Track."
- What the subscriber may assume: "A subscriber MUST NOT send this parameter in PUBLISH_OK or REQUEST_UPDATE if the Track did not include the DYNAMIC_GROUPS Property with value 1. A subscriber MAY include this parameter in SUBSCRIBE without foreknowledge of support. If the original publisher does not support dynamic Groups, it ignores the parameter in that case."
- The Original Publisher: "When an Original Publisher that supports dynamic Groups receives a NEW_GROUP_REQUEST with a value of 0 or a value larger than the current Group, it SHOULD end the current Group and begin a new Group as soon as practical. The Original Publisher MAY delay the NEW_GROUP_REQUEST subject to implementation specific concerns, for example, achieving a minimum duration for each Group. The Original Publisher chooses the next Group ID; there are no requirements that it be equal to the NEW_GROUP_REQUEST parameter value."
- The relay, without an upstream subscription: "A relay that receives a NEW_GROUP_REQUEST for a Track without an Established subscription MUST include the NEW_GROUP_REQUEST when subscribing upstream."
- The relay, with one: "A relay that receives a NEW_GROUP_REQUEST for an Established subscription with a value of 0 or a value larger than the Largest Group MUST send a REQUEST_UPDATE including the NEW_GROUP_REQUEST to the publisher unless: 1. The Track does not support dynamic Groups 2. There is already an outstanding NEW_GROUP_REQUEST from this Relay with a greater or equal value". And: "If a relay receives a NEW_GROUP_REQUEST with a non-zero value less than or equal to the Largest Group, it does not send a NEW_GROUP_REQUEST upstream." And: "After sending a NEW_GROUP_REQUEST upstream, the request is considered outstanding until the Largest Group increases."
- The property: "DYNAMIC_GROUPS (Property Type 0x30) is a Track Property. The allowed values are 0 or 1. ... If an endpoint receives a value larger than 1, it MUST close the session with PROTOCOL_VIOLATION. If omitted, the value is 0."
- **What it does not say**: what a publisher does with the parameter in a REQUEST_UPDATE for a track without the property; which publisher a relay asks where a track has several; whether the SUBSCRIBE a relay answers from a track it already has is "for an Established subscription". How these are read is in the implementation notes, row on NEW_GROUP_REQUEST.

## What the peers do

`moqtail` at `relay@0.15.0`, read on 2026-10-13 in the checkout `cargo install` made for that tag (commit dd8623d; no tag was on the checkout to confirm it by): the libraries, Rust and TypeScript, encode the parameter and the property. The relay and the test client (`apps/`) neither send the parameter nor read it; the only matches for "new group" there are the Group Order parameter and the relay's own adaptive bitrate code. Not run against it for that reason. `moqlivemock` was not looked at.

## What was built

- Both sessions, as a publisher: a REQUEST_UPDATE that carries NEW_GROUP_REQUEST (alone, or with delivery timeouts) is answered with REQUEST_OK and its value handed on; before, it was refused and the subscription ended with UPDATE_FAILED. In `@xmoq/moqt`: `PublishedTrack.onNewGroupRequest`.
- The Rust session: `Session::update_published` sends a REQUEST_UPDATE for a subscription a PUBLISH of the peer made, and its answer on that stream is read.
- The relay: what the draft has for it, towards sources that came with a PUBLISH, and in the SUBSCRIBE it sends upstream. A source the relay itself subscribed to is not sent a REQUEST_UPDATE (known shortcoming).
- The Rust client: `--dynamic-groups yes` puts the property on its PUBLISH and prints `NEW_GROUP_REQUESTED group=N` for a request; `--new-group-request N` puts the parameter into its SUBSCRIBE.

## Runs

Expected before the runs: the relay sends the publisher one REQUEST_UPDATE on the stream of its PUBLISH; the publisher reports the value 0; the Object sent afterwards arrives; no session is closed.

| Run | Result |
|---|---|
| The relay's core, a table of 13 rows from the sentences above (what is passed on, with which Request ID, and what is not) | As expected on the first run; no expectation was changed |
| The relay over a connection in memory: a subscriber's SUBSCRIBE with NEW_GROUP_REQUEST = 0 for a track published with DYNAMIC_GROUPS = 1 | The publisher read `REQUEST_UPDATE` with Request ID 1 and the parameter, its session took it, and its next Group reached the subscriber |
| `bun new-group.ts --publisher own-rust`: own-rust publishes with the property, own-rust subscribes with the parameter, through the own relay | `SEND session=0 stream=2 REQUEST_UPDATE`; the publisher printed `NEW_GROUP_REQUESTED group=0`; the Object arrived |
| `bun new-group.ts --publisher own-browser`: the same with an own page in Chromium as the publisher | `SEND session=0 stream=2 REQUEST_UPDATE`; the page's handler was called with 0; the Object arrived |

A test of the relay's core that was wrong before the code was right: the SUBSCRIBE the relay sends upstream carried no parameter at first, because the subscriber it is sent for is not among those that wait when the relay asks. The expectation stood; the code now takes the value from the SUBSCRIBE that causes the asking.

## Not run

- A publisher that begins a Group for the request, and a viewer that asks instead of fetching: the second step.
- A track with several sources; a source the relay subscribed to (nothing is sent to it).
- A publisher that refuses the relay's REQUEST_UPDATE and ends the subscription with UPDATE_FAILED: read in the session's tests only.
- Any peer that is not of this repository.
