# A FETCH from the relay's cache that is read slowly

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer (the relay's limits)

## Objective

The known gaps have had, among those that can do the most harm, that a fetch stream the relay writes from its cache is bounded by no time as a whole: a viewer that reads its FETCHes slowly keeps the Objects of the Group it is being sent alive after the cache has moved on, up to 8 MiB for each, for as long as it reads. It was derived from the code and never run. This run measures it, and what the relay's other limits do about it.

## What the draft says

- No parameter bounds how long the answer to a FETCH may take to be read. The delivery timeouts are of subscriptions: §8, "Each MOQT subscription has two timeout values associated with it: a SUBGROUP_DELIVERY_TIMEOUT and an OBJECT_DELIVERY_TIMEOUT", and both parameters "MAY appear in a PUBLISH_OK, SUBSCRIBE, or REQUEST_UPDATE message" (§10.2.3, §10.2.4), not in a FETCH. FILL_TIMEOUT (§10.2.5) is about a relay waiting for upstream sources.
- A publisher may give a fetch stream up: §3.3.3 has the Stream Reset Error Codes "when resetting or sending STOP_SENDING on any stream", among them `CANCELLED` (0x1), "The stream was cancelled by either endpoint", and `EXCESSIVE_LOAD` (0x9), "The endpoint is overloaded and is resetting this stream". `TOO_FAR_BEHIND` (0x5) is worded for a subscription ("The corresponding subscription has exceeded the publisher's resource limits"), and `DELIVERY_TIMEOUT` (0x2) for "A delivery timeout (Section 8)", which a FETCH has none of.
- §10.13: "A subscriber keeps FETCH state until it cancels the request (see Section 3.3.2), receives REQUEST_ERROR, or the FETCH data stream receives a FIN or is reset."

## What the relay has today (read in the code)

- It answers a FETCH from its cache where the range lies in the latest Group of the track, which is 8 MiB and 4,096 Objects at most, and writes the fetch stream itself, Object by Object.
- A subscriber that takes nothing of the stream, or of one Object, for 10 seconds has the stream reset. One that takes each Object within 10 seconds is never cut, however long the whole takes.
- A session has 16 FETCHes open at most. The 4 for one session on a publisher are of FETCHes that are passed on, and do not count these.
- What ends a slow subscription (`TOO_FAR_BEHIND`, the bytes and Objects that wait for a session) is about subgroup streams.

## What was built for the measurement

`xmoq-quiet-peer --connect <url> --cert-sha256 <hex> --stay-ms <n>` with one of:

- `--publish-group <objects>x<bytes>`: publishes the track `big` of `(xmoq)` (PUBLISH) and sends Group 0 with that many Objects of that many bytes on one subgroup stream. With `--next-group-after-ms <n>` it sends Group 1, one small Object, that much later, so that the relay's cache moves on.
- `--fetch-cached <n> --read-bytes-a-second <n>`: sends that many Standalone FETCHes for Group 0 of `big` and reads each fetch stream at that rate (0: not at all). It says what it has read every 5 seconds and how each stream ended.

## Expected, written before the runs

One publisher with a Group of 8 MiB (64 Objects of 128 KiB), a reader with 16 FETCHes, and Group 1 five seconds later.

| Reader | Expected |
|---|---|
| Reads nothing | Each stream is reset about 10 s after the relay could write no more of it (the reader's flow control is full after some hundreds of kB). The relay's memory goes back to what it was once Group 1 has come and the streams are reset |
| Reads 16 KiB a second on each stream (an Object of 128 KiB in 8 s) | No stream is reset. The relay's memory stays about 8 MiB above what it has after Group 1 for as long as the reader goes on (the 16 streams share the Objects of the one Group; 16 tracks would be 128 MiB), which would be 8.5 minutes for the whole Group, and is watched for 90 s here |
| Reads 16 KiB a second, and the cache has 4,096 Objects of 2 KiB | By the rule, each Object may take 10 s, so the 8 MiB could be held for 11 hours; not run for that long |

A session that asks for 16 different tracks would hold 16 times 8 MiB. That needs 16 publishers of such a Group or one with 16 tracks, and is not run: the tool publishes one track.

## Environment

Apple M3 Pro, macOS, loopback. Release builds of `xmoq-relay` and `xmoq-quiet-peer`. One run for each line. The relay's memory is its resident size from `ps`, which on this system does not go down when memory is freed; when the relay stopped writing a stream is read from its log (`FETCH_WRITE_ENDED`).

## Observed results

A Group of 8 MiB (64 Objects of 128 KiB), 16 FETCHes of one session for it, and Group 1 five seconds later, after which the cache has Group 1 and no longer Group 0.

| Reader | The relay's writes of the 16 fetch streams | Relay's memory: alone, with the Group, while it writes |
|---|---|---|
| Reads nothing | all 16 given up 10 s after they began: `not_taken pieces=9 code=0x5` | 5.4, 17.0, 31 MB |
| Reads 16 KiB a second on each stream | **none ended in the 90 s the reader stayed**; it had read 23.0 MB of the 134 MB by then. They ended when it left (`stopped pieces=20`) | 5.5, 16.9, 30 to 39 MB |

- **As expected**: a reader that takes nothing is cut after 10 s, and one that takes each Object in time is not cut. Nothing else of the relay ends such a stream: its other limits are of subgroup streams. The Objects of Group 0 were held by the 16 writes for as long as the reader read, 85 s after the cache had moved on; at that rate the whole takes 512 s for a stream, and with 4,096 small Objects it could be 11 hours.
- **The memory was not measured as such.** The 16 streams share the one Group's Objects, so what they keep alive is the 8 MiB of a Group the cache has dropped; the resident size shows the 12 to 20 MB the 16 writes took on top (an Object is copied to be encoded, and the windows of the streams), and does not come down afterwards.
- **A stream that fits into the windows of the connection is not written slowly at all**: a Group of 1 MiB was written whole at once (`finished pieces=64`) while the reader took 16 KiB a second. Its bytes wait in the connection, which has its own bound. The time for the whole can only act on what the relay still has to write.

## What was changed

A fetch stream the relay writes from its cache has a time for the whole, from when the relay begins it: 30 seconds, or as long as its Objects take at 500 kbit/s where that is longer (134 s for 8 MiB). After that the stream is reset with `TOO_FAR_BEHIND` (0x5), the code of a stream that is not taken (`FETCH_WRITE_ENDED ... too_slow`). `--cached-fetch-whole-ms` and `--cached-fetch-least-kbps`; provisional. A fetch stream that is passed on from a publisher is not changed.

| Run, with the limit | Outcome |
|---|---|
| 16 FETCHes read at 16 KiB a second, the relay's own values | all 16 reset 134 s after they began (`too_slow`), and the reader saw every stream end |
| The same with the least rate at 1,000 kbit/s (a first try) | all 16 reset after 67 s: `too_slow pieces=16 code=0x5` |
| One FETCH of the 8 MiB read as fast as a link of 2 Mbit/s lets it (`spikes/lossy-link/proxy.mjs`, 50 ms each way) | the reader had all 8,388,869 bytes and a FIN after 34.7 s; the limit is 134 s, 3.9 times that |
| One FETCH of 320 kB (40 Objects of 8 KiB), the same link | all of it after 1.7 s; the limit is 30 s |
| Interop run `relay-slow-cached-fetch` (the time for the whole set to 3 s), twice by itself | the slow reader's stream reset `too_slow pieces=9 code=0x5`, and a reader that read got all 8,388,869 bytes and a FIN |

**How many FETCHes from the cache a session may have open** (not changed; 16). With the time for the whole, one session can still have 16 Groups of 8 MiB kept alive at a time, 128 MiB, by asking for the newest Group of 16 tracks, or of one track as each comes, and beginning anew every 134 s. With 4, as for FETCHes that are passed on, it would be 32 MiB a session. Neither bounds the relay: 256 sessions are 32 GiB and 8 GiB by that count, against the 256 MiB the cache itself may take. What would bound it is counting the bytes that such writes keep alive beyond the cache against a limit of the relay, and refusing a FETCH from the cache with `EXCESSIVE_LOAD` beyond it. Neither was built.

## Not run

- 16 tracks, so that 16 Groups are kept alive at once: the tool publishes one track.
- A cache with 4,096 small Objects, and a reader for hours.
- More than one session that reads slowly.
- A viewer on a link of less than 500 kbit/s, which is cut where it asks for a Group larger than 1.9 MB (30 s at that rate) and takes it slower than that rate.
- The demo and the terminal viewer on a narrow link with this limit, beyond the interop run, which passed.

## Next action

The time for the whole is in; its two numbers are provisional (`docs/open-questions.md`). How many FETCHes from the cache a session may have, and a bound on what such writes keep alive for the relay as a whole, are reported and not built.
