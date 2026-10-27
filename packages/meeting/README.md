# @xmoq/meeting

A participant of a meeting in a page. Every participant publishes a broadcast of its own under `xmoq/meet/<room>/<id>`, and finds the others with one SUBSCRIBE_NAMESPACE for `xmoq/meet/<room>` ([architecture](../../docs/architecture.md), meetings). This package receives: it reads each one's catalog, and receives the sound of all of them and the small video of the first `mostWatched` who came. How many are watched and how many others are kept are the application's to say. Publishing the page's own broadcast is done with [`@xmoq/media`](../media/README.md) and [`@xmoq/msf`](../msf/README.md), as any broadcast is, and playing what is received is the caller's. The page can also write what happens in the meeting for an agent on the same machine (`createAgentBridge`).

Who is there, in which order, and who has left after a new session is the rule of [`fixtures/meeting-presence/`](../../fixtures/meeting-presence/README.md) (`createPresence`). What is received of them follows it: a participant's catalog is followed, and asked for again after it ends or fails; the chosen tracks of a catalog are subscribed to, and subscribed anew where a catalog changes them; when a participant leaves, its subscriptions are cancelled, and its place among the watched goes to the next who came.

## Writing for an agent: what to know

What is written, and how a line of the inbox is read, is [`fixtures/agent-lines/`](../../fixtures/agent-lines/README.md), laid out as `xmoq-agent --out` lays its own.

- **Chromium alone**: the File System Access API (`showDirectoryPicker`, the permissions of a handle) is in no other browser. Elsewhere the page says so and writes nothing.
- **The folder is chosen once**: the handle is kept in IndexedDB, and a page that comes later asks only to be allowed to write in it again, with a click. Chromium lets a page have no home, desktop or Documents folder itself, only a folder in them; hence `~/Documents/xmoq`.
- **A change of a file is not told**: the page reads `inbox.txt` again every `INBOX_EVERY_MS`, so a line an agent writes is done up to that much later. What is in it when the page begins is passed over. An agent appends to it (`>>`); a file written anew (`>`) is read from its beginning.
- **`events.jsonl` is another file after each write**: Chromium writes a copy and moves it over the file (the WHY at `flush` in `src/web/agent-folder.ts`), so an agent follows it by its name, `tail -F`, and not `tail -f`. Each write copies the whole file, which grows with the meeting.
- **The person and the agent share one seat**: what the agent says is said under the page's name in the chat, and its pictures are of what the page receives. **This is an experiment, and stays so**: the lines and words of the others are kept on the person's disk, and nobody else in the meeting is told; the agent's lines go into the chat under the person's name, and the others cannot tell them from the person's.
- **Closing the page ends the writing**: what was written stays in the folder.
