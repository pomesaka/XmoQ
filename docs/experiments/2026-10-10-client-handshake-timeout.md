# A client whose handshake with the relay never ends

- Date: 2026-10-10
- Commit SHA: the commit that adds this record. The runs were made on top of `68fa616`
- Roadmap item: Phase 0b, "Put the runs in which a publisher or a subscriber disconnects mid-transfer into the interop matrix, so that CI runs them". Three of 1560 of those runs failed because a client could not connect ([record](2026-10-10-disconnects-through-own-relay.md))

## Objective

Find out why an own-rust client now and then does not get connected to the own relay, and whether the relay stops accepting connections at such a moment.

What was known: the client printed nothing, not even `CONNECTED`; the relay printed `HANDSHAKE_FAILED timed out` after its limit of 10 seconds; another client had connected just before; it happened with `wtransport` 0.7.2 and with `web-transport-quinn` 0.13.2.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Relay and client | `xmoq-relay` and `xmoq-moqt-client` 0.0.0 at `68fa616`, `web-transport-quinn` 0.13.2, `quinn` 0.11.12, debug builds |
| Certificate | The one the interop runs create under `interop/.cert` |

## Commands

From the repository root, after `cargo build --locked -p xmoq-moqt-client -p xmoq-relay` and one interop run for the certificate:

```sh
bun spikes/handshake-timeout/repro.ts --count 5000
bun spikes/handshake-timeout/repro.ts --count 4000 --parallel 8
bun spikes/handshake-timeout/repro.ts --count 8000 --killed 1 --limit 3000
python3 spikes/handshake-timeout/bind_check.py
netstat -an -p udp
```

`repro.ts` starts one relay and connects own-rust subscribers to it one after the other. Each asks for a track nobody published, is refused, and exits; one that has not exited after `--limit` milliseconds is counted as stuck. With `--killed`, a publisher on another track is connected and killed before every 20th connection, so that the relay holds sessions that are about to time out.

For three of the runs with `--killed`, `accept` in `crates/moqt-io/src/wt.rs` printed a line when a connection attempt arrived, when its QUIC handshake was done, and when the CONNECT request was read, each with the remote address. That patch was removed again and is not in the tree.

## Observed results

### The cause

**Input.** An own-rust client connects to `https://127.0.0.1:<port>`. It binds its UDP socket to `[::]:0`, a socket for IPv4 and IPv6 at once: `web-transport-quinn` 0.13.2 does so in `src/client.rs`, line 165 (`quinn::Endpoint::client("[::]:0")`), and `connect` in `crates/moqt-io/src/wt.rs` uses the library's `ClientBuilder` as it is.

**Condition.** The port the kernel picks for that socket is one that another process holds with an IPv4 UDP socket (`0.0.0.0:P`).

**Behaviour.** The client's first packet reaches the relay. The relay's answer, sent to `127.0.0.1:P`, goes to the IPv4 socket of the other process and not to the client. Sending again changes nothing, so the QUIC handshake never ends. The client prints nothing, and the relay prints `HANDSHAKE_FAILED timed out` 10 seconds later. The relay goes on accepting other connections all the while.

### Evidence

1. **It reproduces with one relay and no load.**

   | Run | Connections | Stuck |
   |---|---|---|
   | No killed sessions (5000 one after the other, 4000 eight at a time, 300 more) | 9,300 | 0 |
   | A publisher killed before every 20th connection | 38,000 | 2 |

   The numbers are too small to say that killed sessions matter.

2. **The handshake stops inside QUIC, on one connection only.** In the stuck connection of the patched runs the relay printed that the attempt had arrived and never that its QUIC handshake was done:

   ```
   8.281 ACCEPT_STAGE incoming remote=[::ffff:127.0.0.1]:63071
   18.282 HANDSHAKE_FAILED timed out
   ```

   The relay hands each attempt to a task of its own (`accept_sessions` in `crates/relay/src/main.rs`), so one that hangs holds up no other.

3. **The port had not been used before in that run.** `:63071` appears in the relay's log in that one line only. A new client reusing the port of a killed one is not the cause.

4. **Another socket held the same port.** `netstat -an -p udp` right after the run:

   ```
   udp4   18754      0  *.63071                *.*
   ```

   An IPv4 socket bound to that port, with 18,754 bytes received and not read, which is where the relay's answers and their repetitions would have gone. Its owner is not known: `lsof` did not show it among the processes of the same user. Two more UDP sockets were bound in the ephemeral range at that moment.

5. **macOS allows the two binds and delivers to the IPv4 socket.** `bind_check.py` binds `0.0.0.0:P`, then `[::]:P` with `IPV6_V6ONLY` off, and sends to `127.0.0.1:P`:

   ```
   [::]:63459 bound while 0.0.0.0:63459 is held
   a datagram to 127.0.0.1 reached: the IPv4 socket
   ```

6. **The kernel gives a dual-stack socket bound to port 0 a port that an IPv4 socket holds, and gives an IPv4 socket none.** Measured in [the record of the client's socket](2026-10-10-client-socket-family.md) with `spikes/client-socket-family/auto_port.py`: with 4,000 IPv4 UDP sockets held, 4,873 of 20,000 binds of `[::]:0` got a held port and none of `0.0.0.0:0`; the share of hits is the share of the range that is held. A count made for this record before that one existed agreed: with 200 sockets held, 253 of 20,000 binds of `[::]:0` and none of `0.0.0.0:0`. Its script is not kept, as the other does the same.

### What is shown and what is not

Shown: with IPv4 UDP sockets bound in the ephemeral range, a client that binds `[::]:0` gets one of their ports at the rate of their share of the range (6); a datagram to `127.0.0.1` on such a port reaches the IPv4 socket and not the client (5); one stuck connection had a port for which an IPv4 socket with unread bytes was found afterwards, and its handshake stopped inside QUIC while the relay served others (2, 3, 4). A client that binds `0.0.0.0:0` did not get such a port (6).

Not shown: that the IPv4 socket of (4) was there when the client bound, as `netstat` was run after the run; whose socket it was; and that the other stuck connection here and the three failed interop runs had this cause. Their ports were not recorded.

The rate cannot be checked against a prediction. It depends on how many IPv4 UDP sockets are bound at the moment a client binds, which was not counted during any run: three were seen once, after a run, which would give one connection in 5,500. Observed were 2 stuck in 38,000 connections here and 3 failed in 1560 interop runs of about eight connections each.

## Limitations and blockers

- **The three failures of the interop runs are not shown to have this cause.** Their ports were not recorded. They have the same form.
- **The port of the first stuck connection here is not known either**; it happened before the patch that prints the address.
- **One stuck connection is tied to an IPv4 socket, and only after the fact**: `netstat` was run when the run was over.
- `bind_check.py` binds the port it is given. That the kernel picks such a port by itself is shown in the other record (6), with sockets of the same process; other processes' sockets were not tried.
- **Linux was not tried**, so whether the CI runners are affected is open. Linux is expected to refuse the second bind where the first socket holds the port for IPv4; not verified.
- That `wtransport` 0.7.2 also binds a dual-stack socket by default, which would explain the failure before the change of library, was not checked.
- That browsers open IPv4 UDP sockets for their connections to `127.0.0.1`, which would raise the rate during interop runs, was not counted.
- Nothing was changed for this record. The remedy that follows from the cause is in the client: bind a socket of the family of the address it connects to (`0.0.0.0:0` for an IPv4 address). The own Rust client does so since [the record of the client's socket](2026-10-10-client-socket-family.md). The moqtail Rust peer under `interop/peers/` binds with `wtransport`'s default, a dual-stack socket, and is a copy that is not changed, so the runs with moqtail-rust can still fail in this way.

## Open questions raised

- Does a browser client have the same exposure, or does the browser bind a socket of the right family? Not looked at.
- Until the client is changed, interop runs on macOS can fail in this way; three of 1560 failed with a client that could not connect, for a cause that is likely this one and not shown to be. Is that acceptable for CI, given that it is not known whether Linux is affected?
