# The Rust client cancels a FETCH on its request stream alone

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay keeps the latest Group of each track and answers a FETCH from it where it can

## Objective

A subscriber that cancels a FETCH has to stop the request stream and may stop the fetch stream (§5.2: "the subscriber wishing to cancel the FETCH MAY send STOP_SENDING for the data stream as well as the bidi request stream. It MUST send STOP_SENDING for the bidi request stream"); the publisher then "MUST reset the bidi request stream and unidirectional data stream associated with the FETCH". The own clients stop both streams themselves, so no run shows the relay resetting a fetch stream it writes because its request stream was cancelled (`ABORT_FETCH_WRITE` in its log). The Rust client can now cancel after a time, on the request stream alone, and stay. Run it against the own relay.

## Environment

The own relay and the Rust client built from the working copy this record is part of, on both ends. Loopback, port 48600, macOS.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"
# The publisher: Object 0 of the Groups 0 to 3, 400 KiB each. It answers no FETCH; the relay's cache does.
$C --mode publish --payload-hex <409,600 bytes> --timeout-secs 60
# 16 FETCH requests for the latest Group, none of them read, cancelled after 2 s, and 3 s more.
$C --mode fetch --fetch-start 3,0 --fetch-end 3,0 --fetch-count 16 --fetch-read-after-ms 60000 \
    --fetch-cancel-after-ms 2000 --fetch-cancel-request-only --fetch-stay-ms 3000
```

The three flags are new. `--fetch-cancel-after-ms <N>` gives a FETCH up N milliseconds after it was sent. `--fetch-cancel-request-only` has a FETCH that is given up cancelled on its request stream alone: the fetch stream is neither stopped nor read any further. `--fetch-stay-ms <N>` keeps `--mode fetch` for N milliseconds after every FETCH is over, with its streams as they are; without it the client leaves and closes its connection.

What the flags do not do:

- `--fetch-cancel-request-only` is refused together with `--fetch-cancel-after-objects`: the task that counts the Objects ends when the number is reached, and its fetch stream is stopped with it.
- Only a fetch stream that is open when the FETCH is given up is kept. One that the publisher opens afterwards is not taken by the client while it stays, and is left to the session.
- A FETCH that ends in another way than by the client giving it up (its request stream reset by the peer, a fetch stream the client does not take, the client leaving) has its fetch stream stopped as without the flag.
- When the client leaves after its stay, the fetch streams it kept are stopped (STOP_SENDING) before the connection is closed.
- The stay does not watch the connection: the client stays the whole time and ends with the exit code 0 also when the connection went meanwhile. The stay counts against `--timeout-secs`.

## Observed results

One run.

- The client printed `FETCH_ENDED ... how=cancelled code=none objects=0` for all 16, and left after 5 seconds with the exit code 0.
- The relay's log has `RESET_RECEIVED session=1 ... code=0x1` and `STOP_SENDING_RECEIVED session=1 ... code=0x1` for each of the 16 request streams, at 2 seconds.
- **It has `FETCH_WRITE_ENDED session=1 ... finished pieces=1` for all 16, and no `ABORT_FETCH_WRITE`.** The relay had written every fetch stream to its end before the cancellation came: the transport took the 16 times 400 KiB though the client read none of it.

## Limitations and blockers

- **No test of the crate fails when the flag is taken out.** What the flags do is in tasks bound to a connection, and the tests of the crate cover how the flags are read and how a FETCH steps. Five changes that leave them passing: stopping the fetch stream on the path of the flag as well, switching the branch of `cancel_request_only` off, taking the note of the time that ran out for a stream that was stopped, removing the stay, and not starting the task that tells the time. The first two are seen by a pairing of `interop/` that came after this run (a page that publishes eight Objects of 900,000 bytes, and this client with the flags): the relay's log has `FETCH_WRITE_ENDED ... aborted` with the flag and `stopped` without it. That is what the author of the pairing reported of seven runs; it was not run for this record. The other three are seen by nothing.

- **The run did not show what it was made for.** A fetch stream the relay still writes when its request stream is cancelled takes more than the transport takes unread on one stream, and a Group of the Rust publisher is one Object, 400 KiB at most here (the length of a command line). A page that publishes a Group of several megabytes can be fetched by this client with these flags; that pairing is not in `interop/` yet.
- That the fetch stream is kept, unread and not stopped, was not seen from outside: the relay's log would have `stopped` for a stream the client had stopped, and has `finished`, which a stream that was written whole before the cancellation has either way.
- The tasks of the client that hold the streams have no test: they are bound to a WebTransport connection. The flags and what they are refused with are tests of the client.
