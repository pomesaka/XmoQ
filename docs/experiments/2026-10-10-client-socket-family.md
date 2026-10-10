# The Rust client binds a socket of the server's address family

- Date: 2026-10-10
- Commit SHA: the commit that changes how the client opens its socket together with this record. The runs were made on its working copy
- Roadmap item: Phase 0b, "Put the runs in which a publisher or a subscriber disconnects mid-transfer into the interop matrix": three of 1560 of those runs failed because a client could not connect

## Objective

An own Rust client now and then never got connected and printed nothing. The cause was found in another record (`2026-10-10-client-handshake-timeout.md`, not merged when this was written): the client bound its UDP socket to `[::]:0`, a socket for IPv4 and IPv6 at once; macOS can give such a socket a port that an IPv4 socket of another process holds, and then delivers what an IPv4 server sends to that other socket.

Three things had to be shown: that the kernel does choose such a port by itself when a socket is bound to port 0 (the other record tried a bind to a port it named), that the client no longer gets into that state when the server has an IPv4 address, and that a client that cannot connect says so instead of waiting without a word.

Not shown here: that the failures seen in the interop runs had this cause. Their ports were not recorded.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Code | `xmoq-moqt-io` 0.0.0 on `web-transport-quinn` 0.13.2, `quinn` 0.11.12 |

## What was changed

`connect` in `crates/moqt-io/src/wt.rs` builds the QUIC endpoint itself and binds `0.0.0.0:0` for an IPv4 server and `[::]:0` for an IPv6 one. `ClientBuilder` of the library binds `[::]:0` in the call that builds the client and hands out neither the endpoint nor the configuration, so what it sets is repeated: the verifier for a pinned certificate hash, TLS 1.3 only, the ALPN `h3`, no client certificate, quinn's default transport settings. For a host name, the first address of the resolver decides.

The session has to be there within 10 seconds (provisional), or the client fails with what it was waiting for, the server's address and its own.

## Commands

```sh
python3 spikes/client-socket-family/auto_port.py 4000 20000
python3 spikes/client-socket-family/auto_port.py 12000 20000
cargo test -p xmoq-moqt-io wt::
mise run check
mise run interop
```

## Observed results

### Which port the kernel chooses for port 0

`auto_port.py` holds a number of IPv4 UDP sockets on `0.0.0.0` with ports the kernel chose, then binds a new socket to port 0, reads its port and closes it, 20,000 times for each kind of socket. The ephemeral range of the host is 49152 to 65535, 16,384 ports.

| IPv4 sockets held | `[::]:0`, both families | `0.0.0.0:0` |
|---|---|---|
| 4,000 | 4,873 of 20,000 got a held port | 0 of 20,000 |
| 12,000 | 14,698 of 20,000 got a held port | 0 of 20,000 |

**The kernel takes no account of IPv4 sockets when it chooses a port for a socket of both families**: the share of binds that got a held port (24.4% and 73.5%) is the share of the range that was held (24.4% and 73.2%). An IPv4 socket never got a held port. No bind failed.

### The client

**Passed.** The test `a_port_that_an_ipv4_socket_of_someone_else_holds_does_not_stop_a_client` makes the state on purpose, against a server on the loopback interface:

1. It binds an IPv4 UDP socket to `0.0.0.0:0` and reads the port `P` the kernel chose. This stands for the socket of another process.
2. It connects from a socket bound to `[::]:P`, which is what the client used to bind when the kernel happened to choose `P`. macOS allowed the bind, and the connect ended after the 1.5 seconds the test allows, with `no session with 127.0.0.1:<port> within 1.5s, sending from [::]:P: the QUIC handshake did not complete`. The test compares that text on macOS. **This is the hang, reproduced every time**, and now it ends with a message.
3. With the IPv4 socket still held, it calls `connect` as the clients do. The session is established, the client's socket is an IPv4 one, and its port is not `P`.

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `32 of 32 pairings ended as expected`.

## Limitations and blockers

No blocker.

- **Linux was not run.** There the test only asserts that step 2 fails, since Linux is expected to refuse the bind of `[::]:P`; not verified here. CI runs the test on Linux.
- Step 2 goes through the changed `connect` with the old bind address, not through the old code.
- That an IPv4 socket never gets a port another IPv4 socket holds is the kernel's rule for sockets without `SO_REUSEPORT`; the test sees it once per run and does not prove it.
- An IPv6 server was not run. For one, the client binds `[::]:0` as before, which also takes IPv4; whether an IPv4 socket of another process on the same port can take anything from it was not tried (the server's packets are IPv6 there).
- **The moqtail Rust peer has the same weakness and is left as it is.** `interop/peers/moqtail/rust-client/src/main.rs` builds its client with `with_bind_default()`, which `wtransport` 0.7.2 documents as `IpBindConfig::InAddrAnyDual` (`src/config.rs`, read 2026-10-10). The peer is a copy of moqtail's code. A pairing with it can still fail in the old way, about as rarely as before.
- The browser clients were not looked at.

## Open questions raised

- Is the interop runner to tell this failure of a moqtail peer apart from others, for instance by retrying a pairing whose moqtail Rust client never printed `CONNECTED`?
