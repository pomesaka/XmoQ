# One, five or ten pieces of sound a Group, measured again

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: the open decision on how many chunks of audio share a Group and a stream (`DEFAULT_AUDIO_CHUNKS_PER_GROUP`, 10). Material for it; the default is not changed here.

## Objective

Ten chunks of Opus share a Group and a stream since 2026-10-11, decided on a measurement that did not hold ([cause](2026-10-11-sound-falls-behind-on-a-link-with-delay-and-loss.md)). Since the player was mended there was one run of each. With 1, 5 and 10 chunks a Group, several runs each: does the sound differ, by more than one run differs from the next?

Expected, written before the runs: no difference without loss; with loss, a lost packet holds back the rest of its stream, so 10 a Group has pieces up to 200 ms late where 1 has the one piece late, which shows with the short buffer and less with the long one; on a link too narrow for the video the sound is late with all three; the relay has more to do with 50 streams a second than with 5.

## What the drafts say

MSF-01 §6: "Irrespective of the Section 4 in place, each MOQT Object MUST be mapped to a new MOQT Stream." LOC-04 §4.1, of its example with one audio track: "there is one MOQT Object per MOQT Group, where the GroupID in the object header is increment by one for each encoded audio chunk and the ObjectID is defaulted to value 0", sent "over an individual unidirectional QUIC stream". So 1 is what both have; 5 and 10 leave the MUST of MSF, as the video's Group on one stream does (`packages/media/README.md` has where that sentence stood in the working group).

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Viewer | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the watching page with a buffer of 50 and of 150 ms, the sound put in order (`holdForOrderMs`, the buffer less 20 ms) |
| Relay | own, Cubic; for the last link also with `--delivery-patience-ms 500` |
| Link | `spikes/lossy-link/proxy.mjs` between viewer and relay, 50 ms each way: no loss; 1% and 5% loss each way; `rate` 2,000 kbit/s with `queue` 100 ms |
| Sent | Opus, 20 ms a piece, 50 a second, `/publish?audioGroup=1`, `5` and `10`; with it the demo's video, 640x360 at about 450 kbit/s, and for the narrow link `?picture=busy&bitrate=2500000` (about 2.65 Mbit/s), so that the link is narrower than what is sent |

Each run: 15 s (750 pieces), after 2.5 s of playing. With loss and without, the two buffers watched one broadcast at once, each on a connection of its own; on the narrow link one viewer at a time. "Came after" is from the time a piece carries to when it reached the player, after the wait for order. In brackets the mean.

## Result

| Link | Buffer | Pieces a Group | Runs | Too late for the loudspeaker | Dropped as late | Breaks | Held for order | Came after, ms: median | 95th | most |
|---|---|---|---|---|---|---|---|---|---|---|
| no loss | 50 ms | 1 | 3 | 0 (0) | 0 (0) | 0 (0) | 0 to 2 (1) | 55 to 60 | 64 to 67 | 76 to 81 |
| no loss | 50 ms | 5 | 3 | 0 (0) | 0 (0) | 0 (0) | 0 (0) | 60 | 63 to 64 | 68 to 71 |
| no loss | 50 ms | 10 | 3 | 0 (0) | 0 (0) | 0 (0) | 0 (0) | 53 to 60 | 62 to 64 | 64 to 69 |
| no loss | 150 ms | 1 | 3 | 0 (0) | 0 (0) | 0 (0) | 0 to 1 (1) | 55 to 60 | 64 to 67 | 76 to 81 |
| no loss | 150 ms | 5 | 3 | 0 (0) | 0 (0) | 0 (0) | 0 (0) | 60 | 63 to 64 | 69 to 71 |
| no loss | 150 ms | 10 | 3 | 0 (0) | 0 (0) | 0 (0) | 0 (0) | 53 to 60 | 62 to 64 | 64 to 69 |
| 1% loss | 50 ms | 1 | 5 | 8 to 46 (24) | 5 to 8 (7) | 19 to 44 (30) | 16 to 39 (24) | 54 to 58 | 85 to 106 | 118 to 157 |
| 1% loss | 50 ms | 5 | 5 | 18 to 111 (48) | 7 to 26 (16) | 16 to 56 (34) | 26 to 77 (44) | 57 to 61 | 83 to 142 | 178 to 249 |
| 1% loss | 50 ms | 10 | 5 | 8 to 72 (52) | 0 to 12 (8) | 1 to 42 (29) | 2 to 40 (21) | 52 to 61 | 63 to 122 | 173 to 213 |
| 1% loss | 150 ms | 1 | 5 | 0 to 8 (3) | 0 to 2 (1) | 2 to 3 (2) | 22 to 96 (59) | 57 to 61 | 94 to 153 | 187 to 276 |
| 1% loss | 150 ms | 5 | 5 | 0 to 8 (2) | 0 to 1 (0) | 0 to 6 (1) | 10 to 56 (32) | 55 to 60 | 81 to 142 | 179 to 260 |
| 1% loss | 150 ms | 10 | 5 | 0 to 12 (3) | 0 to 1 (0) | 0 to 4 (1) | 11 to 41 (27) | 52 to 60 | 103 to 157 | 178 to 295 |
| 5% loss | 50 ms | 1 | 3 | 283 to 388 (338) | 17 to 34 (23) | 140 to 148 (144) | 165 to 226 (188) | 89 to 107 | 167 to 183 | 288 to 296 |
| 5% loss | 50 ms | 5 | 3 | 152 to 338 (255) | 40 to 58 (50) | 65 to 132 (108) | 111 to 247 (181) | 84 to 98 | 164 to 173 | 192 to 225 |
| 5% loss | 50 ms | 10 | 3 | 379 to 449 (416) | 43 to 62 (51) | 124 to 144 (133) | 146 to 174 (160) | 105 to 118 | 181 to 194 | 296 to 332 |
| 5% loss | 150 ms | 1 | 3 | 34 to 47 (42) | 2 to 11 (6) | 13 to 19 (16) | 182 to 247 (217) | 92 to 107 | 193 to 216 | 284 to 411 |
| 5% loss | 150 ms | 5 | 3 | 21 to 84 (44) | 4 to 42 (18) | 14 to 38 (22) | 128 to 277 (221) | 86 to 121 | 187 to 235 | 287 to 346 |
| 5% loss | 150 ms | 10 | 3 | 31 to 67 (53) | 8 to 14 (11) | 11 to 17 (14) | 133 to 168 (155) | 98 to 110 | 195 to 262 | 294 to 565 |
| 2 Mbit/s | 50 ms | 1 | 3 | 496 to 557 (524) | 0 (0) | 96 to 100 (98) | 11 to 14 (13) | 127 to 132 | 155 to 161 | 167 to 183 |
| 2 Mbit/s | 50 ms | 5 | 3 | 562 to 585 (575) | 17 to 21 (20) | 88 to 107 (95) | 84 to 100 (93) | 139 to 147 | 181 to 186 | 205 to 211 |
| 2 Mbit/s | 50 ms | 10 | 3 | 587 to 618 (606) | 4 to 5 (5) | 73 to 78 (75) | 41 to 56 (48) | 144 to 149 | 184 to 190 | 205 to 212 |
| 2 Mbit/s | 150 ms | 1 | 5 | 0 (0) | 0 (0) | 0 (0) | 6 to 14 (11) | 126 to 128 | 154 to 158 | 163 to 171 |
| 2 Mbit/s | 150 ms | 5 | 5 | 0 to 8 (2) | 0 to 5 (1) | 0 to 2 (1) | 90 to 108 (99) | 140 to 147 | 182 to 190 | 203 to 296 |
| 2 Mbit/s | 150 ms | 10 | 5 | 3 to 12 (8) | 0 to 7 (3) | 0 to 3 (2) | 42 to 53 (50) | 145 to 148 | 185 to 191 | 264 to 399 |
| 2 Mbit/s, patience 500 ms | 50 ms | 1 | 3 | 411 to 534 (479) | 0 (0) | 93 to 131 (107) | 6 to 11 (8) | 128 to 131 | 158 to 159 | 169 to 175 |
| 2 Mbit/s, patience 500 ms | 50 ms | 5 | 3 | 533 to 585 (558) | 0 (0) | 60 to 95 (82) | 56 to 65 (60) | 134 to 139 | 163 to 166 | 177 to 181 |
| 2 Mbit/s, patience 500 ms | 50 ms | 10 | 3 | 443 to 591 (529) | 0 (0) | 64 to 123 (91) | 20 to 27 (24) | 130 to 139 | 161 to 167 | 173 to 179 |
| 2 Mbit/s, patience 500 ms | 150 ms | 1 | 5 | 0 (0) | 0 (0) | 0 (0) | 7 to 15 (11) | 123 to 133 | 152 to 159 | 165 to 177 |
| 2 Mbit/s, patience 500 ms | 150 ms | 5 | 5 | 0 (0) | 0 (0) | 0 (0) | 54 to 63 (59) | 131 to 138 | 161 to 167 | 173 to 181 |
| 2 Mbit/s, patience 500 ms | 150 ms | 10 | 5 | 0 (0) | 0 (0) | 0 (0) | 22 to 37 (29) | 130 to 139 | 163 to 167 | 178 to 181 |

In one run at 5% loss with 5 a Group both viewers received 462 and 466 pieces of 750; why was not looked into, and its numbers are in the rows as they came.

- **Without loss the three are the same**: every piece sounded, no break.
- **At 1% loss the three cannot be told apart.** With the buffer of 150 ms: 0 to 8, 0 to 8 and 0 to 12 pieces too late, 0 to 6 breaks, the ranges inside one another. With 50 ms the means lean to 1 a Group (24 too late against 48 and 52), and the runs overlap (8 to 46 against 8 to 72): five runs do not settle it.
- **At 5% loss and the buffer of 50 ms, 1 a Group lost fewer** (283 to 388 too late against 379 to 449 with 10; three runs, the ranges overlapping by one run). With 150 ms the three overlap again.
- **On the narrow link, 1 a Group was better in every run**, with the buffer of 150 ms: no piece too late and no break in five of five, where 10 a Group had 3 to 12 too late in five of five; a piece came 127 ms after its time against 146 (median) and 157 against 189 (95th), and the latest 171 ms against 399. With the relay's patience none of the three had a piece too late, and 1 was still about 8 ms sooner. With the buffer of 50 ms most of the sound is too late with all three, as the video takes the link.
- **What 1 a Group costs** (three viewers, no link, 20 s, two runs): the relay took 10.3% of one core where it took 7.2 to 7.5% with 5 and 6.6 to 7.2% with 10 (a debug build); the publishing page's browser 12 to 13%, against 9 to 12% and 9 to 11%. No stream waited to be opened with any of the three (`DATA_OPENED after_ms` 0 for all but 15 of 9,470 streams, 4 ms at most).
- **Held for order** is how many pieces came after a gap and waited: more with 5 than with 1 or 10 on the narrow link, and it follows no simple rule; it is not what is heard.

## What this says, and what it does not

It says that **nothing measured here speaks for 10**, which was the question the earlier record left open: 1 a Group was never worse by more than a run differs from the next, it was better in every run where the link was narrower than what is sent, and at 5% loss with the short buffer. It is also what MSF and LOC have. Against it stands a relay that has about a half more to do for the same viewers on one machine, and ten times the streams. It does not say that 1 is better at 1% loss, where five runs could not tell the three apart, nor anything about a real network, where losses come in bursts and a burst takes a whole stream of ten at once. 5 was in no row better than both others.

## Not done

- More viewers (the relay's limits are counted in streams: 256 waiting for a viewer), a relay built for release, and 50 streams a second for an hour.
- A real network; loss in bursts; BBR.
- Listening. A piece too late is a gap of 20 ms; whether three gaps in 15 s are heard was not asked of anybody.
