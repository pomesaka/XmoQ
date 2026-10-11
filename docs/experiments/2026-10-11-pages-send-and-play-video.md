# A page encodes and publishes video, and another plays it, through a relay

- Date: 2026-10-11
- Commit SHA: the commit that adds `createVideoTrackEncoder` and the runs, together with this record. The runs were made on its working copy, on a `main` from before the relay had its cache
- Roadmap item: Phase 1, video from a browser to a browser through the relay

## Objective

The sender and the publisher of `@xmoq/media` had been run against tests alone. To do: connect them to the H.264 encoder of a browser, publish through a relay, play in another page with `playVideoTrack`, and see by the picture itself that every frame arrives, once and in order.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | own (without the cache of the latest Group); `moqtail` relay 0.15.0 |
| Drafts | draft-ietf-moq-transport-18, draft-ietf-moq-loc-04, draft-ietf-moq-msf-01 |

## Commands

```sh
mise run check
mise run interop -- --base-port 45300
(cd interop && bun video.ts --video page-to-page --through own --mapping stream-per-group)   # each of the four, alone
```

## What a run does

A page makes a `createVideoTrackEncoder` (`avc1.42E01F`, 320x180, 25 frames a second, 500,000 bits a second, a key frame every 1,000 ms) and a catalog with the track (`avc3.42E01F`, no `timescale`), and announces the namespace. Every 40 ms it paints frame `n` as 8 bars, black for a 0 and white for a 1, the bits of `n` modulo 256, and encodes it with the time `5,000,000 + n * 40,000` microseconds. When it has painted 30 frames, a second page comes, reads the catalog (through the own relay), and plays the track; it reads the bars of every decoded frame in the middle of each bar, as dark (below 64 of 255), light (above 192), or neither.

## Observed results

What had to be seen was written at the top of `interop/video.ts` before the first run.

| Run | Seen |
|---|---|
| own relay, a Group on a stream | 50 frames, each 320x180, each with the number after the one before it (from 34 or 35 on) and 40,000 microseconds after it; the first within a minute of the clock; no decoder failure; nothing dropped by the viewer. Publisher: 1 key frame asked for, 4 key chunks, no write failed, 26 to 30 chunks unsent as `no-subscriber` before the relay came. As expected, but for the one point below |
| own relay, an Object on a stream | The same |
| moqtail relay, a Group on a stream | The same, without the catalog (below) |
| moqtail relay, an Object on a stream | The same, without the catalog |

- **One expectation was wrong: "every frame that was painted was handed to the encoder".** In the first run 9 of 85 were not, as `encoder-behind` (more than 2 frames waited in the encoder); in the six runs after it, none. The viewer's numbers had no gap in that run, so the frames were lost before the viewer came; when exactly was not looked at. The run now takes frames that were not encoded for that reason, and says how many.
- **The viewer came in the middle of a Group and dropped nothing**: the relay subscribed when the viewer came, the publisher asked its encoder for a key frame with the next chunk, and the relay's first Object was Object 0 of a Group. With that request taken out, the run fails (below).
- **The largest chunk was 256 to 264 bytes.** Bars of black and white are no picture of a camera: nothing here says how large a key frame is, and the limits on an Object and on what waits were not reached.
- **Through the moqtail relay the viewer gets no catalog.** The relay answered the viewer's Joining FETCH for the catalog itself, for {0, 0} to {0, 1} (its log: "Fetching objects from Location { group: 0, object: 0 } to Location { group: 0, object: 1 }"), where the catalog is at Object 0 of the Group of its time in milliseconds; the fetch stream came empty, and the subscription brings a catalog only when one is published after it. So through that relay the viewer is told the codec by the driver.
- No encoder failed, so what `createVideoTrackEncoder` does then was not run.

### Temporary changes, each taken back

| Change | The run |
|---|---|
| Every 40th frame is painted with the number of the one before | Fails: a frame "does not show the number after the one of the frame before it" |
| Every 40th frame is not encoded | Fails the same way, a number left out |
| The publisher asks for no key frame when a subscriber comes | Fails: no key frame asked for (and the viewer waits for the next one) |
| The time of an Object is the time of its frame, without the clock | Fails: the first frame has the time 6,400,000 |
| A bar is read at its left edge, not in its middle | Passes: the edges of bars this wide were not blurred enough to matter. The middle is still where it is read |

`mise run check` passed. `mise run interop -- --base-port 45300`, on the `main` these runs were first made on: 108 of 109 with the four runs here, which were among the 108. The one that failed was the catalog read by `moqlivemock`'s subscriber, which failed now and then on that `main` for a race that PR #89 has mended; nothing of this change is in its way. With the four runs with noise, below: 113 of 113.

## A picture an encoder cannot make small

`--picture noise`: 1280x720, the lower half random pixels that change with every frame, `avc1.640033` at 30,000,000 bits a second, and a viewer that takes Objects of 1 MiB at most (`limits.maxObjectPayloadLength`). Expected, written before the first run: everything of the run with bars; nothing unsent as `too-large` or `behind`; a Group of more than 2 MiB and a chunk of 200,000 bytes or more.

| Run | Seen |
|---|---|
| own relay, a Group on a stream (3 runs) | As expected at the first run: 50 frames in a row by their numbers, nothing dropped, nothing unsent but `no-subscriber`. The largest chunk 638,784 to 638,963 bytes; 25 chunks 3.92 MB on average. No frame lost to `encoder-behind` |
| own relay, an Object on a stream (2 runs) | The same; 639,075 bytes at most |
| moqtail relay, each mapping (1 run each) | The same; 640,033 bytes at most |

- **A Group of 3.9 MB on one stream goes through both relays**, and through the own one with every Object on a stream of its own, 25 streams a second: nothing in the relay's limits (1 MiB undecoded of a stream; 256 streams, 4 MiB and 4,096 Objects waiting for a subscriber) was reached with a viewer that reads.
- **With more bits, a Group of 5.8 MB (45,000,000 bits a second) and of 8.9 MB (70,000,000) went through the own relay as well**, one run each, by a temporary change. The largest chunk stayed at 639,000 bytes: it is the key frame, and this encoder makes it no larger at this size.
- **A key frame near the sender's limit**, one run each by a temporary change (noise on 76% of the height): 988,512 bytes through the own relay with a Group on a stream, 988,577 with an Object on a stream, 988,330 through the moqtail relay; 50 frames in a row each time.
- **A key frame over the limit**, by temporary changes (noise on 78% of the height; 1600x900; 1920x1080): key chunks of 1,004,815 and 1,442,595 bytes. **Nothing was sent at all**: every key chunk `too-large` (13 in 10 seconds), every other chunk `no-key-frame`, the viewer without a frame, and nothing that says so but the counts. Two key frames were asked for.
- `maxEncodeQueue`: no frame was lost to it in any of these runs.
- All temporary changes were taken back.

## On a slower machine: Linux in CI

Run 38123516380 of the pull request, Ubuntu: the browser has an H.264 encoder (the run with bars through the moqtail relay passed), and one run with noise failed, through the moqtail relay with a Group on a stream: "frame 11 (number 62) does not show the number after the one of the frame before it, 47".

- The viewer: 50 decoded of 51 given to the decoder, dropped `decoder-behind` 1 and `late` 13, no failure of the decoder. The publisher: 102 chunks encoded, 65 written, nothing unsent but `no-subscriber`, no write failed; the largest chunk 424,117 bytes (639,000 here: another encoder).
- **Read as**: the decoder was behind once (more than 8 chunks waited in it), and `playVideoTrack` gave the rest of that Group up, as it is to. 14 Objects dropped, 14 numbers passed over. Everything was carried.
- **Checked here**: with a decoder that may have 1 chunk waiting (a temporary change), the same run gave `decoder-behind` 1 and `late` 18 in one run of three, and `late` 16 or 17 in one of six after the change below.
- So "the viewer dropped nothing" was an expectation about the speed of the machine. **What the runs look at now** (the top of `interop/video.ts`): the publisher ends the track, the viewer's subscription ends, and the Objects the publisher wrote are as many as the viewer gave its decoder or dropped; the viewer dropped for a decoder that was behind and for nothing else; every frame shows a later number than the one before it and is that many times 40,000 microseconds after it; no more numbers are passed over than Objects were dropped.
- With that, the publisher's count and the viewer's were the same in every run here, through both relays and with both mappings: 50 or 51 Objects.

| Temporary change, each taken back | The run |
|---|---|
| A decoder that may have 1 chunk waiting | Passes, with what was dropped said: 17 Objects, 17 numbers passed over, in one run of six |
| Every 30th Object does not reach the receiver | Fails: "the publisher wrote 177 Objects and the viewer got 170" |
| Every 40th frame is not encoded | Fails: "2 numbers are passed over between the frames, and the viewer dropped 0 Objects" |
| Every 40th frame is painted with the number of the one before | Fails: a frame that does not show a later number |

## The caller is told when no key frame can be sent

`createVideoTrackEncoder` now calls `onKeyChunksNotSent` when 3 key chunks in a row were not sent for what they are. `--video key-frames-too-large`: the bars, with payloads of 100 bytes at most, through the own relay, nobody viewing. Expected, written before the first run, and seen at it: the page was told once, `{"because":"too-large","inARow":3,"length":264}`; with 5 key chunks encoded it had still been told once; no Object written, 5 chunks `too-large`, the encoder not failed.

Temporary changes, each taken back: told at every key chunk from the third on, never told, told at the first: the run fails for each.

Also in the runs with bars now: the page hands the encoder one frame with the time of the frame before it, while the viewer plays. It is counted as `time-not-after` and not encoded; with that check taken out, the run fails (the viewer has a number twice).

## A chunk of the encoder, kept

The first two chunks of the encoder are in `fixtures/loc-04/` (its README has how they were taken): 229 and 92 bytes. **Every NAL unit in them has a start code of 4 bytes**, the ones after the first of a chunk as well, which had not been looked at. The sequence parameter set says `42 00 0d` where the encoder was configured with `avc1.42E01F`.

## `moqlivemock`'s subscriber takes the video

`--video to-mlmsub`: the bars, a Group on a stream, through the own relay; `mlmsub` v0.16.1 with `-videoname video -loglevel debug`. What it does with a track was read in its source first (`internal/sub/sub.go`, `internal/sub/loc.go`, commit `91d903e9`).

| | Seen |
|---|---|
| The catalog | Read with a Joining FETCH; "selected video track based on substring match trackName=video"; "catalog uses LOC packaging" |
| Groups | Three in a row, Group IDs up by 1, Subgroup 0, Timestamps 1,000,000 microseconds apart and at the time of the clock. As expected |
| Objects | 1 to 24 of the first Group, 40,000 microseconds apart. As expected |

- **With `-videoout`, `mlmsub` cannot take this video, and one expectation was wrong.** Its `LOCVideoWriter` takes a payload to have a length of 4 bytes before every NAL unit. It read the start code of the first Object as a length of 1 and the next four bytes as a length: `failed to write LOC sample error="LOC AVC: NALU length 1107299721 exceeds remaining data 237"`, which had been worked out from the source. **What had not been read is the `return` after that line**: `mlmsub` reads the track no further, so the three Groups that were expected all the same did not come. The run is without `-videoout`.
- So the two implementations agree on the transport, the catalog and the Timestamp, and **not on the form of the payload**: `moqlivemock` sends and reads lengths, the sender here writes start codes, and the receiver here reads both.

## A viewer that comes late: a Joining FETCH

`playVideoTrack` now sends a Relative Joining FETCH with Joining Start 0 for its subscription, and gives what it brings to the same receiver. While it comes, and until the decoder has caught up, more than 8 chunks may wait in the decoder.

- `--video late-viewer` (own relay with its cache, bars, a second viewer while a first watches). Expected, written before the first run, and seen in 3 runs of 3: the FETCH served with 19 Objects; the first frame from 739 to 741 ms before the viewer came; 30 frames in order; nothing dropped.
- The demo (`mise run demo`), headless, a viewer every 2 seconds while another watches, by the page's own count from the accepted SUBSCRIBE to the first frame:

  | | First frame after | Dropped | Joining FETCH |
  |---|---|---|---|
  | `/watch` | 3, 4, 4, 3 ms | nothing | served 10, 23, 3, 14 |
  | `/watch?join=no` | 500, 114, 804, 181 ms | `no-key-frame` 14, 3, 23, 5 | not sent |

- The other runs of `video.ts` play without the FETCH, as before.
- Not looked at: a first viewer (nobody answers the FETCH: the relay has nothing yet); a Group longer than the receiver holds; the moqtail relay; what the burst of old frames looks like.

## Limitations and blockers

- **Not run**: a viewer that comes later than the relay's subscription (it waits for the next key frame; a Joining FETCH is not sent); frames of a real picture; a viewer that does not read, with a stream for every Object, which is where the relay's limit of 256 streams would be met; a subscriber that is behind and a write that fails, with a browser; `moqlivemock`'s subscriber as the viewer; Linux.
- **No chunk of the encoder is kept in `fixtures/`.**
- The viewer's page of the runs with bars connects with the default limit on an Object, 16 MiB; the one of the runs with noise with 1 MiB.
- **The sizes are this encoder's** (Chromium 156 on macOS, arm64). Another encoder can make the key frame of the same picture larger than 1,000,000 bytes, and the run with noise then fails with no frame.
- The frames are painted by a timer of the page, which is not exact; their times are the ones the page writes, 40,000 microseconds apart, so the spacing the viewer sees says nothing of when a frame was painted.

## Open questions raised

- Whether `maxEncodeQueue` of 2 is right: frames were lost to it while the encoder started.
- What a caller is to do when it is told that its key frames are over what is sent, and whether the limit is to go up.
- Whether the sender is to write lengths before the NAL units, as `moqlivemock` does, for a subscriber like `mlmsub`.
- Whether `playVideoTrack` is to send a Joining FETCH, now that the relay keeps the latest Group.
