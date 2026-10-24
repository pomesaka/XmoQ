# TRACK_STATUS through the relays

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: none of its own; draft 18 §4 (MUST, "Relays MUST implement all MOQT messages defined in this document") and §10.14, the known gap "Messages the relay does not implement" of the [implementation notes](../moqt-implementation-notes.md). Decided on 2026-10-24: the relay answers a track it has a source of, asks the publishers that announced a namespace of it otherwise, and says `DOES_NOT_EXIST` where nobody is to be asked; the clients send TRACK_STATUS and answer none.

## Objective

That both own clients can ask both relays for the status of a track, and that what each relay answers is what the implementation notes say of the own relay ("TRACK_STATUS in the relay") and what was read of `moqtail`'s (`fixtures/moqt-18/`, capture P).

## Expected

Written before the first run, at the top of `interop/track-status.ts`. The own Rust client publishes `object` of `xmoq/phase0` with PUBLISH and the Track Property DYNAMIC_GROUPS = 1, announces the namespace, and refuses every TRACK_STATUS it is asked with `NOT_SUPPORTED`. The client asks four times:

| Question | Own relay | `moqtail`'s relay 0.15.0 |
|---|---|---|
| 1. `object`, before any Object | `TRACK_STATUS_OK largest=none track_properties=1` | `TRACK_STATUS_OK largest=0,0 track_properties=0` |
| 2. `object`, after a subscriber had Object 0 of Group 0 | `TRACK_STATUS_OK largest=0,0 track_properties=1` | `TRACK_STATUS_OK largest=0,0 track_properties=0` |
| 3. `other` of the namespace, which nobody published | `TRACK_STATUS_REFUSED error_code=0x10` | `TRACK_STATUS_REFUSED error_code=0x3` |
| 4. `nothing` of `nobody`, which nobody announced | `TRACK_STATUS_REFUSED error_code=0x10` | `TRACK_STATUS_REFUSED error_code=0x10` |

## Environment

macOS 14.4, arm64, loopback; the working copy on `main` after the relay's TRACK_STATUS; `mise run interop -- --only "track status:" --base-port 48100` (the own Rust client's `--mode track-status`, and `MoqtConnection.trackStatus` of `@xmoq/moqt` in Chromium 156).

## Result

- All four pairings (own-rust and own-browser, through each relay) ended as expected, each answer as in the table, on the first run of each.
- The two relays differ as was read of `moqtail`'s: it answers `LARGEST_OBJECT` {0, 0} for a track it has before any Object came, puts no Track Properties in TRACK_STATUS_OK although the PUBLISH had one (§9.6 has "A relay MUST include all Properties associated with a Track when sending any ... TRACK_STATUS_OK"), and passes a publisher's `NOT_SUPPORTED` on as it came, where the own relay passes a refusal on as for a SUBSCRIBE and makes it `DOES_NOT_EXIST`.
- In question 3 the own relay asked the one session that announced the namespace, the publisher; that it asks the next one after a refusal was seen with two such sessions in a run by hand while the relay was written (both refused, and the asker had `DOES_NOT_EXIST`), not in this matrix.
- **Not run**: a publisher that answers a TRACK_STATUS (no client of this repository answers one); a TRACK_STATUS that times out at a publisher, or whose publisher goes, through a relay (the relay's core tests have both); the limit of 8 requests for one subscriber's session on a publisher.
