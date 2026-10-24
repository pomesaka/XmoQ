# An Object past the end of a track, at the relay

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the relay; a known gap of the implementation notes ("An Object after the end of a track is not found (§2.4.2)").

## Objective

Draft 18 §2.4.2 has as the fifth example of a malformed track "An Object is received whose Group and Object ID are larger than the final Object in the Track. The final Object in a Track is the Object with Status END_OF_TRACK", and "If a relay detects a Malformed Track, it MUST immediately terminate downstream subscriptions with PUBLISH_DONE and reset any fetch streams with Status Code MALFORMED_TRACK". The relay kept no end of a track: what a publisher sent after an Object with that status went on to the subscribers. Now a track remembers the Location of such an Object (`Track::past_its_end` in `crates/relay-core`), and an Object larger than it, or such an Object below the largest Location known, ends the track as the relay ends a malformed one already (for a Subgroup that comes again with another priority). The own Rust client can send such an Object: a line `end-of-track` on the stdin of its `publish` mode.

## Expected

**Written after the runs of the one pairing** and before the full interop run.

- An own-rust publisher sends a Group, an Object with the status END_OF_TRACK in the next Group, and one more Group: the subscribers (own-rust, own-browser) get PUBLISH_DONE `MALFORMED_TRACK` (0x12), and the relay cancels the PUBLISH with 0x12.
- With the check taken out of the relay for a moment, the same run fails: the subscribers are not told anything.
- The table test of `crates/relay-core` holds five orders of Objects to what the definition says.

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the working copy on `main` at `lnvzpzkr`; `bun interop/matrix.ts --only "END_OF_TRACK"`, and the full interop run.

## Result

- `--only "END_OF_TRACK"` passed: "the relay ended the track as malformed when pub:own-rust sent Group 2 after END_OF_TRACK in Group 1: PUBLISH_DONE 0x12 to the subscribers, the PUBLISH cancelled with 0x12 (pub:own-rust saw 0x12)", in 6.8 s, and again in 6.4 s after the code was tidied.
- With the check taken out (a temporary `&& false`, put back and seen gone in the diff): `FAIL ... timed out waiting for sub1:own-rust to print /^SUBSCRIPTION_ENDED why=(.*) received=/` after 31.7 s. The run tells the two apart.
- `mise run check` passed, with the table test: an END_OF_TRACK alone and one before an Object below it are not malformed; an Object after it in its Group or in a later Group, and an END_OF_TRACK below an Object already received, are.
- **Not run**: the end that a FETCH says (the relay does not read into a fetch stream it passes on); a track with two sources of which one sends the end.
- `mise run interop -- --base-port 45300`: 180 of 180 pairings ended as expected in 288.3 s, the run above among them.
