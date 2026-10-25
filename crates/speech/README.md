# xmoq-speech

The words of audio tracks, for the transcriber ([`xmoq-transcriber`](../transcriber/README.md)). It decodes the packets of Opus of a track to one channel at 16 kHz, cuts the sound where there is a voice, and reads each piece with a reader: whisper.cpp by default.

| Module | What it does |
|---|---|
| `decode` | Opus to one channel at 16 kHz, with the same decoder as the terminal viewer (`opus-rs`). The decoder gives 16 kHz itself (RFC 6716 §2), so no resampler is needed |
| `frames` | The sound in frames of 20 ms, each with the Timestamp of its first sample. A Timestamp more than 100 ms from where the sound before ended is a gap, and ends the piece under way |
| `judge` | Whether a frame has a voice. **By default by a model of voice activity** (Silero, through whisper.cpp): the frames are judged half a second at a time, together with the half second before them, and a frame has a voice where the window of 32 ms its middle is in has a probability of 0.5 or more. **Or by loudness** (`--voice loudness` of the transcriber, and the tests): -40 dBFS or louder. Why the model, below |
| `cut` | Where a piece begins and ends: with a frame that has a voice, the end after 600 ms without one, and no piece longer than 15 s. Pieces with less than 250 ms of voice are left out. 200 ms before the voice are kept |
| `engine` | Which reader reads the pieces (`Engine`, the transcriber's `--engine`), and opening it (`open`): the one place that names the readers |
| `whisper`, `vad` | whisper.cpp through `whisper-rs`, for reading and for telling a voice: the two files that use it. `whisper` is private to the crate: a reader is had through `engine` |
| `model` | Where a model is looked for |

**All values are provisional**: they were chosen on voices of `say`, not on a meeting. A voice that goes on past 15 s is cut there: in paragraphs of 27 and 42 s no word was lost at a cut, and the word a cut fell in was read on both sides of it or read wrong ([record](../../docs/experiments/2026-10-25-a-voice-longer-than-a-piece.md)).

`Listener` (one for each track) decodes, judges and cuts. Each track has a model of voice activity of its own, which holds what it heard of that track. `Words` reads a piece; `engine::open` gives the readers of the engine it is told, and the transcriber knows them by `Words` alone. The transcriber has its readers for every track. The model is loaded once, and whisper.cpp reads one piece at a time.

```sh
mise run speech-model                    # the default model, into ~/.cache/xmoq/whisper/
mise run speech-model -- tiny            # or large-v3-turbo-q5_0; --dir <dir> elsewhere
mise run speech-model -- silero-v5.1.2   # the model of voice activity
afconvert -f WAVE -d LEI16@48000 -c 1 voice.aiff voice.wav     # macOS, from `say -o voice.aiff ...`
cargo run --release --example read-wav -p xmoq-speech -- ~/.cache/xmoq/whisper/ggml-tiny.bin en voice.wav \
  ~/.cache/xmoq/whisper/ggml-silero-v5.1.2.bin    # without it, a voice is told by loudness
```

## The readers

WHY a choice of readers (asked for on 2026-10-25 by the project owner): the transcription is an experiment, and what reads the words may become another program than whisper.cpp, an agent among them. A reader gets a piece, 16 kHz and one channel, and gives its words; how the sound is cut, and the track and the numbering of the words, stay outside it. A reader is added as a value of `Engine` in `src/engine.rs` and a file of its own.

| Reader | File | What it reads with |
|---|---|---|
| `whisper` | `src/whisper.rs` | whisper.cpp and a model of its own (below) |
| `command` | `src/command.rs` | A program run for each piece: the piece is a WAV file, its path the program's last argument, and its standard output the words. It has a time and a size of output it is stopped at, and fails on an exit that is not 0 (the transcriber's flags). WHY: whatever reads the words (a script around another model, an agent) is put in place without a change here. WHY NOT a service over the network: where the sound of the people present may be sent is undecided (Q35) |

**Every reader needs whisper.cpp to be built**: the model of voice activity that cuts the sound (Silero, `vad`) runs on whisper.cpp too. WHY it is not moved to another runtime now (decided on 2026-10-25): the cutting is outside the reader and works as it is; moving it would be a change of the cutting, measured again, for a build that only a transcriber without whisper.cpp would gain.

## whisper.cpp, and what it needs to build

Read in the crates' manifests and build scripts on 2026-10-25.

| Crate | Used for | License | Needs to build |
|---|---|---|---|
| `whisper-rs` 0.16.0 (`whisper-rs-sys` 0.15.0) | whisper.cpp 1.8.3, built from the source it carries | Unlicense; whisper.cpp and ggml are MIT | A C and a C++ compiler, and **cmake**, which `mise.toml` pins (4.4.4) and CI installs through mise. **libclang**, which `bindgen` loads and without which the build stops ("Unable to find libclang"). On macOS it is in the Command Line Tools; on Linux `scripts/ci/lib.sh` installs `libclang-dev` in CI and asks for it elsewhere. The crate also carries bindings made beforehand, which `WHISPER_DONT_GENERATE_BINDINGS` has it use without libclang; they built on macOS too (tried once). WHY NOT those: they were generated on one system (their first lines name glibc's types), and the crate's default makes them for the system it builds on. On macOS with the feature `metal`, for which the build embeds the Metal library |
| `opus-rs` 0.1.39 | Decoding Opus | BSD-3-Clause | Nothing but Rust (why this decoder: `crates/watch/src/bin/xmoq-watch/sound.rs`) |

WHY `whisper-rs`: it has a safe API (the workspace forbids `unsafe`), and its releases go on (0.16.0 on 2026-03-12, its repository on codeberg.org had commits in March 2026). WHY NOT the program `whisper-cli` of whisper.cpp, run for each piece: the model would be loaded again each time, seconds for the large ones. WHY NOT a whisper written in Rust (`whisper-candle-core` and others): the project owner chose whisper.cpp on 2026-10-25.

whisper.cpp built in 16 s for a debug build and 22 s for a release one on the Mac it was written on (an M3 Pro). That time is added to `mise run check:rust` and to CI the first time; in CI it was not measured.

**Metal on macOS, the processor elsewhere** (`Cargo.toml`). The large model read 8.6 s of speech in 1.0 to 1.1 s with Metal, and in 4.0 s with the processor alone. On Linux whisper.cpp reads on the processor; it was built there by CI and not measured. The first time a model is loaded with Metal, the graphics processor's code is compiled: 9 to 10 s (`MODEL_LOADED ... ms=`). After that, loading takes 0.1 to 0.4 s, except in a run of the interop where it took 9.8 s again.

## Why a voice is told by a model

By loudness alone, a sound that is not a voice is cut into pieces as a voice is, and whisper.cpp makes up words for it. The demo's publisher sends a tone by default (440 Hz with a beep of 1 kHz every 2 s). By loudness it was one voice that did not end, cut every 15 s, and whisper.cpp read it as:

| Model | `ja` | `auto` |
|---|---|---|
| `large-v3-turbo-q5_0` | 「ご視聴ありがとうございました」, 「ん」 | `*music*`, `*BEEP*` |
| `small-q5_1` | 「(エンディング)」 | `(beeping)` |

The `no_speech_probability` that whisper.cpp gives each segment was 0.000 for the tone and for the voices alike. So the made-up words cannot be told apart once they are read; they have to be kept from being read. Silero, the model of voice activity that whisper.cpp carries, gave the tone 0.00 to 0.06 a second and the voices 0.66 to 1.00. It took 0.12 ms for 32 ms of sound, on the processor. With it the tone gave no piece, and the voices were read as before ([record](../../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)).

What it costs: a piece cannot end before its frames are judged, so its words come up to half a second later. Its model is fetched as the others (`silero-v5.1.2`, 885 KB, Hugging Face's `ggml-org/whisper-vad`). WHY NOT the VAD that whisper.cpp runs inside `whisper_full` (`vad` of its parameters): it runs on what it is given to read, and the cutting into pieces, which decides when words come, is before that. WHY loudness is kept: it needs no model, and the tests of the cutting use it.

## The models

The models are not in the repository. `crates/speech/scripts/fetch-model.sh` (`mise run speech-model`) fetches them from Hugging Face's `ggerganov/whisper.cpp`, the repository that whisper.cpp's own download script uses. It checks each against the sha256 written in the script, and keeps it in `${XDG_CACHE_HOME:-~/.cache}/xmoq/whisper/` unless `--dir` names another place. The transcriber looks there unless `--model` names a file.

**The default is `small-q5_1`** (181 MiB), decided on 2026-10-25 by the project owner, for meetings and broadcasts alike. WHY: one transcriber on a Mac kept up with 8 people speaking at the same moment with `--language auto`, and 16 with `en` ([record](../../docs/experiments/2026-10-25-how-many-voices-one-transcriber-reads.md)). It read both voices below right, without punctuation, in a third of the large model's time. WHY NOT **`large-v3-turbo-q5_0`** (547 MiB), the default before: it was the one model of the three that read with punctuation, but one transcriber kept up with 2 voices at once with `auto` and 4 with `en`, and a second reader (`--readers 2`) did not add to that on Metal. It is still there with `--model`. That `small-q5_1` reads Japanese words right was seen on one voice alone. **`tiny`** (74 MiB) is what the interop runs read with: it read the English voice right and the Japanese one wrong. **`silero-v5.1.2`** (885 KB) is the model of voice activity (above); every transcriber loads it, one for each track.

Read on an M3 Pro, release build, `--language` given. Each model ran 3 times on each voice made with `say` (Kyoko, Japanese, 8.6 s; Samantha, English, 5.5 s), while the machine also ran other sessions' builds ([record](../../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)):

| Model | Size | Japanese, ms | English, ms | Japanese, as read | English |
|---|---|---|---|---|---|
| `tiny` | 74 MiB | 98 to 142 | 78 to 99 | 「リレー」 read as 「理例」, no punctuation | right |
| `small-q5_1` | 181 MiB | 340 to 419 | 330 to 336 | right, no punctuation | right |
| `large-v3-turbo-q5_0` | 547 MiB | 1,040 to 1,125 | 1,009 to 1,036 | right, with punctuation | right |

With `--language auto` (the transcriber's default), whisper.cpp first finds the language of each piece. That took the large model 2.0 s on each voice, about twice as long, and both voices were read right.

## Checks

The tests (`cargo test -p xmoq-speech`) decode packets that `opus-rs` encoded from a tone, and cut packets of loud and quiet frames by the table in `src/cut.rs`. They read no model. The interop run `own relay: transcriber: ...` reads a voice with `tiny` through the transcriber (`interop/transcriber.ts`).
