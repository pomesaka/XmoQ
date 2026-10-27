# @xmoq/chat

A participant of the chat over MoQ tracks, for a page: it says its lines on a track of its own and reads the lines of the room from the gatherer, [`xmoq-chat`](../../crates/chat/README.md). The tracks, their payloads and their limits are those of [`xmoq-chat-tracks`](../../crates/chat-tracks/README.md), which the gatherer and the terminal viewer use.

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

**`joinChat` names the `serve` handlers of the connection** (`serveTracks` of `@xmoq/tracks`), so it wants a connection of its own: a request for another track is refused with DOES_NOT_EXIST. `keepInTheRoom` keeps a participant across sessions, and `showChat` (`src/web`) keeps one in a page. Which lines a participant fetches when it comes and when it catches up is the rule of [`fixtures/chat-history/`](../../fixtures/chat-history/README.md); which token a relay asks for which namespace is at `tokenForNamespace` of `@xmoq/room`.

## Not done

- Lines said while nobody has subscribed to `said` (before the gatherer is there, or while it is away) are not sent and not kept for later.
- The rate of lines is counted as the gatherer counts it, but where a line leaves and not where it arrives, so lines that come bunched up can still be dropped there without a word to the sender.
- A FETCH that fails, or ends early, leaves the lines it was to bring missing without an `onGap`, and the next subscription catches up only from the newest line that came. An End of Unknown Range indicator (a gatherer whose `--fetch-lines` is below `catchUpLines`) is passed over without an `onGap` as well.
- A token that runs out while the session lasts is not replaced: the SUBSCRIBE for `lines` made again after its subscription ended is refused from then on, until the caller connects again.
