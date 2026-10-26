# Who speaks in a meeting, for an agent

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings, the reader for agents. The meeting is **proposed** (`docs/scale-and-meetings.md`, Part 2). What [the record of an agent in a meeting](2026-10-25-an-agent-in-a-meeting.md) left out: the transcript of each participant, and `/frame` alone.

## Objective

In a meeting the transcriber listens to every participant's broadcast under the meeting's prefix, and says whose voice each piece was. Does `xmoq-agent --meeting r` write each participant's speech with its `from`, and give the picture of the one who spoke last for `/frame` alone?

## Expected

Written before the first run, at the top of `interop/agent-meeting-speech.ts`:

- The transcriber is told of both pages' namespaces and subscribes to the audio of each (`AUDIO_SUBSCRIBED namespace=xmoq/meet/r/p1`, and p2).
- The reader writes p1 and p2 as `participant` joined, and then a `speech` line whose `from` is `["xmoq","meet","r","p1"]` and whose text has "relay" (any case); no `speech` line of p2 while p2 is silent.
- `/frame` alone, after that line, is answered with a `frame` line of `"id":"p1"` and a PNG the page decodes, as `agent-frames.ts` checks them.
- Once p1 is silent and p2 speaks, a `speech` line of `from` `["xmoq","meet","r","p2"]` with "relay", and `/frame` alone after it is answered with a `frame` line of `"id":"p2"` and such a PNG.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of playwright-core, headless; the own relay, `xmoq-transcriber` with the smallest model and `--language en`, and `xmoq-agent` of this tree (debug builds), on `main` at `xkytttzr` (`--meeting` in) with this change. Both pages say the same "Hello from the relay." of `fixtures/speech/` and paint the same picture; the transcript's `from` is what tells them apart. `mise run interop -- --only "agent" --base-port 31500`, then the whole matrix with the machine's lock.

## Result

**As expected, in the first run** (6.1 s for the pairing). The reader wrote a piece of p1, "Hello from the relay.", of 1.4 s, read 0.75 s after its end, and no piece of p2 before p2 spoke; `/frame` alone gave p1's picture. Once p2 spoke, a piece of p2, "Hello from the relay.", of 1.5 s, read 0.48 s after its end, and `/frame` alone gave p2's picture. The four `agent` pairings ended as expected (17.2 s).

The whole matrix, on `xkytttzr` with this change, ended 238 of 238 as expected (499.9 s), this pairing among them.

## What was not run

- Two participants that speak at once: the transcriber reads each broadcast apart, and the reader's last speaker is whose piece came last; which one that is was not looked at.
- A voice other than this one, a language other than English, and a larger model.
- The page's sound is the voice where it is given and silence while it does not speak, in place of the demo's tone; how a voice activity detector of the page would decide was not tried.
