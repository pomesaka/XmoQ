# @xmoq/chat

A participant of the chat over MoQ tracks, for a page: it says its lines on a track of its own and reads the lines of the room from the gatherer, [`xmoq-chat`](../../crates/chat/README.md). The tracks were decided on 2026-10-24; their names, payloads and limits are the same as in [`xmoq-chat-tracks`](../../crates/chat-tracks/README.md), which the gatherer and the terminal viewer use.

```ts
import { joinChat } from "@xmoq/chat";

const participant = await joinChat(connection, {
  room: { kind: "broadcast", name: "demo" }, // or "meeting": a room of each kind has a chat of its own
  name: "alice",
  onLine: (line) => show(line), // { number, name, text, sentAt, at, how: "live" | "history" }
  onGap: (gap) => showGap(gap), // { after, before }: lines between them were not fetched
});
const outcome = await participant.say("hello"); // sent, kept-back (too-long, too-fast), or not-sent
```

`joinChat` announces `xmoq/chat/<kind>/<room>/<name>` (PUBLISH_NAMESPACE), accepts the relay's SUBSCRIBE for `said` there, and subscribes to `lines` of `xmoq/chat/<kind>/<room>`. **It names the `serve` handlers of the connection**, so it wants a connection of its own: a SUBSCRIBE for another track, and every FETCH, are refused. When its `signal` aborts, the participant withdraws its namespace (§6.2), and the relay tells the gatherer with NAMESPACE_DONE, which has the gatherer end its SUBSCRIBE for `said`; the session stays open.

## What it does

| Part | What it does |
|---|---|
| Saying | Each line is a Group of `said` with one Object, 0, on a stream of its own that ends the Group. The Groups are counted up from the time in milliseconds when `joinChat` was called, so that a page that comes again on the same name begins above what it used before (§2.4). A SUBSCRIBE_OK carries the last Group written as LARGEST_OBJECT (§10.2.11). A line said before the gatherer subscribed is not sent: the gatherer subscribes from the newest Object on, and would never get it |
| The limits | A line over the limits is kept back and `say` says why, so that the page can tell whoever typed it; the gatherer would drop it without a word to the sender. 512 bytes of text and 5 lines in any second by default, the gatherer's; `limits` sets them, from 1 to 8,192 bytes and to 1,000 lines a second, as the gatherer's flags may. The rate is counted as the gatherer counts it, but where the line leaves and not where it arrives, so lines that come bunched up can still be dropped there. A text with a surrogate that has no partner is kept back as well: no reader takes it |
| Reading | One SUBSCRIBE for the room's `lines`, with RENDEZVOUS_TIMEOUT 10 s (the gatherer publishes a room only once somebody is in it, and the first participant comes before that), and, where its SUBSCRIBE_OK has LARGEST_OBJECT, an Absolute Joining FETCH from that Group less 15, and from 0 where there are fewer: the newest 16 lines (§10.12.2.1; WHY not a Relative Joining FETCH at `historyStart` of `src/core/history.ts`, the same as the terminal viewer). A line is handed on once, by its number, with `how` saying whether it came live or by the FETCH. When the subscription ends (the gatherer started again, or the room was not published in time), it is made again after 1 s, or after the Retry Interval of a refusal, by `createAskAgain` of `@xmoq/moqt` (as the terminal viewer asks for the same track), for as long as the session lasts |
| Catching up | The FETCH of a subscription made again asks for the lines after the newest that came (a line or not) up to the new LARGEST_OBJECT, still Absolute Joining (§10.12.2.1), and none where nothing came since. Where they are more than `catchUpLines` (200 by default, what the demo's page shows), it asks for the newest `catchUpLines` and calls `onGap` with the numbers around the lines it did not, before those lines come. `lastHad` gives a participant of a new connection the newest line the caller has, so that its first FETCH catches up the same way instead of bringing the newest 16. A LARGEST_OBJECT below what came, or none, means a gatherer with another store numbered the room anew (its own store only numbers on): the numbers that came are forgotten, so that the new lines are handed on, and the newest 16 are fetched as on coming. The rule is `historyToFetch` of `src/core/history.ts`, with why |
| Tokens | For a relay that asks for AUTHORIZATION TOKENs (§10.2.2): the token for reading the room is the connection's (`authorizationToken` of `connect`), which goes into its SETUP, the SUBSCRIBE for `lines` and the Joining FETCH; `publishNamespaceToken` is one for announcing `xmoq/chat/<room>/<name>`, and only the PUBLISH_NAMESPACE carries it, after the connection's. In the demo's relay these are `sub` for `xmoq/chat/<room>` and `pub` for `xmoq/chat/<room>/<name>`, which the room gives. The participant asks for no token and makes none anew: a token that ran out stays on the connection, so a caller that connects again gets new ones (the demo asks the room each time) |
| Across sessions | `keepInTheRoom` connects (`connect`, called anew for each session, so that a caller can ask for new tokens), joins with `joinChat` (`join` gives its options each time, `lastHad` among them), and connects again; the pause between sessions and what a GOAWAY (§10.4) does are `keepMoving`'s ([`@xmoq/moqt`](../moqt/README.md), "Asking again"). A GOAWAY also ends a session that comes before the PUBLISH_NAMESPACE is answered, and the next `connect` is handed it: the caller connects to its New Session URI, with the certificate it had, and to the same place where the URI is empty. WHY it moves before it announces on the new session: one name in two sessions is two publishers of `said`. Run at a full relay that sends sessions away (`interop/when-full.ts`, [record](../../docs/experiments/2026-10-25-the-chat-follows-a-goaway.md)) |
| In a page | `showChat(view, options)` (`src/web`) keeps a participant in the room with `keepInTheRoom` and shows its lines in `view.log`, one `div` each by its number, a gap as one dimmed line, the newest `shownLines`; it says what is typed in `view.form` and how the chat goes in `view.note`. The page gives the elements, the limits, how many lines it catches up and shows, and the connection |
| Payloads | `{"text","sentAt"}` is written, and `{"name","text","sentAt","at"}` read, as the Rust side does. An Object of `lines` that is no line is passed over and reported with why (`not-an-object`, `name`, `text`, `sent-at`, `at`) |

**The payloads are read with the JSON reader of [`@xmoq/json`](../json/README.md)**, not with `JSON.parse`: the same bytes are to give the same line or the same reason as `serde_json` gives the gatherer, and `JSON.parse` takes `1e3`, `1.0` and `-0` for times, `1e400` for Infinity, an escaped lone surrogate, and nesting of any depth. Why each, at the top of `packages/json/src/json.ts`. Both read the shared rows of [`fixtures/chat-tracks/`](../../fixtures/chat-tracks/README.md) in their tests.

## Bounds

What a participant holds of what comes has a bound that no option removes. **All are provisional.**

| What | Bound | Where |
|---|---|---|
| Lines of the history | The newest 16 (`HISTORY_LINES`) on coming; a FETCH that brings more than its range is not read further | `src/core/history.ts`, `src/moqt/participant.ts` |
| Lines caught up | `catchUpLines`, 200 by default (`CATCH_UP_LINES`); a setting takes it from 1 to 1,000 (`CATCH_UP_LINES_MOST`), the lines one FETCH of the gatherer gets by default | `src/core/history.ts` |
| Line numbers remembered, so that each line is handed on once | 200, or `catchUpLines` where that is more, the newest; a line older than all of them is passed over | `SEEN_LINES` |
| One line read from the FETCH or the subscription | 64 KiB of payload, what the gatherer reads of one stream of `said`; the subscription is given it as its own `maxObjectPayloadLength`, so a longer Object ends its stream before it is gathered | `MAX_LINE_PAYLOAD` |
| Lines being written at once | 16; one more is not sent (`busy`) | `MAX_WRITING` |
| The Joining FETCH | 15 s from when it is sent | `FETCH_TIMEOUT_MS` |
| Times taken in the rate | `linesPerSecond`, 1,000 at the most | `RateWindow`, `src/core/limits.ts` |

## Where it is in the layers

Layer 1, beside `@xmoq/msf` (`package_layer` in `scripts/ci/check-layers.sh`). The decoding of the payloads, the limits and which lines were had are in `src/core`, which does no input or output and is type-checked without the DOM (`tsconfig.core.json`); `src/moqt/participant.ts` speaks on the connection, and `src/moqt/keep-in-the-room.ts` keeps one across sessions.

WHY a package and not a part of `demo/`: the reader of the payloads is a second implementation of what `xmoq-chat-tracks` reads, and its tests read the shared table; a test under `demo/` would not run (`mise run check` runs the tests of `packages/` alone). The interop run (`interop/chat-pages.ts`) drives the same `joinChat` the demo's pages use, which it could not do with code of `demo/`, which nothing may depend on. WHY NOT a part of `@xmoq/media`: the chat has nothing of media, and would sit in layer 2 for no reason.

## Not done

- Lines said while nobody has subscribed to `said` (before the gatherer is there, or while it is away) are not sent and not kept for later.
- A FETCH that fails, or ends early, leaves the lines it was to bring missing without an `onGap`, and the next subscription catches up only from the newest line that came. An End of Unknown Range indicator (a gatherer whose `--fetch-lines` is below `catchUpLines`) is passed over without an `onGap` as well.
- A token that runs out while the session lasts is not replaced: the SUBSCRIBE for `lines` made again after its subscription ended is refused from then on, until the caller connects again.
