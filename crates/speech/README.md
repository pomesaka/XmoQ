# xmoq-speech

The words of audio tracks, for the transcriber ([`xmoq-transcriber`](../transcriber/README.md)). It decodes the packets of Opus of a track to one channel at 16 kHz, cuts the sound where there is a voice, and reads each piece with whisper.cpp.

| Module | What it does |
|---|---|
| `decode` | Opus to one channel at 16 kHz, with the same decoder as the terminal viewer (`opus-rs`). The decoder gives 16 kHz itself (RFC 6716 §2), so no resampler is needed |
| `cut` | Where a piece begins and ends: frames of 20 ms, a voice where a frame is at -40 dBFS or louder, the end after 600 ms without one, and no piece longer than 15 s. Pieces with less than 250 ms of voice are left out. 200 ms before the voice are kept. **All values are provisional**: they were chosen on voices of `say`, not on a meeting |
| `whisper` | whisper.cpp through `whisper-rs`: the one file that uses it |
| `model` | Where a model is looked for |

`Listener` (one for each track) decodes and cuts. `Words` reads a piece, and `Whisper` implements it. The transcriber has one `Whisper` for every track. The model is loaded once, and whisper.cpp reads one piece at a time.

```sh
mise run speech-model                    # the default model, into ~/.cache/xmoq/whisper/
mise run speech-model -- tiny            # or small-q5_1, large-v3-turbo-q5_0; --dir <dir> elsewhere
afconvert -f WAVE -d LEI16@48000 -c 1 voice.aiff voice.wav     # macOS, from `say -o voice.aiff ...`
cargo run --release --example read-wav -p xmoq-speech -- ~/.cache/xmoq/whisper/ggml-tiny.bin en voice.wav
```

## whisper.cpp, and what it needs to build

Read in the crates' manifests and build scripts on 2026-10-25.

| Crate | Used for | License | Needs to build |
|---|---|---|---|
| `whisper-rs` 0.16.0 (`whisper-rs-sys` 0.15.0) | whisper.cpp 1.8.3, built from the source it carries | Unlicense; whisper.cpp and ggml are MIT | A C and a C++ compiler, and **cmake**, which `mise.toml` pins (4.4.4) and CI installs through mise. **libclang**, which `bindgen` loads and without which the build stops ("Unable to find libclang"). On macOS it is in the Command Line Tools; on Linux `scripts/ci/lib.sh` installs `libclang-dev` in CI and asks for it elsewhere. The crate also carries bindings made beforehand, which `WHISPER_DONT_GENERATE_BINDINGS` has it use without libclang; they built on macOS too (tried once). WHY NOT those: they were generated on one system (their first lines name glibc's types), and the crate's default makes them for the system it builds on. On macOS with the feature `metal`, for which the build embeds the Metal library |
| `opus-rs` 0.1.39 | Decoding Opus | BSD-3-Clause | Nothing but Rust (why this decoder: `crates/watch/src/bin/xmoq-watch/sound.rs`) |

WHY `whisper-rs`: it has a safe API (the workspace forbids `unsafe`), and its releases go on (0.16.0 on 2026-03-12, its repository on codeberg.org had commits in March 2026). WHY NOT the program `whisper-cli` of whisper.cpp, run for each piece: the model would be loaded again each time, seconds for the large ones. WHY NOT a whisper written in Rust (`whisper-candle-core` and others): the project owner chose whisper.cpp on 2026-10-25.

whisper.cpp built in 16 s for a debug build and 22 s for a release one on the Mac it was written on (an M3 Pro). That time is added to `mise run check:rust` and to CI the first time; in CI it was not measured.

**Metal on macOS, the processor elsewhere** (`Cargo.toml`). The large model read 8.6 s of speech in 1.0 to 1.1 s with Metal, and in 4.0 s with the processor alone. On Linux whisper.cpp reads on the processor; it was built there by CI and not measured. The first time a model is loaded with Metal, the graphics processor's code is compiled: 9 to 10 s (`MODEL_LOADED ... ms=`). After that, loading takes 0.1 to 0.4 s, except in a run of the interop where it took 9.8 s again.

## The models

The models are not in the repository. `crates/speech/scripts/fetch-model.sh` (`mise run speech-model`) fetches them from Hugging Face's `ggerganov/whisper.cpp`, the repository that whisper.cpp's own download script uses. It checks each against the sha256 written in the script, and keeps it in `${XDG_CACHE_HOME:-~/.cache}/xmoq/whisper/` unless `--dir` names another place. The transcriber looks there unless `--model` names a file.

**The default is `large-v3-turbo-q5_0`** (547 MiB), decided on 2026-10-25 on the numbers below. It was the one model of the three that read Japanese and English right with punctuation, and with Metal it reads a voice in a fifth of its length. One reader can therefore keep up with about five voices at once. **`small-q5_1`** (181 MiB) reads both languages right without punctuation, in a fifteenth of the time. It is the one to take where many voices are listened to at once, or without a graphics processor. **`tiny`** (74 MiB) is what the interop runs read with: it read the English voice right and the Japanese one wrong.

Read on an M3 Pro, release build, `--language` given. Each model ran 3 times on each voice made with `say` (Kyoko, Japanese, 8.6 s; Samantha, English, 5.5 s), while the machine also ran other sessions' builds ([record](../../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)):

| Model | Size | Japanese, ms | English, ms | Japanese, as read | English |
|---|---|---|---|---|---|
| `tiny` | 74 MiB | 98 to 142 | 78 to 99 | 「リレー」 read as 「理例」, no punctuation | right |
| `small-q5_1` | 181 MiB | 340 to 419 | 330 to 336 | right, no punctuation | right |
| `large-v3-turbo-q5_0` | 547 MiB | 1,040 to 1,125 | 1,009 to 1,036 | right, with punctuation | right |

With `--language auto` (the transcriber's default), whisper.cpp first finds the language of each piece. That took the large model 2.0 s on each voice, about twice as long, and both voices were read right.

## Checks

The tests (`cargo test -p xmoq-speech`) decode packets that `opus-rs` encoded from a tone, and cut packets of loud and quiet frames by the table in `src/cut.rs`. They read no model. The interop run `own relay: transcriber: ...` reads a voice with `tiny` through the transcriber (`interop/transcriber.ts`).
