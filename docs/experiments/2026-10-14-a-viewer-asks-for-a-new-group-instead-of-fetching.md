# A viewer asks for a new Group instead of fetching

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, video (how a viewer that comes while a track runs gets its first picture)

## Objective

The second of two steps ([the first](2026-10-13-a-subscriber-asks-for-a-new-group.md)). A viewer of this repository that comes in the middle of a Group gets the start of that Group with a Joining FETCH, or waits for the next key frame. Here the video sender of `@xmoq/media` begins a Group when a subscriber asks (transport §10.2.13), and `playVideoTrack` can ask in its SUBSCRIBE (§5.1.3.1: "Another possible subscriber pattern is to send a SUBSCRIBE with Filter Type Next Group Start and NEW_GROUP_REQUEST equal to 0 ... A publisher that does will begin the next group as soon as practical"). The question for the user is which of the two a viewer uses when nothing is said; this record has the numbers for it.

## What was built

- `createVideoTrackPublisher` and `createVideoTrackEncoder`: `dynamicGroups`, off when left out. With it the SUBSCRIBE_OK has DYNAMIC_GROUPS = 1, and a request of 0, or of a value larger than the Group being sent, makes the next frame a key frame once the Group is `shortestGroupMs` long (500 ms when left out, provisional).
- `playVideoTrack`: `newGroupRequest`, off when left out: NEW_GROUP_REQUEST 0 in the SUBSCRIBE.
- The relay: a REQUEST_UPDATE with the request also to a source the relay subscribed to, on the stream of its own SUBSCRIBE. The page publishers of this repository are such sources: they announce a namespace and are subscribed to. Before, only a source that came with a PUBLISH was asked, and the relay's SUBSCRIBE carried the request only when the track had no source yet.
- The demo: `/publish?dynamic=MS` and `/watch?newgroup=1`, both trials, off when left out.

## Runs

| Run | Expected before it | Result |
|---|---|---|
| `bun video.ts --video late-viewer-new-group` (own relay; a page publishes the bars with a key frame every 10 s and dynamic Groups of 500 ms at least; one page watches; a second comes with NEW_GROUP_REQUEST 0 and no Joining FETCH) | The relay logs `SEND_OWN_REQUEST ... REQUEST_UPDATE`; the publisher takes one request and asks for one key frame; the second viewer's first frame was made after it came and comes within 1,000 ms of its SUBSCRIBE; nothing dropped but `no-key-frame` and `decoder-behind`; 30 frames in order | Passed 3 of 3 runs: the line was there, `newGroupsAskedFor` 1 and `newGroupsBegun` 1, the first frame 123, 112 and 140 ms after the SUBSCRIBE, 1 Object dropped as `no-key-frame` each time |
| `bun video.ts --video late-viewer`, once, after its checks were moved into functions of their own | As before | Passed: 18 Objects fetched, the first frame 707 ms older than the viewer |

## Measurements

The demo (`bun demo/serve.ts` on other ports), Chromium headless on one machine, loopback. One publishing page with `audio=none`, `keyint` as below and `dynamic=` as below; one watching page with `join=no` that stays, so that the relay's subscription to the publisher is Established. Then late viewers one after another, each after a random wait of up to a Group, in turn: no Joining FETCH (`join=no`), a Joining FETCH (the demo's way), and the request (`join=no&newgroup=1`); 10 of each. What is counted is the page's `firstFrameAfterMs` minus its `playingAfterMs`: from when its SUBSCRIBE was accepted to the first frame out of the decoder. On screen it is 55 to 65 ms later (the playout buffer is 50 ms). The script is not in the repository.

| Groups | Shortest Group | No FETCH, no request | Joining FETCH | NEW_GROUP_REQUEST |
|---|---|---|---|---|
| 1 s | 500 ms | 47 to 1,028 ms, median 552 | 1 to 3 ms, median 1 | 15 to 480 ms, median 96 |
| 4 s | 500 ms | 373 to 3,973 ms, median 2,040 | 1 to 4 ms, median 2 | 63 to 579 ms, median 83 |
| 4 s | 0 | 851 to 3,993 ms, median 2,867 | 1 to 4 ms, median 2 | 63 to 97 ms, median 86 |

- **The request takes about 60 to 100 ms where the Group is long enough.** How that is made up (the next frame, the encoder, the way through the relay) was not taken apart. Where the Group had begun less than 500 ms before, it waits for the rest (the two over 480 ms in the second row; none in the third).
- **The Joining FETCH has a frame at once, but an old one**: the frames from the start of the Group, up to a Group older than live, come together and are handed on as fast as the decoder gives them. How that looks to a person was not looked at.
- **What the requests cost**: with Groups of 1 s, 10 requests began 8 Groups (two were met by a key frame that came first), and the publisher made 54 key frames in 1,575 frames; with Groups of 4 s and 500 ms, 10 Groups for 10 requests, 35 key frames in 3,484 frames, where about 29 come by the interval alone. Bytes were not compared.
- Not measured: a link that loses packets or has little room, many viewers that come at once (the relay sends one request while one is outstanding), a camera picture, and the terminal viewer.
