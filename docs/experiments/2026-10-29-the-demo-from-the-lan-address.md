# The demo from the LAN address

- Date: 2026-10-29
- Commit SHA: the commit that adds this record
- Roadmap item: none; the demo shown to others on the same network (`--lan` of `demo/serve.ts`).

## Objective

The person wants to show the demo to colleagues on the same WiFi, without Tailscale; a Cloudflare Tunnel is not expected to carry WebTransport. Can a page opened at the LAN address of the machine reach the relay over WebTransport and use the camera, and what does it take?

Three things were not known before: whether Chromium takes the relay's certificate by its hash (`serverCertificateHashes`) at an address its SAN does not name (it names `localhost` and `127.0.0.1`); whether a page over HTTPS whose certificate nobody trusts, past the warning, is a secure context with WebTransport and the camera; and how the pages reach the rooms (packages/room on Miniflare), which listen on 127.0.0.1 over plain HTTP.

**One machine**, opening its own LAN address (192.168.50.228): no other machine was tried.

## Expected

Said before the runs (in the session, not in a file): the hash is taken whatever the host, since it stands in for the check of the name; a page past the warning is a secure context, as its origin is HTTPS; a page over HTTPS cannot fetch the rooms over plain HTTP.

## Environment

macOS 14.4 (Darwin 23.4.0), Chromium 156.0.8078.4 of Playwright 1.64.0 (`playwright-core` 1.64.0), headless, with `--use-fake-device-for-media-stream` and `--use-fake-ui-for-media-stream` (a fake camera and microphone, and the permission given without asking). Bun 1.3.14. The firewall of macOS was off. The demo on its own ports: `bun demo/serve.ts --lan 192.168.50.228 --relay-port 37443 --port 37080 --rooms-port 37787`, with tokens.

## Commands

1. Before `--lan` was written: from a page of `http://localhost:37080`, and from a page over HTTPS at `https://192.168.50.228:37445` with a self-signed certificate of its own (passed by clicking "Advanced", then "Proceed" on Chromium's warning), `new WebTransport` to the relay at `https://192.168.50.228:37443/` and at `https://127.0.0.1:37443/`, the relay's certificate given by its hash, with the protocol `moqt-18`; and `getUserMedia` of video and audio.
2. With `--lan`: the pages at `https://192.168.50.228:37080`, each past the warning: `/pet?room=lan`, food put on the floor; `/publish?source=camera` and `/watch`; `/meet?room=lan&source=camera` twice. The first line of each page after a few seconds.

## Observed results

**1.** Both pages, to the relay at the LAN address and at 127.0.0.1 alike:

| Page | `isSecureContext` | WebTransport | Camera and microphone |
|---|---|---|---|
| `http://localhost:37080` | true | ready | audio+video |
| `https://192.168.50.228:37445`, past the warning | true | ready | audio+video |

The hash was taken at an address the certificate does not name. Without the protocol `moqt-18` in the handshake, the relay refused it at both addresses ("Opening handshake failed"), which was the probe's mistake, not the address.

**2.** The pages over HTTPS first could not reach the rooms through this server: "TypeError: Failed to fetch". Bun's `fetch` gave the rooms' answer uncompressed and kept `Content-Encoding: gzip`, and the browser could not read it; dropping that header (and `Content-Length`) from the answer passed on fixed it. Then:

| Page | Its line |
|---|---|
| `/pet` | in the garden "lan"; the bowl at (244, 278), the pet `eat` towards it, and 8 s later the floor empty |
| `/publish` | publishing, 20 frames a second from the camera |
| `/watch` | playing, 20 frames a second shown, 150 ms of buffer |
| `/meet` × 2 | each seated, publishing, attending, and the other among the others; both tiles show the fake camera's picture |

**Without `--lan`**, the same pages at `http://localhost:37080` as before: the server on 127.0.0.1 alone, the rooms at `http://127.0.0.1:37787/`, and the pet, the broadcast and the meeting of two as in the table above.

## Limitations and blockers

- Another machine was not tried: the LAN address was opened from the machine itself, whose packets do not cross the network. A router that keeps the machines on its WiFi apart would stop it.
- The fake permission UI gave the camera without asking; that a real Chromium asks for the camera on a page past the warning, and is answered, was not tried.
- The firewall of macOS was off; that it asks for `xmoq-relay` and `bun` on the first run when on was not tried.
- Browsers other than Chromium were not tried.
- portless's LAN mode (`portless proxy start --lan`, `.local` names over mDNS) was considered and not used: a colleague would pass a warning all the same unless they installed portless's certificate authority, and the proxy of the machine serves other projects under `.localhost`, which the LAN mode would move to `.local`.

## Open questions raised

None.
