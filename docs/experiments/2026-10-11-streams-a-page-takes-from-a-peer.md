# How many streams a page takes from a peer, and how many Chromium permits

- Date: 2026-10-11
- Commit SHA: the commit that adds `maxPeerBidiStreams` to `@xmoq/moqt` together with this record. The runs were made on its working copy
- Roadmap item: groundwork for Phase 1 (limits on what is received). It closes no item

## Objective

`@xmoq/moqt` read every bidirectional stream a peer opened, and since it decodes a FETCH whole before refusing it, each can hold up to 64 KiB. How many there can be was up to the browser. Find out how many streams Chromium lets a peer have open at once, put a limit of the library's own on the bidirectional ones as there is on the unidirectional ones, and see both numbers in a real browser.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | the own relay, **with a temporary change that is not committed**: when `XMOQ_PROBE_STREAMS` is set, it opens up to 400 streams towards every session, 1.5 seconds after the session's control stream, writes to each the first bytes of a FETCH (bidirectional) or the type of a Subgroup Header (unidirectional), keeps them open, and prints a line for each it could open. Taken back, the diff of `crates/` empty |
| Page | the own page of `interop/`, with `@xmoq/moqt` at its defaults: 64 unidirectional and, with this change, 64 bidirectional streams of the peer read at once |
| Other client | an own Rust client in the same run, which got the same probe |

## Commands

```sh
cd interop
XMOQ_PROBE_STREAMS=bidi bun disconnect.ts --scenario publisher-killed --publisher own-rust --subscriber own-browser --verbose
XMOQ_PROBE_STREAMS=uni  bun disconnect.ts --scenario publisher-killed --publisher own-rust --subscriber own-browser --verbose
```

The scenario is only there to keep a page connected for some seconds. Counted in its log: the relay's `PROBE_OPENED` lines for the page's session, and the streams of the peer for which the page printed received bytes.

## Observed results

| | Streams the relay could open towards the page | Streams the page read |
|---|---|---|
| Bidirectional, first run | 100 | 64 |
| Bidirectional, second run | 100 | 64 |
| Unidirectional | 100 probes, beside the relay's control stream | 65 over the run: the control stream, one subgroup stream that ended, and 63 probes. 64 at once |

- **Chromium 156 permits a peer 100 streams of each kind at once.** The relay's 101st `open` waited for the rest of the run. That is the number in Chromium's source, read at the tip of its repository on 2026-10-11 (`net/quic/dedicated_web_transport_http3_client.cc`): `kMinimumIncomingWebTransportStreams = 100`, "The WebTransport specification requires clients to initially permit at least 100 peer-initiated streams of each type, even if the application provides a lower hint". A page can ask for more with `anticipatedConcurrentIncomingBidirectionalStreams` and its unidirectional twin, not for fewer; `@xmoq/moqt` passes neither.
- **The page read 64 of each and left 36 with the browser.** With the unidirectional probes there were more than 100 streams of the peer in all (the control stream and the streams of HTTP/3 beside the probes); how Chromium counts those against the 100 was not looked at.
- The own Rust client was given 100 bidirectional probes as well, and 98 unidirectional ones. It reads 64 of each at once, like the page, and sets no QUIC limit of its own: the 100 is the default of `quinn` (`TransportConfig::default` in `quinn-proto` 0.11.19; `client_config` in `crates/moqt-io/src/wt.rs` builds no transport configuration, the relay's side does). So the relay is the one of the three that makes the two numbers the same.
- The scenario passed in all three runs: 64 streams that never say what they are do not disturb a subscription, which is on a request of the page's own.

What the limit bounds, by arithmetic and not by a run: a first message is at most 9 + 2 + 65,535 bytes, so 64 streams hold at most about 4 MiB of requests that have not fully arrived, where the browser's 100 allowed about 6.4 MiB.

The tests with the played transport (`packages/moqt/test/webtransport/connection.test.ts`): with a limit of 2, five streams with the start of a FETCH leave three untaken; a stream gives up its place when the peer ends its direction or resets it, and not when its request was answered. Without the limit that test fails (a temporary change, taken back).

## Limitations and blockers

- **Nothing ends a stream that holds its place and says nothing.** The probes held theirs for the whole run. The Rust implementation stops such a stream after 10 seconds; `@xmoq/moqt` has no timer for it (implementation notes, known gaps).
- How much the browser holds for the 36 streams it does not hand over was not measured; the probes wrote a few bytes each.
- One browser, one version, loopback. The 100 was read in the source of the tip and measured in 156; other versions were not looked at.
- The relay cannot be told, by a page, to open fewer streams: draft 18 has no message for it, and the page cannot lower the browser's number.

## Open questions raised

- When `@xmoq/moqt` gets a time limit for a stream that does not say what it is: before it serves a request of the peer, by the notes.
