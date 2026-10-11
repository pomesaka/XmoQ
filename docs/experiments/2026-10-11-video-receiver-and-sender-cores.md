# The video receiver and sender of `@xmoq/media`, without a connection or WebCodecs

- Date: 2026-10-11
- Commit SHA: the commits that add `createVideoReceiver` and `createVideoSender` to `@xmoq/media`, together with this record. The runs were made on their working copy, on `main` at `20452147`
- Roadmap item: groundwork for Phase 1 ("Browser WebCodecs publisher", "Decode and render video"). It closes no item

## Objective

Two state machines stand between a video track and WebCodecs: one says which Object a chunk of an encoder is, and one says which frames a decoder gets of the Objects a subscription delivers, and in which order. They are written before anything is connected to them. The runs are to show that

1. what the sender writes, the receiver reads, in both ways of putting Objects onto streams and with the Objects of a Group out of order;
2. frames of a publisher that is not ours go through both;
3. the rules each one has for what it drops are the ones its README says.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64 |
| Runtime of the tests | Bun 1.3.14 |
| Frames of another implementation | two video payloads of `mlmpub` of `Eyevinn/moqlivemock` v0.16.1, `fixtures/loc-04/` |
| Drafts | draft-ietf-moq-loc-04; draft-ietf-moq-msf-01; draft-ietf-moq-transport-18 |

## Commands

```sh
mise run check
(cd packages/media && bun test)
```

## Observed results

`@xmoq/media`: 103 tests, none failing, after the reviews of both (86 before the sender was reviewed). `mise run check` passed.

| Run | Expected, and seen |
|---|---|
| Two Groups of three chunks through the sender, its Objects into the receiver: a stream for every Group; a stream for every Object, in order; a stream for every Object, the Objects of each Group in another order | In each of the three, six frames come out in the order of the chunks, the first of each Group marked as a key frame, each with the encoder's time counted from the epoch that was given and with the bytes of its chunk. The expectations are written out in the test |
| The two captured payloads of `mlmpub`, a key frame of 21,533 bytes and the frame after it of 931, into the receiver with the Timestamp the capture had | Two frames: the first a key frame that begins `00 00 00 01 67`, the second not a key frame, beginning `00 00 00 01 41`; the lengths unchanged, the times the ones of the capture |
| The same two frames in Annex B form as chunks of an encoder, through the sender, and its Objects into a second receiver | The same two frames as in the row above, field by field |
| The rules of the receiver, 45 rows: the order, a join in the middle of a Group, a frame that does not come, a frame that cannot be used, End of Group and End of Track, the limits | As the rows say. The expectations of the rows on End of Group and End of Track were written again from the rule after the reviews had it changed (an End of Track is told when it arrives), before the code was |
| The rules of the sender, 11 rows and the tests of the time: Groups and streams, chunks that are not sent | As the rows say |

- Temporary changes to the receiver, one at a time, each taken back: 27 for the first version, of which 3 changed nothing a test could see, as the same check stood in two places, and were removed with the second place; 24 for what the reviews had changed, of which 5 were not caught at first: three got a row each, and two were a tidying that changed nothing and was removed.
- Temporary changes to the sender: 26, of which one was not caught and got a row (a chunk with a length in place of a start code).
- After the reviews of the sender, four things were changed in it, each from a run or a sentence the reviews had: a time over 2^53 - 1 microseconds is not sent, as the receiver here would not hand it on (the sender had taken up to 2^64 - 1); a key chunk when no Group ID is left is not sent, where `push` had thrown after it had changed what it holds; a stream of a Group in which an Object could not be written is reset and not finished (transport §11.4.3); and the longest payload without a setting is 1,000,000 bytes and not 1 MiB, which the relay here does not pass. 15 temporary changes to those, each taken back: all made tests fail.
- **That a payload of 1,000,000 bytes passes the relay, and one of 1 MiB does not, was not run**: it is from the relay's code and its test of the limit.
- A type of `@xmoq/moqt` goes into the receiver in a test: an Object as a subscription delivers it (`ReceivedObject`). The receiver's own type for it is written without that package, and the test does not compile where the two differ.

## Limitations and blockers

- **No encoder, no decoder, no connection, no browser.** Whether a decoder takes the frames in the order and form they come out was run for two frames of `mlmpub`, by hand, in an [earlier record](2026-10-11-loc-video-payloads-in-chromium.md), not through this receiver.
- At first the stream the two payloads came on was not decoded in these tests, and the properties were written out as the README of the fixtures has them. Since `@xmoq/moqt` exports its decoder of a subgroup stream, the tests decode `fixtures/moqt-18/subgroup-stream-two-objects-from-moqlivemock.hex` and hand on what comes out.
- No chunk of a real encoder went through the sender.
- Nothing was measured: the limits of the receiver (32 Objects, 4 MiB) and of the sender (1 MiB) are not from a run.
- Sequences of Objects at random were not run here; the reviewer of the receiver ran 60,000 against its first version and found no rule broken.

## Open questions raised

- When a frame counts as lost at the receiver, and whether an Object without a Timestamp is played: in `docs/open-questions.md`.
