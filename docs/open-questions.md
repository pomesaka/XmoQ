# Open decisions

What the project owner has not decided. Do not infer answers: ask.

When an item is decided, remove it here and write the decision where it takes effect, as WHY and WHY NOT comments in the code or in the document that covers the topic. Decisions are not collected in one place.

## Undecided

- When to move the MoQ implementation from draft 18 to a newer draft
- The codec beyond H.264 in Annex B (video) and Opus (audio). Facts are in the [WebCodecs loopback record](experiments/2026-10-10-webcodecs-loopback.md) and the [record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md).
  - The H.264 profile and level, and whether the hardware encoder is asked for, left to the browser, or avoided
  - The key frame interval, and whether a receiver can ask for a key frame
  - What a receiver does when a chunk is missing (H.264 and VP8 decoders show corrupt pictures without an error)
  - `latencyMode` and `optimizeForLatency`; whether `duration` travels; whether the resolution may change during a broadcast
  - Whether the Opus `description` is passed to the decoder (Chromium decodes without it, [record](experiments/2026-10-11-opus-audio-page-to-page.md))
- Whether `playVideoTrack` of `@xmoq/media` asks for a delivery timeout on a video track without being told. Today it asks for nothing unless told; the demo's watching page asks for 150 ms (`demo/watch.ts`). Measurements are in [the A/V demo and its limits](av-demo.md).
- Whether a browser client ends a subscription itself when its application does not read. Today it reads a bounded number of streams and leaves the rest to the transport, so that the relay ends the subscription (`TOO_FAR_BEHIND`); a page that never reads is not told and loses its session later ([record](experiments/2026-10-10-subscription-backlog-in-chromium.md)).
- Whether a Subgroup whose stream was cut short is continued from another source of the track. Not done; the draft allows it (§2.2, §11.4.3).
- Supported browsers beyond Chromium on desktop, operating systems, Ghostty versions
- The CLI's audio output. `cpal` is used, in `crates/watch/src/bin/xmoq-watch/audio_output.rs` alone, so that it can be changed; nobody has listened to it ([record](experiments/2026-10-12-opus-decoded-and-played-with-rust-crates.md)).
- Authorization beyond the form of the token (a token of the room on every request and on SETUP, `crates/relay/src/auth.rs`): who may get a token (the room of the spike gives one to anyone), how long one lives, and what happens to a subscription whose token has expired
- Authentication and SSO, privacy and moderation
- What the relay and the room say to each other once there is authorization, and where the room lives: a Durable Object can neither accept nor open a WebTransport session
- Hosting, TLS, monitoring and operating costs
- How long the transcriber keeps a transcript: today the latest pieces in memory and nothing on disk (`crates/transcriber/README.md`). Whether it is kept on disk, for how long, and whether the people present are shown it
- An agent in a broadcast or a meeting:
  - What the CLI writes and in what form, and whether an agent is given pictures. `xmoq-agent` writes lines of JSON and pictures asked for ([its README](../crates/agent/README.md)), as a first version
  - Whether an agent ever speaks with a voice
  - How the people present learn that an agent takes part and transcribes them, whether they have to agree to it, and where their sound may be sent
- Spreading a broadcast over relays: who sends a viewer to which relay, and whether an edge learns the namespaces of its upstream with SUBSCRIBE_NAMESPACE ([design draft](scale-and-meetings.md))
- A meeting beyond its first version: the large video of who speaks, whether a mixer beside the relay is added, who may enter, and the meeting's delay ([design draft](scale-and-meetings.md))
- Recording, replay, captions and accessibility: **not discussed**
- The contract with events of a separately developed SNS: **not discussed**
- The OSS license and contribution rules

## Provisional values

A value chosen to get going is marked **provisional** where it is set, with why it is what it is. It is not decided: change it with a measurement, or ask. Where they are:

- The receive limits of the MoQ implementation, the limits and times of the relay's routing, and the limits of the Rust client: [implementation notes](moqt-implementation-notes.md), and the constants they name
- What the relay holds and how many sessions it takes: `HubConfig` in `crates/relay/src/hub.rs`, each set by a flag of the relay
- What `@xmoq/media` holds, waits for and drops when it sends and plays video and sound: [its README](../packages/media/README.md)
- The catalog of `@xmoq/msf`: [its README](../packages/msf/README.md)
- When a client asks again, on its session and for the next session: `packages/moqt/src/core/again.ts` and [`crates/again`](../crates/again/README.md), with the rows both read in [`fixtures/ask-again/`](../fixtures/ask-again/README.md)
- The chat's gatherer, the transcriber and how the sound is cut: the READMEs of `crates/chat`, `crates/transcriber` and `crates/speech`
