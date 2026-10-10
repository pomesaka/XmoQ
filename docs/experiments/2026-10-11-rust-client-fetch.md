# The Rust client sends FETCH, through the own relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

`xmoq-moqt-client` now sends a Standalone FETCH (`--mode fetch`) and a Joining FETCH next to its SUBSCRIBE (`--join`), and reads the fetch stream. Run it through the own relay against a publisher written by someone else, and run what was only tested on in-memory connections so far: a subscriber that does not read its fetch stream, one that gives the FETCH up, one that sends the Joining FETCH together with its SUBSCRIBE, and one that takes every FETCH a publisher is asked for.

## Environment

The own relay and the Rust client built from the working copy this record is part of. moqtail's test client 0.15.0 as the publisher (`spikes/moqtail/.tools/bin/client`). Loopback, port 48600, macOS. Every run but the 50 of run 5 has a relay and a publisher started anew.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
P="spikes/moqtail/.tools/bin/client --command publish --server moqt://127.0.0.1:48600 --no-cert-validation --namespace xmoq/phase0 --track-name object --interval 250 --objects-per-group 3"
$P --group-count 40 --payload-size 8          # runs 1 to 5
$P --group-count 80 --payload-size 1000000    # runs 6 to 9

# Four seconds after the publisher started.
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"
$C --mode fetch --fetch-start 0,1 --fetch-end 1,2                                  # 1
$C --mode subscribe --objects 3 --join relative:1                                  # 2
$C --mode subscribe --objects 2 --join absolute:3                                  # 3
$C --mode fetch --fetch-start 0,0 --fetch-end 2,0 --fill-timeout-ms 0              # 4a
$C --mode fetch --fetch-start 0,0 --fetch-end 2,0 --group-order descending         # 4b
$C --track nothing --mode fetch --fetch-start 0,0 --fetch-end 2,0                  # 4c
$C --mode subscribe --objects 1 --join relative:0 --join-at-once                   # 5: 50 times, one relay and publisher
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fetch-read-after-ms 13000      # 6
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fetch-cancel-after-objects 2   # 7
# 8 and 9: A, and three seconds later B and then C, on sessions of their own.
$C --mode fetch --fetch-count 17 --fetch-start 0,0 --fetch-end 3,0 --fetch-read-every-ms 2000   # 8, A
$C --mode fetch --fetch-count 17 --fetch-start 0,0 --fetch-end 3,0 --fetch-read-every-ms 500    # 9, A
$C --mode fetch --fetch-start 0,0 --fetch-end 1,0                                  # B
$C --mode subscribe --objects 3                                                    # C
```

## Observed results

One run each, but for run 5. Runs 1, 4c and 7 were made again after the client was changed in review to print one `FETCH_ENDED` for every FETCH and to count its Objects where they are printed; the lines here are of the second time, and but for the `FETCH_ENDED` of 4c they are the same as the first.

**The client got what the relay passed on.**

| Run | What the client printed |
|---|---|
| 1 | `FETCH_OK ... end_group=1 end_object=2`, four `FETCH_OBJECT` lines ({0, 1}, {0, 2}, {1, 0}, {1, 1}), `FETCH_ENDED ... how=fin code=none objects=4` |
| 2 | `SUBSCRIBED`, `FETCH_OK ... end_group=5 end_object=1`, Objects {4, 0} to {5, 0} on the fetch stream, then `OBJECT` lines of the subscription from {5, 1} |
| 3 | `FETCH_OK ... end_group=5 end_object=1`, Objects {3, 0} to {5, 0}, then the subscription from {5, 1} |
| 4a | `FETCH_OK ... end_group=2 end_object=0`, `FETCH_RANGE_END ... kind=unknown group=2 object=18446744073709551615`, `FETCH_ENDED ... how=fin ... objects=0` |
| 4b | `FETCH_OK ... end_group=2 end_object=0`, the three Objects of Group 0, `how=fin`. That only Group 0 comes is the publisher's doing ([record](2026-10-11-standalone-fetch-through-own-relay.md), run 7) |
| 4c | `FETCH_REFUSED ... error_code=0x10`, `FETCH_ENDED ... how=refused code=none objects=0` |

In runs 2 and 3 the fetch stream ended with the Object at the Joining Location and the subscription began with the next one.

**Run 5: a Joining FETCH sent together with its SUBSCRIBE was refused 6 times in 50.** 44 got FETCH_OK and their fetch stream; 6 got `FETCH_REFUSED ... error_code=0x32` (`INVALID_JOINING_REQUEST_ID`), and the relay's log has the six REQUEST_ERROR messages. In all 50 the subscription itself went on and delivered its Object. The relay read the FETCH before the SUBSCRIBE in those six (implementation notes, known gaps). The 50 took 17 seconds, on a machine that did nothing else.

**Run 6: a subscriber that read nothing lost its FETCH after 10 seconds, and the publisher's stream was stopped then.** The client printed `FETCH_OK`, then `FETCH_ENDED ... how=request_reset code=0x5 objects=0`. The relay: `PIPE_FETCH ...`, `FETCH_PIPE_ENDED session=1 stream=1 downstream_stalled`, `CANCEL_OWN_REQUEST session=0 own=0 code=0x1`, `RESET_REQUEST session=1 stream=1 code=0x5`. The publisher logged the FETCH at 23:53:10.31 and "Failed to serve FETCH objects: [SendDataStream::send_object], [stream closed or stopped]" at 23:53:20.34: its fetch stream was held for 10 seconds and not longer.

**Run 7: a subscriber that gave the FETCH up after two Objects had the relay's FETCH cancelled within 150 milliseconds.** The client: two `FETCH_OBJECT` lines, `FETCH_ENDED ... how=cancelled code=none objects=2`. The relay: `STOP_SENDING_RECEIVED session=1 stream=1 code=0x1`, `ABORT_FETCH_PIPE session=1 stream=1 code=0x1`, `CANCEL_OWN_REQUEST session=0 own=0 code=0x1`, `RESET_REQUEST session=1 stream=1 code=0x1`, and after those `FETCH_PIPE_ENDED session=1 stream=1 downstream_stopped code=0x1`. The publisher logged the same failure to send 145 milliseconds after it had begun.

The last line of the relay is one more than its own contract has: the task that passed the stream on saw the subscriber's STOP_SENDING for the fetch stream before the relay's order to stop reached it, and reported an end that the relay core no longer had a FETCH for. Nothing came of it.

**Runs 8 and 9: one subscriber can take every FETCH a publisher is asked for, if it reads fast enough.**

| Run | A reads a piece of each fetch stream every | A's 17 FETCH requests | B's FETCH, three seconds after A's | C's subscription | After 25 seconds |
|---|---|---|---|---|---|
| 8 | 2 seconds | 16 FETCH_OK, the 17th `FETCH_REFUSED ... error_code=0x9` | `FETCH_REFUSED ... error_code=0x9` | three Objects | all 16 of A's were over: the relay's log has 16 lines `FETCH_PIPE_ENDED ... downstream_stalled`, and A had printed no Object |
| 9 | half a second | the same | the same | three Objects | A still had its 16 open, and had printed 16 Objects, one of each |

So the limit of 16 for a publisher kept the subscription of C going, and B was refused while A held the 16, as the implementation notes say of it. What the notes say of the time is not what run 8 showed: a subscriber that takes "1 KiB every 10 seconds" is not cut in the tests on in-memory connections, and A in run 8, which read 16 KiB of each stream every 2 seconds, was cut as one that takes nothing. A piece the client's reader takes is 16 KiB (`xmoq-moqt-io`), and the Objects were of 1 MB. That is all that was seen: one subscriber on loopback, 16 streams, two paces.

Not seen, and what the code suggests: the relay's clock does not ask why a subscriber is slow, so a viewer on a slow link that reads one FETCH slowly may be cut the same way; no slow link and no single slow FETCH was run. A subscription is not cut by this clock; its subgroup streams are given up by how much waits to be written, not by time (`xmoq-relay`, read, not run here).

## Limitations and blockers

**Blocker for the catalog of `moqlivemock` through the own relay: `mlmpub` is a server, and the own relay connects to nobody.** Its README at `v0.16.1` (read on 2026-10-11 through the GitHub API) has "`mlmpub` is the server and publisher" and "mlmpub does not announce its namespaces unprompted. A peer finds out what is on offer by sending SUBSCRIBE_NAMESPACE". `mlmrel` reaches it with `-upstream`. The own relay accepts connections only, and has no SUBSCRIBE_NAMESPACE. So a viewer cannot reach `mlmpub` through the own relay, whatever the relay does with a FETCH. `mlmpub` was not built or run for this record.

- **One run each, on one machine**, but for run 5. The 6 in 50 of run 5 is a count on loopback with nothing else running; it says that it happens, not how often it does elsewhere.
- **Why A was cut in run 8 was not looked into.** A guess, not checked against quinn's code: the client's transport tells the relay of room in its receive window in steps and not as each byte is read, and at 8 KiB a second a step took longer than the 10 seconds the relay waits for 1 KiB to be taken.
- Run 9 was stopped after 25 seconds. How long A could have held the 16 was not measured.
- The Rust client does not answer a FETCH: the publisher in every run is moqtail's. The Rust client as the publisher, and both ends being own clients, are the next change.
- A FETCH whose request stream the relay finishes before it answers, and a fetch stream whose bytes do not decode, were not run. What the client decides in each is a table test of `fetching::step`; that the streams then do what was decided is in no test.
- A client that is cut off by its own time limit prints `FETCH_ENDED ... how=left` for a FETCH that is open: seen once, with a Joining FETCH for Objects of 1 MB read at 32 KiB a second and `--timeout-secs 8`.
- What the client holds of a fetch stream it does not read (five reads of 16 KiB, `xmoq-moqt-io`) was not measured in run 6.
- The moqtail npm package, `moqlivemock` and a browser were not run.
