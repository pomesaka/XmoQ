# Experiment records

One file per experiment run: `YYYY-MM-DD-<kebab-case-title>.md`. Failed runs and blockers are recorded in the same form as successful ones.

What to validate is listed in [validation](../validation.md). A record is written only for commands that were actually run.

## Template

```markdown
# <title>

- Date:
- Commit SHA:
- Roadmap item:

## Objective

## Environment

OS, browser and version, Rust toolchain, exact library versions, MoQ draft revision, certificates.

## Commands

## Observed results

## Limitations and blockers

## Open questions raised
```

<!-- The index below is written by scripts/ci/experiments-index.sh (mise run fmt). -->

## Index

### Phase 0 — Feasibility

- 2026-10-10 [moq-dev: one MoQ object between Chromium and Rust](2026-10-10-moq-dev-object-transfer.md)

### Phase 0b — Own MoQ Transport implementation (draft 18)

- 2026-10-10 [`@xmoq/moqt` waits for the subscriber after PUBLISH_DONE](2026-10-10-browser-publish-done-wait.md)
- 2026-10-10 [A client whose handshake with the relay never ends](2026-10-10-client-handshake-timeout.md)
- 2026-10-10 [The Rust client binds a socket of the server's address family](2026-10-10-client-socket-family.md)
- 2026-10-10 [Clients that leave, through the own relay (MOQT draft 18)](2026-10-10-disconnects-through-own-relay.md)
- 2026-10-10 [Own Rust relay against moqtail peers and the own Rust client (MOQT draft 18)](2026-10-10-own-relay-vs-moqtail-peers.md)
- 2026-10-10 [Own Rust and TypeScript MOQT clients through the moqtail relay (draft 18)](2026-10-10-own-rust-and-ts-through-moqtail-relay.md)
- 2026-10-10 [Own Rust client against the moqtail relay and browser (MOQT draft 18)](2026-10-10-own-rust-client-vs-moqtail.md)
- 2026-10-10 [Own TypeScript MOQT client against moqtail peers (draft 18)](2026-10-10-own-ts-client-vs-moqtail.md)
- 2026-10-10 [The relay core returns the same actions in the same order for the same events](2026-10-10-relay-core-fixed-order.md)
- 2026-10-10 [The relay core before and after its restructuring answer the same events alike](2026-10-10-relay-core-restructuring.md)
- 2026-10-10 [The relay core keeps the sources of a track in a list](2026-10-10-relay-core-source-list.md)
- 2026-10-10 [Routing by namespace in the own relay](2026-10-10-relay-namespace-routing.md)
- 2026-10-10 [The relay takes the PUBLISH of a publisher whose earlier subscription still drains](2026-10-10-relay-publish-anew-while-draining.md)
- 2026-10-10 [The relay takes a PUBLISH for a track that waits for an answer or drains](2026-10-10-relay-publish-for-waiting-and-draining-tracks.md)
- 2026-10-10 [The relay takes a track from several publishers that send PUBLISH](2026-10-10-relay-several-publish-sources.md)
- 2026-10-10 [The relay takes each Subgroup from one stream](2026-10-10-relay-subgroup-selection.md)
- 2026-10-10 [Routing from a moqtail-ts publisher, and how soon the relay notices a vanished peer](2026-10-10-routing-follow-ups.md)
- 2026-10-10 [A subscription nobody reads, in `@xmoq/moqt` and in Chromium](2026-10-10-subscription-backlog-in-chromium.md)
- 2026-10-10 [The code and reason of a session close from `@xmoq/moqt` in Chromium](2026-10-10-webtransport-close-info-in-chromium.md)
- 2026-10-10 [Stream resets and STOP_SENDING from `@xmoq/moqt` in Chromium](2026-10-10-webtransport-error-in-chromium.md)
- 2026-10-10 [WebTransport library comparison: wtransport and web-transport-quinn](2026-10-10-webtransport-library-comparison.md)
- 2026-10-10 [Switch of the Rust WebTransport library to web-transport-quinn](2026-10-10-webtransport-library-switch.md)
- 2026-10-11 [The relay asks every publisher of a matching namespace](2026-10-11-relay-ask-every-publisher.md)
- 2026-10-11 [The relay finds a Subgroup that comes again with another Publisher Priority](2026-10-11-relay-subgroup-priority-mismatch.md)
- 2026-10-12 [A request that is not answered is given up](2026-10-12-a-request-that-is-not-answered-is-given-up.md)
- 2026-10-12 [A request that waits for a stream](2026-10-12-a-request-that-waits-for-a-stream.md)
- 2026-10-13 [A FETCH from the relay's cache that is read slowly](2026-10-13-a-fetch-from-the-relays-cache-that-is-read-slowly.md)
- 2026-10-13 [A session that announces a namespace again and again](2026-10-13-a-session-that-announces-a-namespace-again-and-again.md)
- 2026-10-13 [A session that asks for a new track again and again](2026-10-13-a-session-that-asks-for-a-new-track-again-and-again.md)
- 2026-10-13 [A subscriber asks for a new Group](2026-10-13-a-subscriber-asks-for-a-new-group.md)
- 2026-10-13 [What a refusal says of when to ask again](2026-10-13-what-a-refusal-says-of-when-to-ask-again.md)
- 2026-10-14 [A FETCH goes to the next publisher](2026-10-14-a-fetch-goes-to-the-next-publisher.md)
- 2026-10-24 [An Object past the end of a track, at the relay](2026-10-24-an-object-past-the-end-of-a-track.md)
- 2026-10-24 [The Request IDs of requests the Rust client refuses](2026-10-24-the-request-ids-of-requests-the-rust-client-refuses.md)
- 2026-10-25 [The relay checks a Subgroup sent again](2026-10-25-the-relay-checks-a-subgroup-sent-again.md)

### Phase 1 — Video

- 2026-10-10 [WebTransport and WebCodecs per browser: published sources, and Firefox measured](2026-10-10-browser-support-webtransport-webcodecs.md)
- 2026-10-10 [What following LOC and MSF takes: the drafts, FETCH, and who else implements them](2026-10-10-loc-msf-fetch-survey.md)
- 2026-10-10 [Two publishers of one track in the interop matrix](2026-10-10-two-publishers-in-the-interop-matrix.md)
- 2026-10-10 [WebCodecs loopback: encode, copy to bytes, decode in one page](2026-10-10-webcodecs-loopback.md)
- 2026-10-11 [A page answers the FETCH a relay sends it](2026-10-11-a-page-answers-a-fetch.md)
- 2026-10-11 [A page that only announced a namespace answers the relay's SUBSCRIBE](2026-10-11-a-page-answers-the-relays-subscribe.md)
- 2026-10-11 [A page plays the H.264 video of moqlivemock's publisher](2026-10-11-a-page-plays-the-video-of-mlmpub.md)
- 2026-10-11 [A page publishes an MSF catalog, and a page and moqlivemock's subscriber read it](2026-10-11-a-page-publishes-a-catalog.md)
- 2026-10-11 [A page reads the MSF catalog of a publisher that is not ours](2026-10-11-a-page-reads-a-catalog.md)
- 2026-10-11 [A publisher leaves: what the relay tells the subscriber, and when](2026-10-11-a-publisher-leaves-and-the-subscriber-is-told.md)
- 2026-10-11 [A page stops a peer that does not end its direction after a refusal](2026-10-11-a-refused-request-whose-peer-does-not-end-its-direction.md)
- 2026-10-11 [A FETCH that was served leaves its request stream to the subscriber](2026-10-11-a-served-fetch-leaves-its-request-stream-to-the-subscriber.md)
- 2026-10-11 [The relay keeps to the End Group of an AbsoluteRange filter](2026-10-11-absolute-range-end-group-in-the-relay.md)
- 2026-10-11 [A FETCH from a page: the session state of `@xmoq/moqt`, in Chromium against both relays](2026-10-11-fetch-from-a-page.md)
- 2026-10-11 [The own Rust client fetches in the interop matrix](2026-10-11-fetch-from-the-rust-client-through-the-matrix.md)
- 2026-10-11 [The FETCH runs of the interop matrix, with a relay that answers from its cache](2026-10-11-fetch-runs-with-the-relays-cache.md)
- 2026-10-11 [A fetch stream of the moqtail relay, read by the own stream readers](2026-10-11-fetch-stream-read-by-own-readers.md)
- 2026-10-11 [A stream of the peer that does not say what it is, stopped by a page after a time](2026-10-11-first-bytes-timeout-in-chromium.md)
- 2026-10-11 [A Joining FETCH sent together with its SUBSCRIBE, through the own relay](2026-10-11-joining-fetch-sent-with-its-subscribe.md)
- 2026-10-11 [A Joining FETCH through the own relay](2026-10-11-joining-fetch-through-own-relay.md)
- 2026-10-11 [LOC video payloads in Chromium: `avc3`, and frames of moqlivemock's publisher](2026-10-11-loc-video-payloads-in-chromium.md)
- 2026-10-11 [moqlivemock's subscriber takes a finished request stream for a failed FETCH, at times](2026-10-11-mlmsub-joining-fetch-race.md)
- 2026-10-11 [FETCH bytes from moqlivemock and its library](2026-10-11-moqlivemock-fetch.md)
- 2026-10-11 [What the moqtail relay answers to a FETCH, and the bytes it answers with](2026-10-11-moqtail-relay-fetch.md)
- 2026-10-11 [Object Properties through both relays](2026-10-11-object-properties-through-both-relays.md)
- 2026-10-11 [One video track to many viewers through the own relay](2026-10-11-one-video-track-to-many-viewers.md)
- 2026-10-11 [A page encodes and publishes video, and another plays it, through a relay](2026-10-11-pages-send-and-play-video.md)
- 2026-10-11 [Pages without a token, at a relay that checks SETUP: what they are told](2026-10-11-pages-without-a-token-at-a-relay-that-checks-setup.md)
- 2026-10-11 [The tool that compares two relay cores gets worlds with FETCH](2026-10-11-relay-core-diff-with-fetch.md)
- 2026-10-11 [The relay stops a fetch stream it writes itself](2026-10-11-relay-stops-a-fetch-stream-it-writes.md)
- 2026-10-11 [The Rust client answers a Standalone FETCH, through the own relay](2026-10-11-rust-client-answers-a-fetch.md)
- 2026-10-11 [The Rust client cancels a FETCH on its request stream alone](2026-10-11-rust-client-cancels-a-fetch-on-its-request-stream-alone.md)
- 2026-10-11 [The Rust client sends FETCH, through the own relay](2026-10-11-rust-client-fetch.md)
- 2026-10-11 [The sound falls behind on a link with delay and loss, and does not catch up](2026-10-11-sound-falls-behind-on-a-link-with-delay-and-loss.md)
- 2026-10-11 [A Standalone FETCH through the own relay](2026-10-11-standalone-fetch-through-own-relay.md)
- 2026-10-11 [How many streams a page takes from a peer, and how many Chromium permits](2026-10-11-streams-a-page-takes-from-a-peer.md)
- 2026-10-11 [The relay keeps to the Start Location of a Subscription Filter](2026-10-11-subscription-filter-in-the-relay.md)
- 2026-10-11 [The demo over a link with delay and loss: the relay with Cubic and with BBR](2026-10-11-the-demo-over-a-lossy-link-with-bbr.md)
- 2026-10-11 [The relay answers a FETCH from its cache of the latest Group](2026-10-11-the-relay-answers-a-fetch-from-its-cache.md)
- 2026-10-11 [The relay compares an Object that comes again with what its cache holds](2026-10-11-the-relay-compares-an-object-that-comes-again.md)
- 2026-10-11 [The video receiver and sender of `@xmoq/media`, without a connection or WebCodecs](2026-10-11-video-receiver-and-sender-cores.md)
- 2026-10-11 [Where the delay of the demo comes from, and a buffer that follows the link](2026-10-11-where-the-delay-of-the-demo-comes-from.md)
- 2026-10-11 [What a writer fails with in Chromium: a stopped stream, an ended session, a lost connection](2026-10-11-write-abort-in-chromium.md)
- 2026-10-12 [A cancelled subscription, and what the relay had not sent yet](2026-10-12-a-cancelled-subscription-and-what-the-relay-had-not-sent-yet.md)
- 2026-10-12 [A later Group of video waits for the end of the one before it: tried, and not put in](2026-10-12-a-later-group-of-video-waits-for-the-one-before-it.md)
- 2026-10-12 [Cubic and BBR over a link with too little room](2026-10-12-cubic-and-bbr-over-a-link-with-too-little-room.md)
- 2026-10-12 [The pages connect again when the relay comes back](2026-10-12-pages-connect-again-when-the-relay-comes-back.md)
- 2026-10-12 [The demo's broadcast has an MSF catalog, and others read it](2026-10-12-the-demo-has-a-catalog.md)
- 2026-10-12 [The relay gives up a Group a viewer does not get in time](2026-10-12-the-relay-gives-up-a-group-a-viewer-does-not-get-in-time.md)
- 2026-10-12 [The relay gives up the older Groups when a newer one begins](2026-10-12-the-relay-gives-up-older-groups-when-a-newer-one-begins.md)
- 2026-10-12 [Video over a link that delays and loses: how late the frames come, and the congestion control](2026-10-12-video-over-a-link-that-delays-and-loses.md)
- 2026-10-14 [A viewer asks for a new Group instead of fetching](2026-10-14-a-viewer-asks-for-a-new-group-instead-of-fetching.md)
- 2026-10-24 [A percent outside a variable reference in a catalog](2026-10-24-a-percent-outside-a-variable-reference-in-a-catalog.md)

### Phase 2 — Audio and quality

- 2026-10-11 [The demo over a link that loses, delays and cuts](2026-10-11-demo-over-a-lossy-link.md)
- 2026-10-11 [The demo plays picture and sound together, and shows the delay](2026-10-11-demo-sync-and-delay.md)
- 2026-10-11 [Opus audio from a page to a page, through the relay](2026-10-11-opus-audio-page-to-page.md)
- 2026-10-12 [A delivery timeout for each version of the video, with a viewer that changes between them](2026-10-12-a-delivery-timeout-for-each-version-of-the-video.md)
- 2026-10-12 [A judge of the versions that counts what the relay gave up, and tries a higher version before it shows it](2026-10-12-a-judge-that-counts-what-the-relay-gave-up-and-tries-before-it-goes-up.md)
- 2026-10-12 [A page behind a link with too little room, and a relay that gives Groups up](2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md)
- 2026-10-12 [A delivery timeout, and a playout buffer, that follow how late the sound comes](2026-10-12-a-timeout-and-a-buffer-that-follow-the-sound.md)
- 2026-10-12 [A higher version is tried before it is shown, and a try that fails comes again as soon](2026-10-12-a-try-before-going-up-that-comes-again-as-soon.md)
- 2026-10-12 [A viewer changes the delivery timeout of its subscription with REQUEST_UPDATE, and the relay follows](2026-10-12-a-viewer-changes-its-delivery-timeout-with-request-update.md)
- 2026-10-12 [Delivery timeouts of a subscription: what draft 18 has, who does what, and the viewer's side made ready](2026-10-12-delivery-timeouts-of-a-subscription-what-draft-18-has-and-who-does-what.md)
- 2026-10-12 [The relay follows the delivery timeout a viewer asks for](2026-10-12-the-relay-follows-the-delivery-timeout-a-viewer-asks-for.md)
- 2026-10-12 [The terminal viewer asks for a delivery timeout](2026-10-12-the-terminal-viewer-asks-for-a-delivery-timeout.md)
- 2026-10-12 [The terminal viewer chooses a version of the video](2026-10-12-the-terminal-viewer-chooses-a-version-of-the-video.md)
- 2026-10-12 [The terminal viewer has the page's automatic buffer](2026-10-12-the-terminal-viewer-has-the-pages-automatic-buffer.md)
- 2026-10-12 [Two versions of the video, and a viewer that changes between them](2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md)
- 2026-10-12 [Which delivery timeout a viewer asks for on its video](2026-10-12-which-delivery-timeout-a-viewer-asks-for-on-its-video.md)
- 2026-10-13 [A page that waits for the broadcast at the relay](2026-10-13-a-page-that-waits-for-the-broadcast-at-the-relay.md)
- 2026-10-13 [A publisher and a viewer whose clocks run at different speeds](2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md)
- 2026-10-13 [The automatic buffer: what raises it, and how fast it comes down](2026-10-13-the-automatic-buffer-what-raises-it-and-how-fast-it-comes-down.md)
- 2026-10-13 [The default playout buffer: 50 ms, 150 ms, or one that follows the link](2026-10-13-the-default-playout-buffer.md)
- 2026-10-13 [The run in which the buffer stood still while the sound was too late](2026-10-13-the-run-in-which-the-buffer-stood-still.md)
- 2026-10-13 [The sound behind the video on a narrow link, and what priorities do for it](2026-10-13-the-sound-behind-the-video-on-a-narrow-link-and-priorities.md)
- 2026-10-14 [A buffer of 100 ms beside 50, 150 and the automatic one](2026-10-14-a-buffer-of-100-ms-beside-50-150-and-the-automatic-one.md)
- 2026-10-14 [A piece of sound a stream, measured beside ten](2026-10-14-a-piece-of-sound-a-stream-measured-again.md)
- 2026-10-14 [The sound subscribed to again alone, while the picture goes on](2026-10-14-the-sound-subscribed-to-again-alone.md)
- 2026-10-14 [Three minutes behind a narrow link, and the relay's limit for one viewer](2026-10-14-three-minutes-behind-a-narrow-link-and-the-relays-limit-for-one-viewer.md)
- 2026-10-15 [The terminal viewer asks for its sound again](2026-10-15-the-terminal-viewer-asks-for-its-sound-again.md)
- 2026-10-15 [What the sound waits behind on a narrow link](2026-10-15-what-the-sound-waits-behind-on-a-narrow-link.md)
- 2026-10-24 [A GOAWAY on the stream of one request, and how soon the next session stands](2026-10-24-a-goaway-on-the-stream-of-one-request.md)
- 2026-10-24 [A GOAWAY the clients take](2026-10-24-a-goaway-the-clients-take.md)

### Phase 3 — Ghostty CLI

- 2026-10-11 [A broadcast in a terminal, picture and sound, with one command](2026-10-11-a-broadcast-in-a-terminal-picture-and-sound.md)
- 2026-10-11 [A Rust subscriber writes a LOC video track as an H.264 byte stream](2026-10-11-rust-subscriber-writes-loc-video-as-annex-b.md)
- 2026-10-11 [A Rust subscriber writes an Opus track of LOC as an Ogg Opus file](2026-10-11-rust-subscriber-writes-opus-as-ogg.md)
- 2026-10-11 [Video in a terminal with the Kitty graphics protocol, and two ways to decode it](2026-10-11-video-in-a-terminal-with-kitty-graphics.md)
- 2026-10-12 [Keys for the terminal viewer, and following the room's broadcast](2026-10-12-keys-for-the-terminal-viewer.md)
- 2026-10-12 [Opus decoded and played with Rust crates](2026-10-12-opus-decoded-and-played-with-rust-crates.md)
- 2026-10-12 [Picture and sound in the terminal by one clock](2026-10-12-picture-and-sound-in-the-terminal-by-one-clock.md)
- 2026-10-12 [The Opus decoder that overflows, set against libopus](2026-10-12-the-opus-decoder-that-overflows-set-against-libopus.md)
- 2026-10-12 [The terminal viewer at 720p and 1080p](2026-10-12-the-terminal-viewer-at-720p-and-1080p.md)
- 2026-10-12 [The terminal viewer finds its tracks in the catalog](2026-10-12-the-terminal-viewer-finds-its-tracks-in-the-catalog.md)
- 2026-10-12 [The underruns of the terminal viewer](2026-10-12-the-underruns-of-the-terminal-viewer.md)
- 2026-10-13 [A viewer that comes before the broadcast](2026-10-13-a-viewer-that-comes-before-the-broadcast.md)
- 2026-10-21 [Opus decoders in Rust set against libopus](2026-10-21-opus-decoders-in-rust-set-against-libopus.md)
- 2026-10-24 [One video track to many viewers on Linux, and with a track of the shape of sound](2026-10-24-one-video-track-to-many-viewers-on-linux.md)
- 2026-10-24 [The terminal viewer decodes its sound with opus-rs](2026-10-24-the-terminal-viewer-decodes-its-sound-with-opus-rs.md)
- 2026-10-24 [The terminal viewer refuses the requests of the relay](2026-10-24-the-terminal-viewer-refuses-the-requests-of-the-relay.md)

### Phase 4 — Rooms and interactions

- 2026-10-11 [A session the relay closes with a code, as the peer sees it](2026-10-11-a-session-closed-with-a-code-as-the-peer-sees-it.md)
- 2026-10-11 [Tokens a room hands out, checked by the relay](2026-10-11-authorization-tokens-of-a-room-checked-by-the-relay.md)
- 2026-10-11 [A room on a Durable Object, run locally](2026-10-11-durable-objects-room.md)
- 2026-10-12 [Chat as MoQ tracks, beside the room's WebSocket](2026-10-12-chat-as-moq-tracks-beside-the-rooms-websocket.md)
- 2026-10-12 [Reactions over the room's WebSocket, and whether the room has to bundle them](2026-10-12-reactions-over-the-rooms-websocket.md)
- 2026-10-12 [The terminal viewer joins a room](2026-10-12-the-terminal-viewer-joins-a-room.md)
- 2026-10-24 [A page fetches the lines it missed](2026-10-24-a-page-fetches-the-lines-it-missed.md)
- 2026-10-24 [A page withdraws its chat namespace](2026-10-24-a-page-withdraws-its-chat-namespace.md)
- 2026-10-24 [The gatherer of the chat](2026-10-24-chat-gatherer.md)
- 2026-10-24 [The demo's pages chat over the tracks](2026-10-24-demo-chat-over-tracks.md)
- 2026-10-24 [Namespace discovery through the relays](2026-10-24-namespace-discovery-through-the-relays.md)
- 2026-10-24 [Ten rooms of a hundred viewers](2026-10-24-ten-rooms-of-a-hundred-viewers.md)
- 2026-10-24 [The chat off the room's WebSocket](2026-10-24-the-chat-off-the-rooms-websocket.md)
- 2026-10-24 [The gatherer asks a participant again](2026-10-24-the-gatherer-asks-a-participant-again.md)
- 2026-10-24 [The relay on cores of its own, and the writers of a session in one task](2026-10-24-the-relay-alone-and-the-writers-of-a-session-in-one-task.md)
- 2026-10-24 [The relay in the VM and the viewers on the Mac](2026-10-24-the-relay-in-the-vm-and-the-viewers-on-the-mac.md)
- 2026-10-24 [The relay's socket drops what comes in, and the Objects of the publishers come late to the hub](2026-10-24-the-relays-socket-drops-what-comes-in.md)
- 2026-10-24 [Where the relay spends its time on a track of an Object a stream](2026-10-24-where-the-relay-spends-its-time-on-a-track-of-an-object-a-stream.md)
- 2026-10-24 [Why a run of 1,000 viewers with sound falls behind with cores to spare: what it is not](2026-10-24-why-a-run-of-1000-viewers-with-sound-falls-behind-with-cores-to-spare.md)
- 2026-10-25 [A meeting of pages](2026-10-25-a-meeting-of-pages.md)
- 2026-10-25 [A reader joins a room that starts later](2026-10-25-a-reader-joins-a-room-that-starts-later.md)
- 2026-10-25 [A reader of the room for an agent](2026-10-25-a-reader-of-the-room-for-an-agent.md)
- 2026-10-25 [agent-demo starts and stops](2026-10-25-agent-demo-starts-and-stops.md)
- 2026-10-25 [agent-demo with a meeting](2026-10-25-agent-demo-with-a-meeting.md)
- 2026-10-25 [An agent in a meeting](2026-10-25-an-agent-in-a-meeting.md)
- 2026-10-25 [An agent reads the demo with xmoq-agent](2026-10-25-an-agent-reads-the-demo-with-xmoq-agent.md)
- 2026-10-25 [An agent takes a seat in a room's meeting](2026-10-25-an-agent-takes-a-seat-in-a-rooms-meeting.md)
- 2026-10-25 [moqtail's relay and the Joining FETCH of a page's catalog](2026-10-25-moqtail-relay-joining-fetch-of-a-catalog.md)
- 2026-10-25 [Objects before their SUBSCRIBE_OK](2026-10-25-objects-before-their-subscribe-ok.md)
- 2026-10-25 [Pictures of a broadcast for an agent](2026-10-25-pictures-of-a-broadcast-for-an-agent.md)
- 2026-10-25 [SUBSCRIBE_NAMESPACE from a page](2026-10-25-subscribe-namespace-from-a-page.md)
- 2026-10-25 [The chat of a room before its broadcast](2026-10-25-the-chat-of-a-room-before-its-broadcast.md)
- 2026-10-25 [The chat with tokens through the gatherer](2026-10-25-the-chat-with-tokens-through-the-gatherer.md)
- 2026-10-25 [The demo has a meeting page](2026-10-25-the-demo-has-a-meeting-page.md)
- 2026-10-25 [The reader for agents asks for its video again](2026-10-25-the-reader-for-agents-asks-for-its-video-again.md)
- 2026-10-25 [The reader for agents reads the transcript](2026-10-25-the-reader-for-agents-reads-the-transcript.md)
- 2026-10-25 [The reader says its counts before it ends](2026-10-25-the-reader-says-its-counts-before-it-ends.md)
- 2026-10-25 [The room lets 49 into a meeting](2026-10-25-the-room-lets-49-into-a-meeting.md)
- 2026-10-25 [The room names its transcript](2026-10-25-the-room-names-its-transcript.md)
- 2026-10-25 [The room names two transcripts](2026-10-25-the-room-names-two-transcripts.md)
- 2026-10-25 [Who speaks in a meeting, for an agent](2026-10-25-who-speaks-in-a-meeting.md)

### Phase 5 — An agent follows a broadcast

- 2026-10-25 [How many voices one transcriber reads at once](2026-10-25-how-many-voices-one-transcriber-reads.md)
- 2026-10-25 [One rule for the next session](2026-10-25-one-rule-for-the-next-session.md)
- 2026-10-25 [The transcriber reads a voice](2026-10-25-the-transcriber-reads-a-voice.md)

### Phase 6 — Meetings

- 2026-10-25 [A page of a meeting writes for Claude Code](2026-10-25-a-page-writes-for-claude-code.md)
- 2026-10-25 [A voice longer than a piece](2026-10-25-a-voice-longer-than-a-piece.md)
- 2026-10-25 [An agent keeps the meeting over a GOAWAY](2026-10-25-an-agent-keeps-the-meeting-over-a-goaway.md)

### Phase 7 — Future

- 2026-10-25 [A full relay sends the next session away](2026-10-25-a-full-relay-sends-the-next-session-away.md)
- 2026-10-25 [A relay below another relay](2026-10-25-a-relay-below-another-relay.md)
- 2026-10-25 [A relay below another, when the one above goes](2026-10-25-a-relay-below-another-when-the-one-above-goes.md)
- 2026-10-25 [A viewer of an edge when the origin goes](2026-10-25-a-viewer-of-an-edge-when-the-origin-goes.md)
- 2026-10-25 [A viewer of an edge, with attempts to connect every second](2026-10-25-a-viewer-of-an-edge-with-attempts-every-second.md)
- 2026-10-25 [One video track through two edges](2026-10-25-one-video-track-through-two-edges.md)
- 2026-10-25 [The captions of a page follow a GOAWAY](2026-10-25-the-captions-follow-a-goaway.md)
- 2026-10-25 [The chat of a page follows a GOAWAY](2026-10-25-the-chat-follows-a-goaway.md)
- 2026-10-25 [The chat through two relays](2026-10-25-the-chat-through-two-relays.md)
- 2026-10-25 [The meeting of a page follows a GOAWAY](2026-10-25-the-meeting-follows-a-goaway.md)
- 2026-10-25 [Two relays that ask for tokens](2026-10-25-two-relays-that-ask-for-tokens.md)

### Not tied to a Phase

- 2026-10-10 [2026-10-10 — moqtail: one Object between browser and Rust (MOQT draft 18)](2026-10-10-moqtail-object-transfer.md)
- 2026-10-11 [`test.each` of Bun 1.3.12 loses its table](2026-10-11-bun-test-each-loses-its-table.md)
- 2026-10-11 [The demo with the publishing page in a background tab: a frame a second, and no sound](2026-10-11-demo-in-a-background-tab.md)
- 2026-10-11 [The demo in Playwright's Firefox and WebKit](2026-10-11-demo-in-firefox-and-webkit.md)
- 2026-10-11 [`@xmoq/loc` follows another revision by one line: a check with a made-up one](2026-10-11-loc-revision-switch.md)
- 2026-10-11 [Tokens of the room, carried by the pages of the demo to the relay](2026-10-11-the-rooms-tokens-in-the-demo.md)
- 2026-10-12 [One, five or ten pieces of sound a Group, measured again](2026-10-12-one-five-or-ten-pieces-of-sound-a-group.md)
- 2026-10-12 [The demo at 720 and 1080 lines](2026-10-12-the-demo-at-720-and-1080-lines.md)
- 2026-10-12 [The first picture of a viewer who comes late: fetch the Group, or wait for the next](2026-10-12-the-first-picture-of-a-viewer-who-comes-late-fetch-or-wait.md)
- 2026-10-13 [An hour of the demo](2026-10-13-an-hour-of-the-demo.md)
- 2026-10-14 [The page that crashed when the relay came back: with #226 and without it](2026-10-14-the-page-that-crashed-when-the-relay-came-back.md)
- 2026-10-24 [A limit on the payload of Objects for each subscription](2026-10-24-a-limit-on-objects-for-each-subscription.md)
- 2026-10-24 [A silent subgroup stream is stopped, in Rust](2026-10-24-a-silent-subgroup-stream-is-stopped-in-rust.md)
- 2026-10-24 [A silent subgroup stream is stopped](2026-10-24-a-silent-subgroup-stream-is-stopped.md)
- 2026-10-24 [Datagrams read by the browser client](2026-10-24-datagrams-read-by-the-browser-client.md)
- 2026-10-24 [GOAWAY and moqtail](2026-10-24-goaway-and-moqtail.md)
- 2026-10-24 [Immutable Properties in the browser client](2026-10-24-immutable-properties-in-the-browser-client.md)
- 2026-10-24 [The demo's chat with tokens: the pages' side](2026-10-24-the-demo-chat-with-tokens.md)
- 2026-10-24 [The LOC Timestamp inside Immutable Properties, in Rust](2026-10-24-the-loc-timestamp-inside-immutable-properties-in-rust.md)
- 2026-10-24 [The LOC Timestamp inside Immutable Properties](2026-10-24-the-loc-timestamp-inside-immutable-properties.md)
- 2026-10-24 [The page that crashed when the relay came back: not seen again in CI](2026-10-24-the-page-that-crashed-not-seen-again-in-ci.md)
- 2026-10-25 [A terminal viewer pairing that fails only in the full run](2026-10-25-a-terminal-viewer-pairing-that-fails-only-in-the-full-run.md)
- 2026-10-25 [Whether Chromium gives freed connection credit to a stream by its `sendOrder`](2026-10-25-send-order-of-data-streams-in-chromium.md)
- 2026-10-25 [The relay raises the Largest Object at an Object's first byte](2026-10-25-the-relay-raises-the-largest-object-at-its-first-byte.md)
- 2026-10-25 [TRACK_STATUS through the relays](2026-10-25-track-status-through-the-relays.md)
