# @xmoq/chat

A participant of the chat over MoQ tracks, for a page: it says its lines on a track of its own and reads the lines of the room from the gatherer, [`xmoq-chat`](../../crates/chat/README.md). The tracks were decided on 2026-10-24; their names, payloads and limits are the same as in [`xmoq-chat-tracks`](../../crates/chat-tracks/README.md), which the gatherer and the terminal viewer use.

```ts
import { joinChat } from "@xmoq/chat";

const participant = await joinChat(connection, {
  room: "demo",
  name: "alice",
  onLine: (line) => show(line), // { number, name, text, sentAt, at, how: "live" | "history" }
});
const outcome = await participant.say("hello"); // sent, kept-back (too-long, too-fast), or not-sent
```

`joinChat` announces `xmoq/chat/<room>/<name>` (PUBLISH_NAMESPACE), accepts the relay's SUBSCRIBE for `said` there, and subscribes to `lines` of `xmoq/chat/<room>`. **It names the `serve` handlers of the connection**, so it wants a connection of its own: a SUBSCRIBE for another track, and every FETCH, are refused.

## What it does

| Part | What it does |
|---|---|
| Saying | Each line is a Group of `said` with one Object, 0, on a stream of its own that ends the Group. The Groups are counted up from the time in milliseconds when `joinChat` was called, so that a page that comes again on the same name begins above what it used before (§2.4). A SUBSCRIBE_OK carries the last Group written as LARGEST_OBJECT (§10.2.11). A line said before the gatherer subscribed is not sent: the gatherer subscribes from the newest Object on, and would never get it |
| The limits | A line over the limits is kept back and `say` says why, so that the page can tell whoever typed it; the gatherer would drop it without a word to the sender. 512 bytes of text and 5 lines in any second by default, the gatherer's; `limits` sets them, from 1 to 8,192 bytes and to 1,000 lines a second, as the gatherer's flags may. The rate is counted as the gatherer counts it, but where the line leaves and not where it arrives, so lines that come bunched up can still be dropped there. A text with a surrogate that has no partner is kept back as well: no reader takes it |
| Reading | One SUBSCRIBE for the room's `lines`, with RENDEZVOUS_TIMEOUT 10 s (the gatherer publishes a room only once somebody is in it, and the first participant comes before that), and, where its SUBSCRIBE_OK has LARGEST_OBJECT, an Absolute Joining FETCH from that Group less 15, and from 0 where there are fewer: the newest 16 lines (§10.12.2.2; WHY not a Relative Joining FETCH at `historyStart` of `src/core/history.ts`, the same as the terminal viewer). A line is handed on once, by its number, with `how` saying whether it came live or by the FETCH. When the subscription ends (the gatherer started again, or the room was not published in time), it is made again 2 s later, with a FETCH again, for as long as the session lasts |
| Payloads | `{"text","sentAt"}` is written, and `{"name","text","sentAt","at"}` read, as the Rust side does. An Object of `lines` that is no line is passed over and reported with why (`not-an-object`, `name`, `text`, `sent-at`, `at`) |

**The payloads are read with a JSON reader of this package**, not with `JSON.parse`: the same bytes are to give the same line or the same reason as `serde_json` gives the gatherer, and `JSON.parse` takes `1e3`, `1.0` and `-0` for times, `1e400` for Infinity, an escaped lone surrogate, and nesting of any depth. Why each, at the top of `src/core/json.ts`. Both read the shared rows of [`fixtures/chat-tracks/`](../../fixtures/chat-tracks/README.md) in their tests.

## Bounds

What a participant holds of what comes has a bound that no option removes. **All are provisional.**

| What | Bound | Where |
|---|---|---|
| Lines of the history | The newest 16 (`HISTORY_LINES`); a FETCH that brings more is not read further | `src/moqt/participant.ts` |
| Line numbers remembered, so that each line is handed on once | 200, the newest; a line older than all of them is passed over | `SEEN_LINES` |
| One line read from the FETCH or the subscription | 64 KiB of payload, what the gatherer reads of one stream of `said`; the subscription is given it as its own `maxObjectPayloadLength`, so a longer Object ends its stream before it is gathered | `MAX_LINE_PAYLOAD` |
| Lines being written at once | 16; one more is not sent (`busy`) | `MAX_WRITING` |
| The Joining FETCH | 15 s from when it is sent | `FETCH_TIMEOUT_MS` |
| Times taken in the rate | `linesPerSecond`, 1,000 at the most | `RateWindow`, `src/core/limits.ts` |

## Where it is in the layers

Layer 1, beside `@xmoq/msf`: it depends on `@xmoq/moqt` alone (`package_layer` in `scripts/ci/check-layers.sh`). The decoding of the payloads, the limits and which lines were had are in `src/core`, which does no input or output and is type-checked without the DOM (`tsconfig.core.json`); `src/moqt/participant.ts` speaks on the connection.

WHY a package and not a part of `demo/`: the reader of the payloads is a second implementation of what `xmoq-chat-tracks` reads, and its tests read the shared table; a test under `demo/` would run in no check (`mise run check` leaves `demo/` out). The interop run (`interop/chat-pages.ts`) drives the same `joinChat` the demo's pages use, which it could not do with code of `demo/`, which nothing may depend on. WHY NOT a part of `@xmoq/media`: the chat has nothing of media, and would sit in layer 2 for no reason.

## Not done

- Lines said while nobody has subscribed to `said` (before the gatherer is there, or while it is away) are not sent and not kept for later.
- A line between the newest 16 and what a page last had, after the subscription was made again, is not fetched: the FETCH asks for the newest 16 every time.
- The namespace is not withdrawn when the participant stops; that happens when the session closes.
- No authorization: the gatherer sends no token, and a relay that asks for tokens refuses it.
