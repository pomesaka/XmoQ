# What whisper writes for noise

- Date: 2026-10-27
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, an agent in a meeting: an agent was told `[BLANK_AUDIO]` as something said, in a meeting of the demo with the person who called it.

## Objective

What does whisper.cpp write for a piece in which nobody spoke, which the transcriber then publishes as words? Is there a form the transcriber can leave out without leaving out words that were said?

**One machine, files only.**

## Expected

Said before the run (in the session, not in a file): marks in brackets, as `[BLANK_AUDIO]`, and now and then words made up.

## Environment

macOS 14.4, arm64. `xmoq-speech` with whisper.cpp 1.8.3 (`whisper-rs` 0.16.0), `cargo run --release --example read-wav -p xmoq-speech`, with the judge of loudness (no model of voice activity, so that each file is cut as one piece). The models `ggml-tiny.bin` and `ggml-large-v3-turbo-q5_0.bin`, each with the language `ja` and `auto`. Four files of 6 s at 48 kHz, made with Python, the sound from 1 s to 5 s: white noise at about −20 dBFS and at about −35 dBFS, a tone of 120 Hz at −20 dBFS, and clicks five times a second over a faint noise.

## Observed results

| File | tiny, `ja` | tiny, `auto` | turbo, `ja` | turbo, `auto` |
|---|---|---|---|---|
| noise, −20 dBFS | ご視聴ありがとうございました | `[BIRDS CHIRPING]` | ご視聴ありがとうございました | you |
| noise, −35 dBFS | ご視聴ありがとうございました | (nothing) | ご視聴ありがとうございました | you |
| tone | (音楽) | (nothing) | ご視聴ありがとうございました | you |
| clicks | (音楽) | `[MUSIC PLAYING]` | お疲れ様でした | Það er hann. |

`[BLANK_AUDIO]` did not come here. The source of whisper.cpp 1.8.3 has none of these marks: they are what the model writes.

So the transcriber leaves out a piece whose words are marks in brackets alone (`[...]`, `(...)`, `（...）`, `【...】`), as it left out one with no words (`only_marks` of `crates/transcriber/src/marks.rs`). The words made up from noise are left in: each is also what a person says.

## Limitations and blockers

- The meeting in which `[BLANK_AUDIO]` came was not run here: which model and language the demo's transcriber had then was not looked at.
- Real noise (a breath, a keyboard, a room) was not tried, nor the model of voice activity, which cuts fewer such pieces.
- A transcriber with the change was not run against a voice and noise together; the interop pairings of the transcriber (`--only transcriber`, a voice alone) passed with it.

## Open questions raised

None.
