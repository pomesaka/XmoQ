# A FETCH goes to the next publisher

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, FETCH through the relay

## Objective

The relay asked one publisher for a FETCH it could not answer from its cache: the first source of the track. When that one refused, or went before it answered, the subscriber was refused, though another publisher of the track had the Objects. Transport §9.5 lets the relay choose: "When more than one publisher is available, the Relay MAY send the FETCH to any of them"; it says nothing of a second ask. Here the relay asks the next one after a refusal or a publisher that goes, and the run shows that the harm was there before and is gone after.

## Runs

`bun fetch.ts --fetch other-publisher --relay own` (own relay): the own Rust client publishes the track with PUBLISH and sends Group 0; it refuses every FETCH with `NOT_SUPPORTED`. Then an own page announces the namespace; it has Object 0 of Groups 100 to 102 and never sent them. A second page asks for {100, 0} to {102, 1}.

| Run | Expected before it | Result |
|---|---|---|
| The relay before the change | REQUEST_ERROR `0x10` (`DOES_NOT_EXIST`, what the relay makes of `NOT_SUPPORTED`), though the announcing page has the Objects | As expected: `OPEN_REQUEST session=0 own=1 FETCH`, `RECV session=0 own=1 REQUEST_ERROR error_code=0x3`, and the page got `0x10`. The relay had subscribed to the announcing page (`OPEN_REQUEST session=2 own=0 SUBSCRIBE`), so it was a source of the track as well |
| The relay after the change, 3 times | FETCH_OK with End Location {102, 1}, the Location after the announcing page's Largest Object (§10.13), and Objects {100, 0}, {101, 0}, {102, 0} | Passed 3 of 3: the refusal of session 0 was followed by `OPEN_REQUEST session=2 own=2 FETCH` and `RECV session=2 own=2 FETCH_OK` |

## What is not covered

- A publisher that does not answer in time is not followed by the next one: each would get the whole wait.
- A publisher that goes after its FETCH_OK still cuts the FETCH; what was passed on cannot be taken back.
- The order in which the publishers are asked is the order the relay holds them in, not who is likely to have the range.
