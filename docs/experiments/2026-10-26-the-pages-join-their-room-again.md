# The pages join their room again

- Date: 2026-10-26
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, the demo: a page whose socket to the room had closed asked the room for a token on it for ever (`joinRoom` of `demo/shared.ts`), and the connection that waited for the token was never made; a meeting page said "open the page again".

## Objective

With the room's socket opened again whenever it closes (`openRoomSocket` of `demo/room-socket.ts`), and what was asked on it answered with nothing when it closes, does a watching page that waited for a token play once the room is back, does a publishing page say again where it publishes, and does a meeting page take its seat again?

**Loopback, one machine.**

## Expected

Written after the first run of the fixed pages and before the others (no Expected was written before the first one):

- The publishing page's socket is closed by the test, refused for 3 s, and through after that: the page's count is there again, and the room knows its broadcast, so a watching page opened after it plays.
- The watching page's socket is closed as the page asks for the token of the broadcast's namespace (`sub`, `xmoq/demo`), and refused for 3 s: the page gets no token, the relay with `--auth` refuses the connection, and the page plays once the room is back, after more than one connection.
- With the asks left unanswered when the socket closes, the watching page stays at "connecting" with no frame.
- A meeting page whose socket is closed after it was seated says that it joins again, and sends `join-meeting` again on the next socket.

## Environment

macOS 14.4, arm64, Chromium of Playwright 1.64.0, headless; a demo of its own, `bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787 --auth`, on the own relay (debug build). Each page's room socket goes through `page.routeWebSocket` of Playwright, which closes it (`close()` of the route) and refuses the next for 3 s by closing them as they come. `serve.ts` builds the pages as it starts, so it was started again for each version of the pages.

## Result

**As expected.**

- The publishing page: sockets 2 to 4 were refused, the 5th went through, and the count was 1 again. The watching page opened 6 s after the cut found the broadcast.
- The watching page, cut at its token for `xmoq/demo`: sockets 2 to 4 refused, the 5th through; it played (317 frames in 15 s, the first after 4,173 ms) after 5 connections. Cut at its first token instead (that of the transcript), it played after 1 connection (319 frames, the first after 4,093 ms).
- With `resolve(undefined)` on close taken out: cut at the token for `xmoq/demo`, it stayed at "connecting", 0 frames and 1 connection, though the room was back after 3 s. Cut at the transcript's token, it played (318 frames): the broadcast's token was asked on the next socket, so that run does not tell the two apart.
- The meeting page: seated and publishing 6 s after it opened; 1 s after the cut it said "the room's socket closed: joining it again"; 6 s after, "seated" and still publishing, with `join-meeting` sent once more on the new socket (and three times at first: the publishing, the attending and the chat each ask for the seat).

## Not looked at

- A room that is gone for good: the page tries again every second for as long as it is open.
- A meeting page whose socket came back while somebody else had taken the last seat: by the code, it says the meeting is full and the connections it has go on; this was not run.
- Whether a person sees anything of this: the pages were looked at through `document.body.dataset` alone.
