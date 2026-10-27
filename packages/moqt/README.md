# @xmoq/moqt

MoQ Transport for the browser, written from the text of `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`): the client end of a session, its requests and the ones of the peer it serves, and when a client asks again and follows a GOAWAY (`src/core/again.ts`, `src/webtransport/keep-moving.ts`). `src/core/` does no input or output; `src/webtransport/connect.ts` alone touches the browser's `WebTransport`.

How the two implementations read what the draft leaves open is in the tables of `fixtures/`, which both read in their tests, and in the comments of the code, with the section of the draft. What of the draft is not met, and the readings no test can hold, are in the [implementation notes](../../docs/moqt-implementation-notes.md).

## Known gaps

These are no requirement of the draft.

- **A subscription a page accepted holds a place for as long as it lasts.** Details at `maxPeerBidiStreams` (`src/webtransport/connection.ts`).
- **`PublishedTrack.ended` after a PUBLISH_DONE that could not be written.** Details at `PublishedTrack.done` (`src/webtransport/connection.ts`).
