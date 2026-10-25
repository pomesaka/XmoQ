# How many voices one transcriber reads at once

- Date: 2026-10-25
- Commit SHA: the commit that adds this record (the script, `spikes/transcriber-load/run.ts`, on `main` at `62d82350`)
- Roadmap item: Phase 6, meetings of 49 at most; Phase 5, "what the transcription takes of the machine".

## Objective

A meeting has 49 participants at most. The transcriber has one reader, a thread with one model of whisper.cpp, for every track it listens to (`crates/transcriber/README.md`). How many people can speak at once before the words come later and later, and where does it stop keeping up? Measured on **one Mac, over loopback**.

## Expected

Written before the runs, from the earlier record (`2026-10-25-the-transcriber-reads-a-voice.md`): `large-v3-turbo-q5_0` with Metal read 5.5 s of speech in about 2 s with `auto` and 1 s with a language code. So one reader would keep up with about 3 continuous voices with `auto`, and about 6 with a code. Beyond that, the time a piece waits would grow for as long as the run lasts, and pieces would be dropped once 16 wait (`--waiting`).

## Environment

macOS 14.4 on an Apple M3 Pro (18 GiB), **one machine, loopback**. The own relay (debug build), `xmoq-transcriber` (release build) with `large-v3-turbo-q5_0` on Metal and Silero, Chromium 156 of playwright-core, headless, a page for each speaker. The runs were made with the machine's lock of the full interop run held, from 11:57 and from 12:05. Other sessions still ran their builds: the load average was 3 to 11.

`bun spikes/transcriber-load/run.ts --speakers N --seconds 90 [--language en] --port 36100`. Each page publishes the same voice (`publishVoice` of the interop page): "Hello everyone. Today we will talk about how the relay delivers video and audio to many viewers.", made with `say` (Samantha, 5.5 s at 16 kHz), then 1.5 s of silence, over and over. All pages begin together, so every speaker speaks at the same time, for 5.5 s out of each 7 s. The transcriber cut each voice as one piece of about 5.7 s.

The delay is `behind_ms` of the transcriber's `SPEECH` lines: its clock when it had read the piece, less the piece's `endUs`. `endUs` is up to 200 ms after the voice ended, so the time from the end of speaking to the line is up to 200 ms more.

## Result

| Speakers | Language | Pieces cut / read / dropped | Delay p50 / p95 / max, ms | Median of the first quarter → the last, ms | Waited for the reader, p50 / max, ms | whisper.cpp, p50, ms |
|---|---|---|---|---|---|---|
| 1 | auto | 12 / 12 / 0 | 2,831 / 3,124 / 3,124 | 2,917 → 2,983 | 0 / 2 | 2,226 |
| 2 | auto | 24 / 24 / 0 | 4,196 / 4,642 / 4,988 | 4,196 → 4,490 | 1,901 / 2,282 | 1,910 |
| 4 | auto | 49 / 44 / 0 | 8,625 / 13,110 / 14,719 | 5,853 → 11,195 | 6,137 / 12,001 | 1,883 |
| 8 | auto | 101 / 42 / 42 | 27,154 / 32,654 / 33,339 | 12,190 → 30,728 | 24,516 / 30,800 | 1,918 |
| 16 | auto | 208 / 43 / 148 | 28,125 / 32,535 / 33,072 | 12,336 → 30,721 | 25,627 / 30,647 | 1,908 |
| 4 | en | 48 / 48 / 0 | 3,350 / 4,681 / 5,336 | 3,350 → 3,669 | 1,921 / 3,388 | 970 |
| 8 | en | 98 / 86 / 0 | 9,227 / 14,018 / 15,464 | 5,733 → 11,862 | 7,621 / 13,763 | 960 |

Every piece read said the sentence right.

- **It keeps up with 2 voices at once with `auto`, and with 4 with `en`.** There the delay stayed where it began, about 3 s for one voice and 4 s for two, because the pieces that come together wait for each other.
- **With 4 voices and `auto`, and with 8 and `en`, it falls behind.** The delay of the last quarter was twice that of the first, and grew to the end of the run.
- **With 8 and 16 voices and `auto`, pieces were dropped.** 16 pieces waited for the reader (`--waiting`), and each one more was dropped. The reader read 42 or 43 pieces in 90 s either way, one each 2.1 s, and what was read came half a minute late.
- whisper.cpp took the same time for a piece whatever the number of speakers: about 1.9 s with `auto`, about 1 s with `en`. One reader reads one piece at a time. What limits it is the reader's time, at about 2.1 s of reading for 7 s of a speaker's sound with `auto`, so at most 3.3 such speakers, and 7 with `en`. What was measured agrees: 2 kept up, 4 did not.

As expected, but for the margin: with 4 voices and `auto` it was expected to be on the edge, and it fell behind.

## What it means for meetings

A meeting where 49 speak at once is not a case: people take turns. What a meeting with one transcriber of this model can have is about 2 people speaking at the same moment with `auto`, and about 4 with a language code, before the words come later and later. More at once, for longer than the waiting pieces allow, has pieces dropped. To read more at once, the choices are a smaller model (`small-q5_1` read the same voice in a third of the time, `crates/speech/README.md`), a language code, more than one reader, or more than one transcriber. None of them was measured here, and none is decided.

## What was not run

- People who take turns, or who speak over each other in part: here every speaker spoke at the same moment, the hardest case.
- `small-q5_1`, `ja`, more than one reader, or more than one transcriber.
- Linux, a machine without a graphics processor, and a machine with nothing else running. Other sessions' builds ran on this one.
- How much of the processor and the graphics processor the transcriber took.
