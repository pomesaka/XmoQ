# Three minutes behind a narrow link, and the relay's limit for one viewer

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link with less room than the video

## Objective

The relay counts what waits to be written to a subscriber for its session as a whole: 4 MiB of payload, 4,096 Objects and 256 subgroup streams not yet delivered (`SendLimits` in `xmoq-moqt-io`). The subscription whose Object goes over the limit ends with PUBLISH_DONE `TOO_FAR_BEHIND` (0x5). Read from the code, a viewer whose video fills the limit can have its sound's subscription ended with it (implementation notes, the gap on priorities, (3)). Earlier runs on a narrow link lasted 20 to 24 s. Here they last 3 minutes.

Expected, written before the runs:

- **The demo's viewer as it is** (it asks for a delivery timeout of 150 ms on the video): the relay resets each video stream that is not delivered 150 ms after its end, so what waits stays small. No PUBLISH_DONE 0x5 in 3 minutes, and the sound comes to the end.
- **The viewer with no delivery timeout for the video** (`videoTimeout=0`): what waits grows. The 256 streams are reached first, at about 5 s of backlog, since the sound opens 50 streams a second. So the sound's subscription is the one that is ended. After that the page gets no more sound and does not connect again, because the demo's viewer connects again when its video ends and not when its sound does.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback; publisher, relay and viewer on the one machine |
| Viewer | Chromium (playwright-core of `interop/`), headless; `/watch?via=48086&buffer=150` |
| Relay | own, Cubic, no flag |
| Link | `spikes/lossy-link/proxy.mjs` between viewer and relay: `rate` 2,000 kbit/s, `queue` 100 ms, 50 ms each way, no loss. The publisher goes to the relay directly |
| Sent | `/publish?picture=busy&bitrate=2500000`: about 2.65 Mbit/s of video, one Group a second; Opus, one piece of 20 ms a Group and a stream |

## Commands

```sh
bun demo/serve.ts --port 48081 --relay-port 48082 --rooms-port 48084 --relay-log <file>
node spikes/lossy-link/proxy.mjs --listen 48086 --to 48082 --control 48087
# a script (not in the repository) that sets the link, opens the two pages,
# and reads the viewer's report every 5 s for 180 s
```

Which subscription the relay ended is read from its log: the viewer subscribes to the catalog, the video and the sound in that order, on streams 1, 3 and 5 (stream 5 carried 8,971 streams in run A, the sound's count for 3 minutes).

## Result

| Run | Viewer | PUBLISH_DONE 0x5 | Sound, pieces a second (50 sent) | Picture |
|---|---|---|---|---|
| A | as it is (video timeout 150 ms) | none in 180 s | 50 throughout; came 122 to 137 ms after it was made | about 22 frames a second of 30; came 358 to 398 ms after |
| B | `videoTimeout=0` | **the sound first** (stream 5), then the video (stream 3) at once, at about 109 s | 50 until 53 s; then 37, 29, 26, 14, 20, 15, 13, 8, 6, 6, 1 a second, coming 255 to 804 ms late; none from 109 s | down to about 2 frames a second within 30 s, 2,098 Objects dropped as `late`; came 529 to 1,237 ms after |
| C | `videoTimeout=0` | **the video alone** (stream 3), at about 106 s | 50 until 53 s; then down to 7 a second at 103 s, coming up to 820 ms late; still coming when the video ended | as in B |

After the video's end in B and C the page said "the video track ended (subscription-ended) ... connecting again every 2 s", and had sound and picture again within 5 s on a new session. Then the same began again: in B the sound fell below 20 pieces a second about 70 s after the new session began, when the run ended.

- **With the demo's viewer as it is, nothing is ended in 3 minutes.** The delivery timeout on the video keeps the relay's backlog for that viewer small.
- **Without it, a subscription is ended at about 106 to 109 s, and which one varies**: the sound in one run of two, the video in the other.
- **The sound suffers long before that, and not because of the limit.** From about 55 s it comes later and later and fewer pieces come, while its subscription still goes on. It waits behind the video that waits in the relay: the demo's publisher gives the two the same priority unless `audioPriority` says otherwise, and the relay sends streams of one priority in turn. That is the relay's backlog, not its limit; a sound above the video was not run here.
- **Not as expected: the page connected again in both runs.** In B the video was ended a moment after the sound (the next Objects of the video went over the limit as well). Had the video gone on, the page would have stayed without sound: it connects again only when its video ends. That case was not seen.
- Not run: the page with ABR (`?abr`), more than two runs a case, other rates of the link.

## The sound above the video

The same, one run each, with the sound's streams at Publisher Priority 0 (`/publish?audioPriority=0`; the video is 128).

Expected, written before the runs: with no timeout on the video, the relay sends the sound before the video that waits, so the sound keeps 50 pieces a second and comes 120 to 140 ms after it was made for the 3 minutes; the video's subscription is still ended at about 105 s. With the demo's viewer as it is, the sound comes about as in run A (122 to 137 ms), since the relay holds little there.

| Run | Viewer | PUBLISH_DONE 0x5 | Sound, pieces a second (50 sent) |
|---|---|---|---|
| D | `videoTimeout=0` | **the sound first** (stream 5), then the video, at about 108 s | 50 until 53 s; then 42, 38, 32, 28, 21, 15, 16, 16, 6, 7, 4 a second, coming 202 to 693 ms late; none from 108 s |
| E | as it is | none in 180 s | 50 throughout; came 114 to 134 ms after it was made |

- **Not as expected: the priority does not keep the sound from declining.** Run D is run B again, within a few pieces a second, and the sound's subscription was again the one ended first. So what the sound waits behind from about 55 s is not the order in which the relay hands streams to its transport. Where it waits was not found here. Whether the sound's streams left the relay with priority 0 was not looked at on the wire; the page sets it, and the relay takes it from the header.
- With the demo's viewer as it is, the priority changes nothing that was measured (114 to 134 ms against 122 to 137 ms).

## After the relay ends the subscription that carries the most, and the sound in fewer streams

On 2026-10-14 the relay was changed to end, when a session's limit is used up, the subscription whose open streams carry the most payload bytes, and not the one that went over (implementation notes, row on a subscriber that does not take what is sent to it). Then, one run each, as run B (`videoTimeout=0`):

Expected, written before the runs: F, the sound declines from about 55 s as before, and at about 105 s the video's subscription is ended and the sound's is not. G (`/publish?audioGroup=10`, ten pieces a stream, 5 streams a second): if what the sound waits behind is the number of streams, it keeps 50 pieces a second until the video is ended; if not, it declines from about 55 s.

| Run | Sound | PUBLISH_DONE 0x5 | Sound, pieces a second (50 sent) |
|---|---|---|---|
| F | a piece a stream | **the video alone** (stream 3), at about 105 s | 50 until 48 s; then 40, 25, 20, 18, 5 a second, coming 298 to 801 ms late; 50 again on the new session |
| G | ten pieces a stream | the video alone, at about 105 s | 50 until 48 s; then 42, 18, 15, 7, 7, 4 a second, coming 278 to 1,157 ms late |

- **As expected, F: the sound's subscription was not ended**, and the page connected again because the video was.
- **G: the sound declines in fewer streams as well, and comes later**, so the number of streams is not what it waits behind.
- **What fits the times, not shown directly**: the relay hands a stream's bytes to its transport as they come, and the transport takes up to its send window for the connection (4 MiB, `SEND_WINDOW` in `crates/relay/src/main.rs`). With about 0.7 Mbit/s more sent than the link carries, 4 MiB are filled after about 48 s, plus the few seconds the video takes to begin: about where the sound began to decline in every run. Once that window is full, a writer waits for room in it whatever its priority (implementation notes, priorities, (2)), which would also explain why the sound above the video (run D) did not help. Neither the window nor the writers' waits were looked at during the runs.
