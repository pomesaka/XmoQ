# A session that asks for a new track again and again

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer (the relay's limits)

## Objective

§13.1 of draft-ietf-moq-transport-18 is about a subscriber: "A malicious subscriber could attempt to overwhelm a publisher or relay by requesting subscriptions to many tracks simultaneously. Relays SHOULD implement rate limiting on subscription requests and MAY reject excessive subscriptions with REQUEST_ERROR using the EXCESSIVE_LOAD error code." The own relay has no such limit. A SUBSCRIBE for a track that has no source makes it send its own SUBSCRIBE to every session that announced a namespace of the track, so a session that asks for another track name each time has the relay ask those publishers each time. The record of the session that announces again and again measured the other way to the same load; this one measures this way, and the cost to the publishers that are asked.

## What was built

`xmoq-quiet-peer --connect <url> --cert-sha256 <hex> --stay-ms <n>` with one of:

- `--ask-per-second <n> --ask subscribe|fetch-nothing|fetch-old [--rendezvous-ms <n>] [--cancel yes]`: that many requests a second, each for a track name not used before (`subscribe`: a SUBSCRIBE for `s0`, `s1`, ... of `(xmoq)`, with a RENDEZVOUS_TIMEOUT where the flag gives one; `fetch-nothing`: a Standalone FETCH for Group 0 of such a track; `fetch-old`: a Standalone FETCH for Group 0 of the track `a`, which a publisher beside it publishes). With `--cancel yes` the request is reset (CANCELLED) when it is answered or 50 ms after it was sent, whichever is first. At the end it prints what it counted, by the Error Code of the answers.
- `--announce-and-refuse yes`: a publisher that has nothing. It announces `(xmoq)` once, refuses every SUBSCRIBE and FETCH the relay sends it with `DOES_NOT_EXIST`, and prints how many it was sent.

The script that ran the pieces and took the numbers was a throwaway and is not in the repository: the relay, a publisher and a subscriber of `xmoq/a` (own Rust clients; an Object every 100 ms, whose delay is taken from the line on the publisher's stdin to the subscriber's `OBJECT` line), one or four sessions that announce and refuse, and the session that asks, for 10 s.

## Expected, written before the runs

What the relay has that bounds this: a session has 256 requests open at most (the Rust session; one more gets `EXCESSIVE_LOAD`); the relay has 64 requests of its own open towards one session, and passes a publisher over that has that many; a publisher gets 5 s for its answer; 16 SUBSCRIBEs are held for a session and 1,024 tracks in all; a session has 16 FETCHes open, and the relay 16 of its own towards one publisher.

| Asked | Expected |
|---|---|
| SUBSCRIBE for a new name, no RENDEZVOUS_TIMEOUT, not cancelled, 1 publisher that refuses | One SUBSCRIBE of the relay to the publisher for each: as many a second as are asked. The publisher refuses at once, so nothing piles up and no limit is reached. Every request is answered `DOES_NOT_EXIST` with the publisher's Retry Interval (0). The relay's CPU about as for the announcements with 2 tracks: 0.1 of a core at 100 a second, 0.5 to 1 at 1,000. The Objects beside it are not late |
| The same with 4 publishers | Four SUBSCRIBEs of the relay for each request: 4,000 a second at 1,000, and each publisher is sent as many as are asked. The relay near a whole core at 1,000 a second |
| With a RENDEZVOUS_TIMEOUT of 20 s, not cancelled | The first 16 are held when the publishers have refused, and stay; every one after is answered `DOES_NOT_EXIST` when the publishers have refused. The publishers are asked for each all the same. The cost as without the parameter |
| Cancelled 50 ms after it was sent | As not cancelled: the answer comes sooner than the 50 ms |
| FETCH for a track nobody has | Passed on to the publishers that announced, as a SUBSCRIBE is; each refuses, and the requester gets `DOES_NOT_EXIST`. No more than 16 are open at once for the session, which is not reached while the answers come at once |
| FETCH for Group 0 of a track that is published | Answered from the relay's cache where the Group is the latest, and else passed on to the publisher of the track; the 16 of the session and the 16 towards the publisher are reached at 100 and 1,000 a second if the publisher is slow, and the rest get `EXCESSIVE_LOAD`. The subscriber beside it is not disturbed |

## Environment

Apple M3 Pro (11 cores), macOS, loopback. Release builds of `xmoq-relay`, `xmoq-moqt-client` and `xmoq-quiet-peer`. The relay's log went to a file. One run of 10 s for each line. "Publishers" are the sessions that announced `(xmoq)`: the own Rust publisher of `xmoq/a`, which announces before it publishes and refuses what it is asked for beyond its track, and with 4 three sessions more that announce and refuse.

## Observed results

**A session that cancels each request when it is answered** (or 50 ms after it sent it), which is what a client does that goes on asking:

| Asked | Publishers | A second | Answered | Requests the relay sent upstream | Each publisher was sent | Relay's CPU in 9.5 s | Delay of the Objects beside it (p50 / p95 / max) | Relay's memory |
|---|---|---|---|---|---|---|---|---|
| SUBSCRIBE | 1 | 100 | 1,000 `DOES_NOT_EXIST` | 1,000 | 1,000 | 0.42 s | 0.5 / 0.7 / 1.0 ms | 11.5 MB |
| SUBSCRIBE | 1 | 1,000 | 9,998 `DOES_NOT_EXIST` | 9,999 | 9,999 | 1.94 s | 0.5 / 0.7 / 0.8 ms | 12.0 MB |
| SUBSCRIBE | 4 | 100 | 1,000 `DOES_NOT_EXIST` | 4,003 | 1,000 | 1.18 s | 0.7 / 1.2 / 1.4 ms | 12.4 MB |
| SUBSCRIBE | 4 | 1,000 | 9,999 `DOES_NOT_EXIST` | 40,003 | 10,000 | 4.19 s | 0.5 / 0.7 / 1.1 ms | 12.8 MB |
| SUBSCRIBE with RENDEZVOUS_TIMEOUT 20 s | 4 | 1,000 | 6,877 `DOES_NOT_EXIST`, 3,122 cancelled before an answer | 39,999 | 9,999 | 3.77 s | 0.4 / 0.7 / 0.8 ms | 13.8 MB |
| FETCH, a track nobody has | 4 | 1,000 | 9,999 `DOES_NOT_EXIST` | 10,004 | 10,001 the publisher of `xmoq/a`, 1 each of the others | 1.77 s | 0.5 / 0.7 / 1.0 ms | 12.6 MB |
| FETCH, Group 0 of `xmoq/a` | 1 | 100 | 1,000 `DOES_NOT_EXIST` | 1,000 | 1,000 | 0.41 s | 0.5 / 0.8 / 1.1 ms | 11.7 MB |
| FETCH, Group 0 of `xmoq/a` | 1 | 1,000 | 9,999 `DOES_NOT_EXIST` | 10,000 | 10,000 | 1.84 s | 0.5 / 0.8 / 1.3 ms | 12.2 MB |

Without the session that asks, the relay took 0.02 s of CPU in 4.6 s, had 10.0 MB, and the delay was 0.9 / 1.6 / 2.7 ms.

**A session that leaves its request streams open** (answered or not):

| Asked | Publishers | A second | Answered | Requests the relay sent upstream | Relay's CPU in 9.5 s |
|---|---|---|---|---|---|
| SUBSCRIBE | 1 | 10 | 100 `DOES_NOT_EXIST` | 100 | 0.12 s |
| SUBSCRIBE | 1 | 100 | 264 `DOES_NOT_EXIST`, 736 never read | 264 | 0.20 s |
| SUBSCRIBE | 1 | 1,000 | 256 `DOES_NOT_EXIST`, 8 `EXCESSIVE_LOAD`, 9,737 never read | 256 | 0.08 s |
| SUBSCRIBE | 4 | 10 | 100 `DOES_NOT_EXIST` | 403 | 0.31 s |
| SUBSCRIBE | 4 | 100 | 264 `DOES_NOT_EXIST` | 1,059 | 0.51 s |
| SUBSCRIBE | 4 | 1,000 | 256 `DOES_NOT_EXIST`, 8 `EXCESSIVE_LOAD` | 1,027 | 0.09 s |
| SUBSCRIBE with RENDEZVOUS_TIMEOUT 20 s | 4 | 100 | 248 `DOES_NOT_EXIST`, 16 held | 1,059 | 0.47 s |
| FETCH, a track nobody has | 4 | 1,000 | 256 `DOES_NOT_EXIST`, 8 `EXCESSIVE_LOAD` | 259 | 0.06 s |
| FETCH, Group 0 of `xmoq/a` | 1 | 1,000 | 256 `DOES_NOT_EXIST`, 8 `EXCESSIVE_LOAD` | 256 | 0.12 s |

Against what was expected:

- **One SUBSCRIBE for a new name makes the relay send one SUBSCRIBE to each publisher that announced the namespace, as expected**: 4 for 1 with four of them, and each publisher is sent as many requests as the session asks. A publisher that is a page has to refuse 1,000 requests a second for as long as it goes on.
- **The relay's CPU is less than expected**: 0.2 of a core at 1,000 a second with one publisher and 0.44 with four, where a whole core was expected with four. About 0.1 ms for each request the relay sends upstream, as with the announcements.
- **The Objects beside it were not late**, as expected: the 95th percentile stayed at 0.7 to 1.2 ms.
- **Not expected: a session that leaves its request streams open is stopped after 264 requests.** The session's 256 open requests are counted until the peer ends its direction of the stream, also for a request that was refused; after that 8 more are refused with `EXCESSIVE_LOAD`, and the rest are not read, since the relay lets a client have no more streams open. So the limit that holds this way of asking is the one on open requests, and what it does not hold is a session that ends each request, which every row of the first table is.
- **With a RENDEZVOUS_TIMEOUT the first 16 are held and the rest refused, as expected**, and the publishers are asked for each. Cancelled after 50 ms, about a third are cancelled before the relay has refused them (it waits for the four publishers' answers).
- **A FETCH for a track nobody has goes to one publisher, not to all**: the relay asks one publisher for a FETCH, and it was the publisher of `xmoq/a` each time. Not as expected.
- **A FETCH for Group 0 of the track that is published is passed on to its publisher each time**, which refuses it (the Rust publisher serves no FETCH in that mode), and the requester gets `DOES_NOT_EXIST`. The 16 FETCHes of a session and the 16 towards a publisher were not reached, since each was answered at once. With a publisher that answers slowly they would be; that is the question of one subscriber taking every place, and was not run here.
- No limit of the relay was reached by the session that cancels: not the 64 requests towards a publisher, not the 5 s for an answer, not the 1,024 held tracks.

## A publisher that does not answer

The runs above had publishers that refuse at once, so the relay never had many requests open towards one. Here the publisher announces `(xmoq)`, publishes the track `live` with PUBLISH, and answers nothing the relay asks of it (`xmoq-quiet-peer --announce-and-stay-quiet yes`). The relay gives a publisher 5 s for an answer, and has 64 requests of its own open towards one session, 16 of them FETCHes.

**Expected, written before the runs:**

| Scene | Expected |
|---|---|
| A. One session sends 30 SUBSCRIBEs a second, each for a new track name of the namespace, and ends each when it is answered. Beside it a viewer sends one SUBSCRIBE a second for a track name of its own | Without the first session, each SUBSCRIBE of the viewer has the publisher asked, and is answered `TIMEOUT` 5 s later. With it, the 64 requests towards the publisher are taken after about 2 s and stay taken, one coming free and being taken again 30 times a second; the viewer's SUBSCRIBEs are answered `EXCESSIVE_LOAD` at once, nearly all of them, for as long as the first session goes on. The first session gets `TIMEOUT` for about 13 a second and `EXCESSIVE_LOAD` for the rest |
| A, and the viewer asks for the track `live`, which the same publisher publishes | SUBSCRIBE_OK at once, with and without the first session: the relay has the track and asks nobody for it |
| B. One session sends 10 Standalone FETCHes a second for Group 0 of `live`, and ends each when it is answered. Beside it a viewer sends one such FETCH a second | Without the first session, each FETCH of the viewer is passed on to the publisher and answered `TIMEOUT` 5 s later. With it, the 16 FETCHes the relay may have towards the publisher are taken after 2 s by the one session (which may have 16 open itself), and the viewer's FETCHes are answered `EXCESSIVE_LOAD` at once, nearly all of them |

**Result.** The same machine and builds; one run of each scene. The viewer asks once a second for 36 s, and the other session asks from second 9 to second 24. Each line of the viewer is one request: when it was sent, and what came back.

| Scene | The viewer's requests | Before the other session (4 to 5 requests) | While it asked (about 15 requests) | After it (7 requests) |
|---|---|---|---|---|
| A | SUBSCRIBE for a track name of its own | `TIMEOUT` 5.0 s later, each | **8 `EXCESSIVE_LOAD` at once**, 7 `TIMEOUT` 5.0 s later | `TIMEOUT` 5.0 s later, each |
| B | FETCH for Group 0 of `live` | `TIMEOUT` 5.0 s later, each | **12 `EXCESSIVE_LOAD` at once**, 2 `TIMEOUT` 5.0 s later | `TIMEOUT` 5.0 s later, each |

`TIMEOUT` after 5 s is the relay having asked the publisher, which is what a publisher that answered would have turned into an answer; `EXCESSIVE_LOAD` at once is the relay not asking anybody.

- **One session keeps the others from a publisher, as expected, with few requests**: 30 SUBSCRIBEs a second in A (450 sent: 123 `TIMEOUT`, 265 `EXCESSIVE_LOAD`, 62 unanswered at the end) and 10 FETCHes a second in B (151 sent: 29 `TIMEOUT`, 106 `EXCESSIVE_LOAD`). The relay opened 213 SUBSCRIBEs towards the publisher in A and 68 FETCHes in B, over the whole run.
- **Not "nearly all" in A, against the expectation: about half.** The 64 requests are taken within 2 s, and all run out 5 s after they were sent, nearly together; for the second or two it takes the session to take them again, the viewer's SUBSCRIBE finds one free. In B the 16 are taken again in under 2 s at 10 a second, and the viewer got one of them twice in 14 requests.
- **A track the publisher has published is not touched, as expected**: a viewer of `live` got SUBSCRIBE_OK after 204 ms before and after 14 ms while the other session asked (scene A). The relay has that track and asks nobody.
- **This is the open question of one subscriber taking every FETCH of a publisher, seen in a run**, and the same for the SUBSCRIBEs the relay sends for tracks that are not published yet. It lasts as long as the session goes on, and ends with it: the first requests after it were answered as before.
- **What a viewer sends, counted on the demo** (`bun demo/serve.ts` on ports of its own, headless Chromium, the request streams of the watching page's session in the relay's log): 5 in its first second, and none in the 9 s after, with `/watch`, with `&abr=1` (on loopback it changed no version) and with `&wait=1`. The terminal viewer was not counted; it sends a SUBSCRIBE for the catalog, the video and the sound.

**What would stop it, and what would not** (nothing was built):

- **A limit on how many requests a second a session sends (§13.1) does not stop this.** The one session needed 13 SUBSCRIBEs a second to keep 64 places taken, and 3.2 FETCHes a second for the 16; a viewer sends 5 requests in a second by itself. A rate that lets a viewer start lets this through.
- **A share of a publisher's places for one subscriber's session does.** For example: of the 64 requests the relay has towards a publisher, no more than 8 on behalf of one subscribing session, and of the 16 FETCHes no more than 4 (a session may have 16 FETCHes open today, which is all of a publisher's). A viewer of the demo has 3 tracks and so 3 such SUBSCRIBEs at once at most, and one or two FETCHes. One session then leaves 56 and 12 for the others, and it takes 8 sessions, or 4, to do what one does now; the relay takes 256 sessions, so many sessions of one sender are not stopped by it.
- The numbers are examples, not measured against a deployment.



## With a share of a publisher for one session

After the runs above the relay was given the share: of the 64 requests it has open on one publisher, 8 for one subscriber's session, and of its 16 FETCH requests, 4. The same scenes, the same machine, one run each. The viewer of scene B asks every 1.5 s here: at one a second and 5 s for an answer it has five FETCHes open by itself, which is more than its own share, and it was refused every fifth, before, while and after the other session asked (seen in the first run with the share, and not what the scene is about).

| Scene | The viewer's requests | Before the other session | While it asked | After it | The other session |
|---|---|---|---|---|---|
| A | 31 SUBSCRIBEs, one a second | `TIMEOUT` 5.0 s later, each | **`TIMEOUT` 5.0 s later, each: all passed on** | the same | 451 sent: 16 `TIMEOUT`, 426 `EXCESSIVE_LOAD` |
| B | 21 FETCHes, one every 1.5 s | `TIMEOUT` 5.0 s later, each | **`TIMEOUT` 5.0 s later, each: all passed on** | the same | 150 sent: 8 `TIMEOUT`, 138 `EXCESSIVE_LOAD` |

- **Every request of the viewer reached the publisher while the other session asked**, where 8 of 15 SUBSCRIBEs and 12 of 14 FETCHes had been refused. The other session had 8 SUBSCRIBEs, or 4 FETCHes, passed on at a time (16 and 8 in its 15 s, two shares), and the rest refused at once. The relay opened 60 requests towards the publisher in A and 36 in B, where it had opened 213 and 68.
- A viewer of the track `live` got SUBSCRIBE_OK after 207 ms before and 5 ms while the other session asked, as before.
- **The demo is not touched by it**: `bun demo/check.ts` passed ("THE DEMO PLAYS: picture and sound"), a watching page with `&abr=1` played, and the relay's log of those runs has no `EXCESSIVE_LOAD` in its 832 lines. On loopback the page changed no version, so the moment of a change was not seen; the terminal viewer and every other client are in the interop run, which passed.
- Interop run `relay-one-session-asks-for-much`, twice by itself: of 270 SUBSCRIBEs of the one session 8 passed on and 254 refused, and all 3 of the viewer passed on; of 90 FETCHes 4 passed on and 82 refused, and both of the viewer passed on.

## Not run

- More than four publishers of one namespace. The requests the relay sends for one SUBSCRIBE are as many as the sessions that announced a namespace of the track, and nothing bounds those; by the cost of a request, about ten publishers would bring the relay to a whole core at 1,000 a second.
- A publisher that answers a FETCH and writes its stream slowly (only one that answers nothing was run), and a page as the publisher that does not answer.
- Several sessions of one sender, which have a share each: 8 sessions, or 4 for FETCH, would do what one did.
- More than one session that asks; a machine with one core; a link that is not loopback; a page as the publisher that is asked.
- How many requests a second a viewer of the demo and the terminal viewer send at their busiest. It is needed for the number of a limit, and was not counted, since no limit is made here.

## Next action

No rate limit on requests was made: with publishers that answer, nothing beside the session was harmed on this machine. With a publisher that does not answer, one session kept others from it; the relay now gives one subscriber's session a share of a publisher (8 of 64 requests, 4 of 16 FETCHes, provisional), with which the other viewer's requests all reached the publisher. Several sessions of one sender are not counted together.
