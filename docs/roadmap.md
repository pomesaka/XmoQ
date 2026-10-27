# Roadmap (proposed, no committed dates)

What is left to do, by Phase. A Phase with nothing under its heading is done; what was built is in the [requirements](requirements.md), and the runs of each Phase are in the [experiment records](experiments/README.md).

## Phase 0 — Feasibility

## Phase 0b — Own MoQ Transport implementation (draft 18)

## Phase 1 — Video

## Phase 2 — Audio and quality

## Phase 3 — Ghostty CLI

- [ ] Native audio output and terminal controls. Open: nobody is recorded to have heard the terminal viewer's sound; on Linux its sound output has not opened, as no machine with a device was tried ([record](experiments/2026-10-24-one-video-track-to-many-viewers-on-linux.md#xmoq-watch-on-linux)); the output is `cpal`, provisional and [undecided](open-questions.md).
- [ ] Measure FPS, CPU and memory. Open: what a terminal that draws the picture adds, and how many frames a second it shows ([the run with the picture written to /dev/null](experiments/2026-10-12-the-terminal-viewer-at-720p-and-1080p.md)).

**Exit:** reproducible terminal demo with compatibility notes.

## Phase 4 — Rooms and interactions

- [ ] Design room and session identity. Open: who may make or enter a room.
- [ ] Validate Durable Objects integration. Open: nothing was deployed to Cloudflare.
- [ ] Validate the chat and reactions. Open: a person looks at the chat in a browser and a terminal; the terminal viewer with its two tokens; a terminal viewer and an agent in the count of reactions, and many senders at once ([requirements](requirements.md), rows "Chat" and "Reactions").
- [ ] Design and test authorization before broader rollout. Open: who may get a token.

**Exit:** managed room with verified browser/CLI interactions.

## Phase 5 — An agent follows a broadcast

- [ ] Measure the time from a word being said to its text being written, and what the transcription takes of the machine. Open: the processor and memory the transcriber takes, and any machine but one Mac ([how many voices one transcriber reads](experiments/2026-10-25-how-many-voices-one-transcriber-reads.md)).

**Exit (proposed):** an agent follows what is said and written in a broadcast and answers in its chat, reproducibly, on one machine.

## Phase 6 — Meetings

- [ ] Decide the choices of the [design draft](scale-and-meetings.md) beyond the first version ([open decisions](open-questions.md)).
- [ ] Each participant receives the video it shows and the sound of those who speak, within its link. Open: the large video of who speaks, what fits its link, a person hearing the sound, and more than 3 pages.
- [ ] Measure a meeting of 49 on one relay: what each participant receives, the relay's load, the delay from mouth to ear. Not begun: the runs so far had 3 participants. What it would reach of the relay's limits is reasoned in the [design draft](scale-and-meetings.md) ("Limits of the relay a meeting reaches"), not measured.

**Exit (proposed):** a meeting of 49 participants with an agent among them, measured on one relay.

## Phase 7 — Future

- [ ] Relay scale and load testing, toward the goals in the [requirements](requirements.md) (row "Latency, capacity"), by the [proposed design](scale-and-meetings.md). Open: the delay a second hop adds on its own machine; an origin with 20 edges; a page with both tracks on one session; and a broadcast of 10,000, which takes about 21 machines or a scaled-down run with fewer viewers an edge ([the one run with two edges](experiments/2026-10-25-one-video-track-through-two-edges.md)).
- [ ] SNS event integration contract.

**Exit:** evidence-based design decisions, not assumed launch.
