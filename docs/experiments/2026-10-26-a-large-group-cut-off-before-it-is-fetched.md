# A large Group cut off before it is FETCHed

- Date: 2026-10-26
- Commit SHA: the commit that adds this record, on its working copy on top of `49e30200`
- Roadmap item: none; the interop run (`interop/fetch.ts --fetch page-large-group-cancelled`, `interop/fetch-rust.ts --fetch page-large-request-only`)

## Objective

The pairing `own relay: fetch: own-browser asks for a Group of 8.3 MB the relay holds, reads nothing and gives up; the relay gives the FETCH up` failed once in a full interop run of another working copy (2026-10-26, about 12:07), and passed alone. Find why, and make the run not depend on how fast the page reads.

## What the failed run shows

From the output of that run, the lines of the pairing (sent by the session that ran it; the file of the pairing under `interop/.logs/matrix/` had been written over):

```
[relay] RESET_DATA out=3 code=0x5
[relay] RESET_DATA out=4 code=0x5
[relay] STOP_DATA session=0 stream=5 code=0x1
[relay] SEND session=1 control GOAWAY timeout_ms=0
[relay] CLOSE_SESSION session=1 code=0x10 reason="the GOAWAY's time is over"
[driver] FAILED page-large-group-cancelled FETCH through the own relay: evaluate: Error: SubgroupClosedError: the subscriber stopped the stream (STOP_SENDING)
    at Object.sendLargeGroup (…)
```

There was no `WRITE_FETCH_STREAM`, `FETCH_WRITE_ENDED` or `LOG_DROPPED` line: the FETCH was never sent.

## Why

- The track has two subscribers when the Group is sent: the own Rust client that has the relay ask the page for the track (`publishFromAPage`), and the page that fetches, which the run waits for to have the 83 Objects by its subscription before it sends the FETCH. The 8.3 MB go to both on their subscriptions first.
- The relay holds at most 4 MiB to be sent for one subscription and 8 MiB for one session unless told otherwise (`HubConfig` in `crates/relay/src/hub.rs`). A subscriber that reads the Group slower than it comes passes that, and the relay ends its subscription with TOO_FAR_BEHIND (0x5): the `RESET_DATA … code=0x5` lines. The `OPEN_DATA` lines of the run, sent afterwards, name both: `out=3` was to session 1 (the own Rust client) and `out=4` to session 2 (the page that fetches).
- The relay then stops the publisher's stream of the Group with CANCELLED (`STOP_DATA session=0 … code=0x1`), and the publishing page's write fails with `SubgroupClosedError … (STOP_SENDING)`. In the relay's core, a subscription that is ended lets go of the publisher's streams only where it was the last of the track (`remove_subscriber` in `crates/relay-core/src/lib.rs`, through `TrackEnd::Unwanted`). Both subscribers were cut off, so the subscription of the second was the last.
- That a subscriber read slower than the Group came only in a full run, with many pairings beside it, is a guess.
- `interop/fetch-rust.ts --fetch page-large-request-only` waits in the same way for a page to have its Group of 7.2 MB, which is over 4 MiB as well. No failure of it was seen.

### A second form of the same failure

The same session ran the pairing alone on top of another change (about 12:50, `--base-port 36000`), and it failed in one of three runs on `main` as well, with `timed out waiting for the page to have the 83 Objects of Group 3 by its subscription`. Its lines: the page that fetches (session 2) had Objects 0 to 22 of Group 3 when the relay reset its stream (`RESET_DATA out=4 code=0x5`) and sent it `PUBLISH_DONE status_code=0x5 stream_count=1`; the own Rust client (session 1) got the whole Group (`FINISH_DATA out=3`), and the publishing page sent it whole (`large group sent: group=3 objects=83`). One subscriber was left, so the publisher's stream was not stopped, and the page never had the 83 Objects the run waits for. It is the same cut, of one subscriber instead of two.

The failure of the same pairing in [the record of the reader and the transcript](2026-10-25-the-reader-for-agents-reads-the-transcript.md) is another one: there the relay's writing of the FETCH ended `aborted` where the run waited for `stopped`, which the run has taken as well since.

## The change

Both runs start the relay with `--subscription-send-bytes` and `--session-send-bytes` of 32 MiB (`RELAY_ARGS_FOR_A_LARGE_GROUP` in `interop/fetch-publishers.ts`), more than a run sends at once, so that a subscriber is not cut off however slowly it reads. What these runs are about is how a FETCH is given up, and not those limits. The relay's 8 MiB of a track is not changed.

## Runs

Each through `mise run interop -- --base-port 37500 --only …`, on macOS, arm64, 11 cores, after the change; the failure was not looked for before it:

- `asks for a Group of 8.3 MB`: 5 runs, 5 ended as expected, each with `FETCH_WRITE_ENDED … stopped pieces=62 code=0x1`. The relay stops writing the FETCH at 62 Objects, as before the change: what holds it back is what the page takes of a stream it does not read, and not the relay's limits.
- `own-rust asks for a Group of 7.2 MB …`: 5 runs, 5 ended as expected.

That the change mends the failure is shown only by the full runs after it not failing in this way.

The session that saw the failure alone in one of three runs on `main` ran the pairing on this change 7 times, and all 7 ended as expected; they took 17 to 45 s, where the runs that passed before the change had taken 1.9 to 3.5 s.

### How long the run takes

Measured here while another full interop run held the machine, with the driver printing the time of each step (a change made for these runs alone, and taken out). In ms from the start of the driver:

| Relay | Relay started | Publisher | Page subscribed | Group sent | Page has 83 | FETCH given up | Writing ended | Pairing |
|---|---|---|---|---|---|---|---|---|
| this change | 3127 | 8860 | 11157 | 12103 | 13260 | 14298 | 14299 | 45.8 s |
| this change | 3362 | 6873 | 8136 | 9382 | 10530 | 11740 | 11741 | 43.6 s |
| this change | 3060 | 5767 | 7614 | 9025 | 9148 | 10239 | 10239 | 42.3 s |
| the relay's own limits | 2406 | 6189 | 7569 | 8742 | 9390 | 10499 | 10499 | 42.2 s |
| the relay's own limits | 4329 | 10394 | 14952 | 18794 | 18810 | 19941 | 19944 | 54.8 s |
| the relay's own limits | 2078 | 7263 | 9026 | 10115 | 10784 | 12049 | 12050 | 45.3 s |

- The runs with the relay's own limits took as long: the time is not the change's. The page has the 83 Objects within 1.2 s of the Group being sent in every run, and the FETCH steps take about a second.
- The steps of the driver end within 20 s of its start; the longest step, the publisher's page, took 6.1 s, of the 20 s each step may take (`STEP_TIMEOUT_MS`). Sending the Group took 3.8 s at the most.
- The rest of each pairing, about 30 s, comes after the driver's steps, while what it started is stopped. In each of the six runs the own Rust client's session ended at the relay with `Lost("connection error: timed out")`. Where the 30 s go was not found: the lines of the log have no time.
- 42 to 55 s is within the 120 s a pairing may take, with these steps; how much the machine may be busier before a step meets its 20 s was not measured.

## Not looked at

- Why the subscribers read the Group slower than it came in those runs.
- The pairing was not run on `main` here before the change.
