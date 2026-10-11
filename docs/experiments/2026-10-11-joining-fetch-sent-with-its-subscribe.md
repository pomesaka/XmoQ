# A Joining FETCH sent together with its SUBSCRIBE, through the own relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

A Joining FETCH that the relay read before the SUBSCRIBE it names was refused, 6 times in 50 on loopback ([record](2026-10-11-rust-client-fetch.md), run 5). The relay now holds such a FETCH until its SUBSCRIBE comes. See that a subscriber which sends the two together is no longer refused, and that a FETCH whose SUBSCRIBE never comes still is.

## Environment

The own relay and the Rust client built from the working copy this record is part of. moqtail's test client 0.15.0 as the publisher. `spikes/fetch-capture` for run 2. Loopback, port 48600, macOS, nothing else running.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
spikes/moqtail/.tools/bin/client --command publish --server moqt://127.0.0.1:48600 --no-cert-validation \
  --namespace xmoq/phase0 --track-name object --interval 250 --objects-per-group 3 --group-count 1200 --payload-size 8

# 1: 500 times, one after the other, with one relay and one publisher.
target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object \
  --mode subscribe --objects 1 --join relative:0 --join-at-once

# 2: a Relative Joining FETCH, Request ID 2, that names the Request ID 0, and no SUBSCRIBE.
spikes/fetch-capture/target/debug/xmoq-fetch-capture --url https://127.0.0.1:48600 --cert-sha256 <sha256> --control af000000 \
  --wait-ms 13000 --request "16 00 07 02 02 00 00 01 20 c8"

# 3: five times, each with a relay and a publisher of its own (Objects of 1 MB).
target/debug/xmoq-moqt-client ... --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fetch-cancel-after-objects 2
```

The request of run 2 is the bytes of `fixtures/moqt-18/fetch-relative-joining-moqtail-rs.hex`.

## Observed results

**Run 1: none of 500 was refused.** All 500 clients printed `FETCH_OK`, `FETCH_ENDED ... how=fin`, and an Object of the subscription, and left with exit code 0. The relay's log has no REQUEST_ERROR with `0x32`. It has 12 lines `HOLD_REQUEST session=N stream=S FETCH`: in 12 of the 500 the relay read the FETCH before the SUBSCRIBE, held it, and went on when the SUBSCRIBE came. The run was made twice; the first time, before the relay logged that line, all 500 got FETCH_OK as well. Each took 131 seconds.

**Run 2: a FETCH whose SUBSCRIBE never comes is refused, after it was held.** The relay's log: `HOLD_REQUEST session=1 stream=1 FETCH`, then `SEND session=1 stream=1 REQUEST_ERROR error_code=0x32`. The tool got `05 00 03 32 00 00` (REQUEST_ERROR, `INVALID_JOINING_REQUEST_ID`, Retry Interval 0, no reason) and a FIN on the request stream, within the 13 seconds it waited. When within those 13 seconds was not measured; that it is 10 seconds is a test of the relay with the clock stopped.

**Run 3: the relay's log has one line for the end of a fetch stream that is cut, and not two.** In [the record of the Rust client](2026-10-11-rust-client-fetch.md), run 7, the log had `ABORT_FETCH_PIPE` and after it `FETCH_PIPE_ENDED` for the same FETCH. Here, in five runs: four had `FETCH_PIPE_ENDED session=1 stream=1 downstream_stopped code=0x1` and no `ABORT_FETCH_PIPE`, one had `ABORT_FETCH_PIPE session=1 stream=1 code=0x1` and no `FETCH_PIPE_ENDED`. All five had `CANCEL_OWN_REQUEST session=0 own=0 code=0x1`.

## Limitations and blockers

No blocker.

- **Loopback, one machine, nothing else running.** How long a SUBSCRIBE can be behind its FETCH on a link that loses packets was not looked at; the relay waits 10 seconds.
- 12 in 500 is how often the relay read the FETCH first in this run, 2.4%, where the run of the record before had 12% refused. The two runs differ in the relay (this one logs more) and were not made to be compared; what the count says is that the path was taken, and that none of those that took it was refused.
- A SUBSCRIBE that comes and waits for a publisher, one the relay refuses, an ID that something else uses, and a subscriber that leaves while its FETCH is held were not run against a real peer; they are tests of the relay core.
- Run 3 shows that the two lines did not come together in five runs. That they cannot is by reading the relay: an end that is reported for a FETCH the relay has ordered stopped is dropped before the core sees it.
- `moqlivemock`'s viewer, which sends the two together, was not run: it cannot reach its publisher through the own relay ([record](2026-10-11-rust-client-fetch.md), blocker).
