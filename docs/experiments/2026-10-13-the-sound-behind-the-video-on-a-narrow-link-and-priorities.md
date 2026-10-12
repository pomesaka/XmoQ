# The sound behind the video on a narrow link, and what priorities do for it

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link with less room than the video. [The terminal viewer's sound output ran dry 766 and 1,516 times in 24 s](2026-10-12-the-terminal-viewer-chooses-a-version-of-the-video.md) with the large version of the video on such a link, and never with the small one.

## Objective

1. Why the sound suffers: do its Objects come late, or not at all, and is a page better off than the terminal viewer?
2. What draft 18 has on priorities, and what the publisher, the viewers and the relays do with them now.
3. Whether a relay that schedules by those priorities keeps the sound in time.

## 1. The facts first

The link: 2,000 kbit/s, 50 ms each way, queue 100 ms (`spikes/lossy-link/proxy.mjs` between relay and viewer); the busy picture at 2.5 Mbit/s with a timeout of 150 ms asked for it; Opus, ten pieces of 20 ms a Group; the own relay with no flag, Cubic. macOS 14.4, arm64, loopback.

**The terminal viewer** (`try-narrow-link.sh 25 --video-timeout-ms 150 --measure-demo-marks on --sound off`, one run each):

| `--buffer-ms` | Packets of sound that came | Left out as too late | The output ran dry | Sound played, of 24 s | A packet came after the fastest, ms (half; 95 of 100; the most) |
|---|---|---|---|---|---|
| 50 | 1,201 | 920 | 1,847 times | 5.5 s | 78; 104; 334 |
| auto (50 to 400) | 1,201 | 23 | 57 times | 23.4 s | 63; 89; 509 |
| 300 | 1,198 | 0 | 0 | 23.7 s | 74; 105; 119 |

**A page** (`/watch?buffer=N`, 20 s counted, 1,000 pieces, two runs each):

| Buffer | Pieces sounded | Too late | Breaks | A piece came after the fastest, ms (half; 95 of 100; the most) |
|---|---|---|---|---|
| 50 ms (the default) | 233, 254 | 769, 746 | 73, 104 | 65 to 70; 93 to 97; 125 to 276 |
| 150 ms | 1,002, 997 | 0, 0 | 0, 0 | 66 to 71; 96 to 105; 118 to 308 |
| 300 ms | 995, 998 | 0, 0 | 0, 0 | 66; 94 to 100; 116 to 296 |

- **Every piece of the sound comes. It comes 65 to 105 ms later than the fastest one**, which a buffer of 50 ms does not cover and one of 150 ms does.
- **The page is not better off than the terminal viewer.** The records that have "the sound is whole" on this link ran the page with a buffer of 150 ms; with its default of 50 ms three pieces of four are too late.

## 2. Priorities: the draft, and who does what

Draft-ietf-moq-transport-18:

| | The text |
|---|---|
| The numbers | §7.1: "A lower priority number indicates higher priority; the highest priority is 0." Subscriber Priority is of a request (SUBSCRIBE, FETCH; the parameter of §10.2.7), Publisher Priority of an Object: "A single subgroup or datagram has a single publisher priority", from the subgroup header or the track's default (DEFAULT PUBLISHER PRIORITY, §12.4: "If omitted, the Default Publisher Priority is 128") |
| The order | §7.2: "the objects SHOULD be selected as follows": first by "the highest subscriber priority"; then, with the same, by "the highest publisher priority"; then, "in response to the same request" and with both the same, by "the group order of the associated subscription"; then by "the lowest Subgroup ID" |
| What that means on a transport | §7.2: "The definition of \"scheduled to be sent first\" in the algorithm is implementation dependent and is constrained by the prioritization interface of the underlying transport" |
| Between subscriptions that are alike | §7.2: "This algorithm does not provide a well-defined ordering for objects that belong to different subscriptions or FETCH responses, but have the same subscriber and publisher priority ... the expectation is that all subscriptions will be able to send some data" |
| Control | §7.2: "the control streams SHOULD be prioritized highest, followed by the bidi request streams and then all subscribed Objects" |
| A relay | §7.3: "For downstream subscriptions, relays SHOULD respect the subscriber and original publisher's priorities", and "SHOULD NOT directly use Subscriber Priority or Group Order from incoming subscriptions for upstream subscriptions" |

Who does what, before this change:

| | Publisher Priority | Subscriber Priority | Scheduling |
|---|---|---|---|
| The publishing page (`@xmoq/media`) | None in its subgroup headers, video and sound alike, and no default on the track: 128 for both | | |
| The watching page | | None in its SUBSCRIBE | |
| `xmoq-watch` | | 128 for every track | |
| The own relay | Passed on in the header as it came | Not read | **None: every stream to a subscriber has the transport's one priority**, and the transport takes them in turn |
| `moqtail`'s relay (tag `relay@0.15.0`, `apps/relay/src/server/subscription.rs`, read on 2026-10-12) | | | Sets the priority of each stream it opens from Subscriber Priority, Publisher Priority and the Group (`compute_stream_priority`): 65,536 bands, one for each pair of the two priorities, and within a band the Group modulo 65,536 by the Group Order |

So on the narrow link nothing puts the sound before the video: both are 128 and the relay does not look.

## 3. Expected of a relay that schedules by priority, written before the runs

What is built: the relay gives each stream it opens to a subscriber a priority of the transport from the subscription's Subscriber Priority, the stream's Publisher Priority and its Group, in the order of §7.2; and the sound's publisher (`@xmoq/media`) gives its streams a higher Publisher Priority than the video has.

- **Less gain than hoped for is what is expected.** The sound comes 65 to 105 ms late where the video comes 300 ms and more late: the sound is not stuck behind the relay's own backlog of video, of which the transport already gives each stream its turn. What it waits in is mostly the link's queue of 100 ms, which holds packets that have left the relay and that no relay can put in another order.
- So: **half of the pieces within 40 to 70 ms of the fastest, not within 10 to 20** as with the small version on the link; a page with a buffer of 50 ms still loses pieces, fewer than three of four. The video the same as before (the sound is some 30 kbit/s of 2,000).
- If instead the sound comes within 20 ms, the relay's queue was the larger part after all, and the priority is worth having by default.

## What was built

- `SendStream::set_send_order` in `xmoq-moqt-io` (quinn's stream priority behind it: "Locally buffered data from streams with higher priority will be transmitted before data from streams with lower priority", quinn 0.11.12), and `send_order` in `xmoq-relay-core`: the number of a stream towards a subscriber from the Subscriber Priority of its SUBSCRIBE (128 without one) and the stream's Publisher Priority (its header's, or the default of its source, or 128). **Every number is below 0**, which the control stream and the request streams have, so those go first (§7.2).
- **§7.2's third and fourth steps, the Group by the Group Order and the Subgroup ID, are not taken.** A first version had the Group in the number, as `moqtail`'s relay has. The transport knows one number for a stream and not whose it is, so the Group would have ordered the streams of different tracks too: the demo's sound has five Groups a second and its video one, both counted from the time of day, so with the older Group first the video's are older for good, and on a link with less room than both the sound would not be sent. It was taken out before the runs.
- `publisherPriority` on the audio publisher of `@xmoq/media` (none when left out, as before), and `/publish?audioPriority=N` to set it.

## Result

The same link and picture as above; the relay with the change.

**A page** (two runs each, 1,000 pieces counted):

| The sound's Publisher Priority | Buffer | Pieces sounded | Too late | A piece came after the fastest, ms (half; 95 of 100) | Frames shown of 600 |
|---|---|---|---|---|---|
| none (128, as the video) | 50 ms | 259, 253 | 740, 746 | 62 and 68; 97 and 97 | 425, 428 |
| none | 150 ms | 1,002, 1,000 | 0, 0 | 65 and 64; 94 and 93 | 416, 420 |
| 64 | 50 ms | 288, 232 | 709, 771 | 62 and 70; 89 and 96 | 420, 420 |
| 64 | 150 ms | 1,002, 1,002 | 0, 0 | 71 and 62; 98 and 94 | 413, 427 |
| **200, below the video** | 150 ms | 136 | 142 | 2,844; 14,418 | 260 |

**The terminal viewer** (`--buffer-ms 50`, three runs each, about 1,200 packets):

| The sound's Publisher Priority | Left out as too late | Sound played, of 24 s | A packet came after the fastest, ms (half; 95 of 100) |
|---|---|---|---|
| none | 891, 979, 322 | 6.2, 4.3, 17.3 s | 75, 81, 48; 98, 107, 72 |
| 64 | 307, 870, 235 | 17.8, 6.8, 19.2 s | 45, 72, 43; 71, 97, 69 |

- **The priority is acted on**: with the sound below the video the relay sends the video first and the sound hardly at all (half of the pieces 2.8 s late). That is the check that the number reaches the transport, from the page's header through the relay.
- **A higher priority for the sound does not keep it in time, as expected, and less than expected: nothing at all on the page.** With 64 the pieces come 62 to 71 ms after the fastest, as without. On the terminal viewer two of three runs with 64 are better than two of three without, and the third of each is the other way round: the runs differ by more than the priority does.
- **The sound waits in the link's queue, not in the relay.** A relay puts in order what it has not sent; what stands between it and the viewer is 100 ms of packets that are on their way. With the queue at 20 ms in place of 100 (two runs, buffer 50 ms) half of the pieces come within 35 to 41 ms, 339 to 356 are too late in place of 740 to 771, and the video falls apart (91 and 98 of 600 frames), since the link now drops what it held.
- With the same priorities the relay is as it was: the rows without a priority are the rows of the first table again.

`mise run check` and `mise run interop` passed with the change (the numbers are in the pull request).

## What this says, and what it does not

- **On a link narrower than the video the sound is late by the link's queue, for the page as for the terminal viewer, and the default buffer of 50 ms does not cover it.** 150 ms did here, with a queue of 100 ms. That, and not the relay's order, is what the 766 and 1,516 dry runs were.
- **A relay that schedules by priority is what the draft asks (§7.2, SHOULD), and it does not mend this.** It would where the relay itself holds a backlog that the sound stands behind, which this relay does not have between two subscriptions of one viewer: the transport already gave each stream its turn.
- What would mend it is on the viewer's side or the sender's: a buffer for the sound that follows how late the sound comes (the automatic buffer does, and the terminal viewer played 23.4 of 24 s with it), a sender that keeps the link's queue short (a congestion control that does, or a video that fits: the small version), or less in the queue (a shorter timeout does not shorten it).
- Not known: a real network, whose queues are not one FIFO of 100 ms; loss; many subscribers behind one relay, where a relay does hold a backlog.

## What an automatic buffer by default would cost: what the records have, and what they lack

Nothing here was run anew. The automatic buffer (`/watch?buffer=auto`, `xmoq-watch --buffer-ms auto`; `fixtures/playout/` has the rule both go by) holds what 99 of 100 Objects of the last 10 seconds came within, and 10 ms more, from 50 ms where it begins up to 400 ms.

| Link | Where it stands | What it costs or gains against the fixed 50 ms | Record |
|---|---|---|---|
| No link, or 50 ms each way with no loss | 40 ms (the rule's least there) | Nothing that was measured: a frame is shown 27 ms after it was made with no link and 79 to 81 ms at 50 ms each way, where a fixed 150 ms has 147 and 192 to 196 ms; no break in the sound, and 175 pieces played slower or faster in 20 s where the fixed buffer has 12. The fixed 50 ms on that link is not in the record | [delay](2026-10-11-where-the-delay-of-the-demo-comes-from.md) |
| 50 ms each way, 1% loss | 110 to 140 ms, moving | 6 to 10 breaks in the sound in a run and 7 to 20 pieces too late, where a fixed 150 ms leaves 1 break and none too late; the picture 162 to 175 ms after it was made against 197: the buffer covers 99 of 100 and the hundredth is the break | [delay](2026-10-11-where-the-delay-of-the-demo-comes-from.md) |
| 50 ms each way, 5% loss | 360 to 400 ms | 0 to 5 breaks and the picture about 460 ms after it was made, where a fixed 150 ms has 23 to 41 breaks and 215 ms | [sound-following buffer](2026-10-12-a-timeout-and-a-buffer-that-follow-the-sound.md) |
| 2 Mbit/s with a video of 2.5 | **400 ms, the limit, throughout**, page and terminal viewer | The sound whole (the terminal viewer plays 23.1 to 23.5 of 24 s, against 8 to 17 s with 50 ms), and picture and sound together about 480 ms behind. A fixed 150 ms on the page, or 300 ms in the terminal, plays all of the sound with less delay, and has the picture come after the sound | [sound-following buffer](2026-10-12-a-timeout-and-a-buffer-that-follow-the-sound.md), [terminal](2026-10-12-the-terminal-viewer-has-the-pages-automatic-buffer.md) |

- **Where the link is good the automatic buffer costs nothing that was measured, and where it loses packets it is better than 50 ms and worse than a fixed 150 ms by a few breaks.** On the narrow link it keeps the sound and puts everything nearly half a second behind, because it follows the video's lateness too; the one that follows the sound alone stood at 110 ms there and let picture and sound be 230 to 270 ms apart, and was not taken.
- **What is lacking for a decision**:
  - the fixed 50 ms on the links with loss, run beside the automatic one (the record of 1% has 150 ms and automatic; the default itself was not in it);
  - how the buffer moving sounds: pieces are played a twentieth slower or faster while it changes (27 to 330 packets a run in the terminal viewer), and nobody listened to that;
  - the first seconds: the buffer begins at 50 ms and grows when 50 Objects have come, and the terminal viewer's underruns with it (25 to 56 a run) are thought to be of that beginning, which was not written down by the second;
  - a link that is good and then bad and good again: how long the buffer stays large after the bad time (it shrinks 10 ms twice a second, so 400 ms take 18 s to come back to 40);
  - a real network.
