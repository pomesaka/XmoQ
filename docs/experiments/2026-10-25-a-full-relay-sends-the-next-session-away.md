# A full relay sends the next session away

- Date: 2026-10-25
- Commit SHA: the commit that adds this record (on `main` `9c91aec1`)
- Roadmap item: Phase 7, relay scale (decided on 2026-10-25: a relay takes 1,000 sessions, and one that is full sends the next to another relay)

## Objective

A relay with `--when-full <URL>` answers a session that comes when `--max-sessions` are served with its SETUP and at once a GOAWAY naming that URL with `sent-away=1` in its query, and closes such a session where it is full itself. Does a page that comes to a full relay play at the one it is sent to, and do two full relays that name each other send it away once and not round and round?

**Loopback, one machine**: the two relays and the pages share one Mac.

## Expected

Written before the first run, in the head of `interop/when-full.ts`:

- With the relay above full (`--max-sessions 2`: the publishing page and the session of the relay below upstream), a second viewer that comes to it is sent to the relay below: the relay above prints `SESSION_SENT_AWAY` with the URL of the relay below and `?sent-away=1`, the page says `moved` with that URI, and then plays at the relay below.
- With the relay below full as well (`--max-sessions 1`, its first viewer) and naming the relay above, the relay above sends the second viewer away once, the relay below closes it with `too many sessions` and sends nothing on, and in 5 seconds the relay above sends no second session away; the viewer draws no frame.

## Environment

Apple M3 Pro, macOS 14.4, Chromium of Playwright; the own relay (debug build) on 34500 and 35500 (35501 and 34501 for the second case), the bars of `interop/video.ts` published by a page (`keepVideoPublished`) and played by two (`keepVideoPlayed`, which connects to the URI of a GOAWAY with the certificate hash of the run). `mise run interop -- --base-port 34500 --only "when full"`.

## Result

**As expected**, both cases, in one run:

| Case | Took | What the second viewer said |
|---|---|---|
| moves | 4.1 s | connecting, moved `https://127.0.0.1:35500?sent-away=1`, connecting, playing 1 |
| both-full | 8.3 s | connecting, moved `https://127.0.0.1:35501?sent-away=1`, connecting, failed, connecting, failed, … |

- In the first case the relay above logged `SESSION_OPENED`, `SESSION_SENT_AWAY` and `SESSION_CLOSED` for the session: the page closed it itself on the GOAWAY, before the 2,000 ms of its Timeout. At the relay below the page played through the session upstream the first viewer had opened.
- In the second the page kept asking the relay below, which closed it each time: `keepPlaying` connects to the URI of the last GOAWAY until another comes. Nothing went back to the relay above.

The relay's tests have the same on in-memory connections (`crates/relay/src/tests.rs`): the URI with and without a query of its own, a session that came with the mark, a relay without `--when-full`, the refusal with `GOING_AWAY` of what the session asks, the close with `GOAWAY_TIMEOUT` at the end of the Timeout, and the 64 sessions that may be being sent away at once.

## Not looked at

- The demo's watching page, which now connects to the URI of a GOAWAY: not run.
- The terminal viewer and the agent's reader at a full relay: they follow GOAWAY as in the other scenes, and were not run here.
- The chat's session of a page: `@xmoq/chat` does not follow GOAWAY (known gap, [design draft](../scale-and-meetings.md)).
- An edge that comes to a full relay above it: it is counted and sent away like a viewer.
