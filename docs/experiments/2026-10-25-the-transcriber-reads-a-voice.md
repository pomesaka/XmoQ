# The transcriber reads a voice

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 5, "The broadcast's sound is transcribed with whisper.cpp" and "Measure the time from a word being said to its text being written".

## Objective

The transcriber (`xmoq-transcriber`) is a client of the relay. It listens to the audio tracks of the broadcasts under a prefix, reads them with whisper.cpp (`xmoq-speech`), and publishes the words as the track `speech` of `xmoq/transcript/<prefix>`. Two questions:

1. Which model is the default? The three candidates were weighed by size, how well they read Japanese and English, and how fast.
2. Do the words of a voice that a page publishes reach a reader of the transcript through the own relay? How long after the voice do they come?

## Expected

For 1, nothing was written before: the runs were to give the numbers for the choice.

For 2, written before the first run, at the top of `interop/transcriber.ts`:

- The transcriber is told of the page's namespace by its SUBSCRIBE_NAMESPACE for `xmoq/voice` (`BROADCAST`), finds the audio track in the catalog, and subscribes to it (`AUDIO_SUBSCRIBED`). It has published the transcript (`TRANSCRIPT_PUBLISHED`).
- A live reader of `speech` gets 2 pieces: Object 0 of two Groups, one after the other. Each is `{"from":["xmoq","voice"],...}` whose text has the word "relay" (any case). `endUs` is after `startUs` by 1 to 3 s, and the second piece begins after the first ends.
- `at` is no more than 3 s after `endUs`.
- A later reader FETCHes the whole transcript and gets the pieces kept, the two above among them, with FETCH_OK ending at {largest, 1}.
- No piece is dropped.

## Environment

macOS 14.4 on an Apple M3 Pro (18 GiB); Bun 1.3.14; Chromium 156.0.8078.4 of playwright-core, headless; whisper.cpp 1.8.3 of `whisper-rs` 0.16.0, built with cmake 4.4.4; this tree, on `main` at `3b5864ec` with this change and the one below it (`xmoq-track-serving`). Other sessions' builds and interop runs were running on the same machine (load average 3 to 17).

- For 1: `cargo run --release --example read-wav -p xmoq-speech -- <model> <language> <wav>`. It encodes a WAV file of 48 kHz to packets of Opus of 20 ms with `opus-rs`, as a browser sends them, and passes them through the listener and whisper.cpp. Two voices of `say` were used: Kyoko, 「こんにちは。今日は、新しい配信の仕組みについて話します。リレーを通して、映像と音声を届けます。」, 8.6 s; and Samantha, "Hello everyone. Today we will talk about how the relay delivers video and audio to many viewers.", 5.5 s. Each model ran 3 times on each voice with Metal and `--language` given, once on each voice with `--language auto`, and once on each voice on the processor alone (a build without the feature `metal`).
- For 2: `mise run interop -- --only transcriber --base-port 36000`, with `tiny`.

## Result

### 1. The models

The time whisper.cpp took for the whole voice, which the listener cut as one piece:

| Model | Size | Japanese, ms (3 runs) | English, ms (3 runs) | Japanese, with `auto` | English, with `auto` | On the processor alone, ja / en |
|---|---|---|---|---|---|---|
| `tiny` | 74 MiB | 142, 119, 98 | 99, 84, 78 | | | 201 / 167 |
| `small-q5_1` | 181 MiB | 419, 340, 376 | 330, 336, 333 | 569 | 481 | 1,184 / 949 |
| `large-v3-turbo-q5_0` | 547 MiB | 1,125, 1,040, 1,059 | 1,009, 1,036, 1,025 | 2,040 | 1,966 | 3,994 / 4,459 |

What they read:

- `tiny`: the Japanese voice as 「こんにちは今日は新しい配信の仕組みについて話します 理例を通して映像と音声を届けます」. 「リレー」 became 「理例」, and there was no punctuation. The English voice was right.
- `small-q5_1`: both right; the Japanese without punctuation.
- `large-v3-turbo-q5_0`: both right, with punctuation, also with `auto`.

Loading a model with Metal took 9.3 s the first time, while the graphics processor's code was compiled, and 0.1 to 0.4 s after that.

**Decided on these numbers: `large-v3-turbo-q5_0` is the default.** It is the only one of the three that read both voices right with punctuation. With Metal it took 12 to 19% of the voice's length (with `auto`, 24 to 36%), so one reader keeps up with three to five voices at once. `small-q5_1` is the choice for more voices at once or without a graphics processor. Both are in `crates/speech/README.md`.

### 2. The interop run

**As expected**, the first time it ran (15.8 s):

- The transcriber printed `MODEL_LOADED ... ggml-tiny.bin language=en ms=9788`, `TRANSCRIPT_PUBLISHED prefix=xmoq/voice forward=true largest=none`, `WATCHING prefix=xmoq/voice`, `BROADCAST namespace=xmoq/voice/page` and `AUDIO_SUBSCRIBED namespace=xmoq/voice/page`.
- The live reader got Groups 1792884515830 and 1792884515831. Both said "Hello from the relay.", from `xmoq/voice/page`, and lasted 1,360 ms and 1,480 ms (the voice is 1,184 ms).
- whisper.cpp took 110 ms and 79 ms (`took_ms`), and no piece waited for the reader (`waited_ms=0`). Each piece's words were there 493 ms and 459 ms after its `endUs` (`behind_ms`). `endUs` is up to 200 ms after the voice ends, and a piece is cut 600 ms after the voice, so the words came about 650 to 700 ms after the voice ended.
- The later reader fetched both pieces, and FETCH_OK ended at {1792884515831, 1}.
- No piece was dropped. The page made 227 pieces of sound of 20 ms, which is two voices.

`MODEL_LOADED` took 9.8 s here again, though `tiny` had been loaded with Metal before by another binary. The run waits for that line, so the time is not lost to the pairing; it lengthens the run.

In the full runs (`mise run interop -- --base-port 36000`, with the machine's lock) the pairing took 5.6 s and 5.5 s, with the same two pieces, there 453 to 506 ms after their ends. The last full run, on the final code of this change, ended 219 of 219 as expected (448.1 s; load average 2.4 and 18.0 GB of 19.5 GB of swap in use when it began). Rebased on `main` with `xmoq-agent` (#330), it ended 226 of 226 as expected (451.1 s).

## What was not run

- Linux: neither the build of whisper.cpp nor the run. CI is to do both, with the processor alone.
- A meeting, a real microphone, voices over each other, music or noise. The values of cutting were chosen on voices of `say` alone.
- `--auth-key-hex`, `moqtail`'s relay, a transcriber that starts again, and a catalog that changes its audio track.
- How much of the machine the transcriber takes while it reads.
