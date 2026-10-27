# @xmoq/media

What stands between a LOC video or audio track and the codecs of a browser: from a subscription to a decoder, from the chunks of an encoder to a subscription, picture and sound played together by one clock, and a publisher and a viewer that go on over more than one session. Drawing a frame and taking one from a camera are the caller's. "LOC" is `draft-ietf-moq-loc-04`, "MSF" `draft-ietf-moq-msf-01`, and "transport" `draft-ietf-moq-transport-18`. Why each rule and value is so is at the function or constant that holds it.

## What a sender and its viewers count on

- **Object 0 of a Group is a key frame, and a decoder begins there.** Nothing in the payload is looked at. No draft has it as a rule: transport §2.3, MSF §4.1 and the example of LOC-04 §4.2 point to it, and it is taken as a promise of the sender. The sender here and `moqlivemock` keep it, and the receiver here and `crates/picture` count on it. The Video Frame Marking of LOC is not read.
- **The Timestamp of LOC is wall-clock time in microseconds, without a Timescale**, so **the catalog of a track this package sends has no `timescale`** (MSF §5.2.21): a receiver given one reads these Timestamps in that unit. The catalog is the caller's to write.
- **The catalog of a video track says `avc3.…`** with the profile and level the encoder was configured with. Why, at `codec` of `createVideoTrackEncoder`.
- The rules of the playout that the terminal viewer (`crates/watch`) shares are the rows of [`fixtures/playout/`](../../fixtures/playout/README.md).

## Not met

- **An Object after an End of Group or an End of Track cancels nothing** (transport §2.4.2, MUST). The receiver cannot tell it from the case §2.1 has as no error, and only drops the Object; `@xmoq/moqt` finds one only on the same subgroup stream.
- **Of what a publisher can say about Objects that do not exist, only the Object Status is read.** The END_OF_GROUP bit of a subgroup stream does not reach the receiver, and the Prior Group ID Gap and Prior Object ID Gap properties (transport §12.8, §12.9) are passed over: a Group with a gap its publisher announced is given up like one with a loss.
- **After a restart the sender does not say which Groups it left out** (MSF §6.1, SHOULD: the Prior Group ID Gap).
- **A Group that was cut short is not marked as ended**: the sender gives no End of Group.
- **The end of the subscription is not told to the receiver** (PUBLISH_DONE with TRACK_ENDED, which MSF §11.3 ends a broadcast with). That is for what connects it.
- No Audio Config or Audio Level of LOC, and no `description` is sent or given to an audio decoder (Chromium's encoder gives one of 19 bytes; whether it is to travel is undecided).

## Known gaps

- **A publisher whose video and sound are timed by two clocks loses one of them at the viewer.** The fastest way is one for both tracks, so the track whose clock runs faster keeps renewing it and the other is later and later ([record](../../docs/experiments/2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md)). With `clock: "least"` of the encoders, not the default, a source 1,000 ppm off held ([record](../../docs/experiments/2026-10-13-an-hour-of-the-demo.md)); not run with a camera or a microphone.
- **A publisher cannot send to several viewers without a relay**: it has one subscription.
- **A track whose key frames are all over `maxPayloadLength` sends nothing.** `createVideoTrackEncoder` tells its caller (`onKeyChunksNotSent`) and goes on; what to do then is the caller's, and nothing makes the frames smaller.
- **A viewer that comes while a track runs has its picture at once only where something answers its Joining FETCH, or the sender begins a Group for it** (`newGroupRequest` and `dynamicGroups`, both off when left out). Otherwise it waits for the next key frame, up to the key frame interval ([record](../../docs/experiments/2026-10-14-a-viewer-asks-for-a-new-group-instead-of-fetching.md)).
- **A subscriber that comes while the SUBSCRIBE of it is answered misses a key chunk that comes meanwhile**, and waits for the next one the encoder makes where the caller does not act on `askForKeyFrame`.
- **The sender does not keep to a frame rate or a size of a Group**: a source that sends no key chunk makes one Group without end.
- **Over more than one session**, the viewer does not ask for what it missed, and nothing limits how long a session is tried.
- **`moqlivemock`'s subscriber cannot write the video of this sender out.** `mlmsub` v0.16.1 takes a LOC video payload to have a length before every NAL unit, and this sender writes start codes; LOC-04 has both forms, and the receiver here reads both.
- Video does not wait for a Group that is late, nothing conceals a piece of sound that is missing, and nobody judged the sync by eye and ear.
- Only H.264 was run.
