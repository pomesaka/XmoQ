# xmoq-speech

The words of audio tracks, for the transcriber ([`xmoq-transcriber`](../transcriber/README.md)). It decodes the packets of Opus of a track to one channel at 16 kHz, cuts the sound where there is a voice, and reads each piece with a reader: whisper.cpp by default, or a program run for each piece (`--engine command` of the transcriber). **Every value is provisional**: they were chosen on voices of `say`, not on a meeting.

```sh
mise run speech-model                    # the default model, into ~/.cache/xmoq/whisper/
mise run speech-model -- tiny            # or large-v3-turbo-q5_0; --dir <dir> elsewhere
mise run speech-model -- silero-v5.1.2   # the model of voice activity
afconvert -f WAVE -d LEI16@48000 -c 1 voice.aiff voice.wav     # macOS, from `say -o voice.aiff ...`
cargo run --release --example read-wav -p xmoq-speech -- ~/.cache/xmoq/whisper/ggml-tiny.bin en voice.wav \
  ~/.cache/xmoq/whisper/ggml-silero-v5.1.2.bin    # without it, a voice is told by loudness
```

## The models

The models are not in the repository. `crates/speech/scripts/fetch-model.sh` (`mise run speech-model`) fetches them from Hugging Face's `ggerganov/whisper.cpp`, the repository that whisper.cpp's own download script uses, and the model of voice activity (`silero-v5.1.2`) from `ggml-org/whisper-vad`. It checks each against the sha256 written in the script, and keeps it in `${XDG_CACHE_HOME:-~/.cache}/xmoq/whisper/` unless `--dir` names another place. The transcriber looks there unless `--model` names a file. The default model, and why, is `DEFAULT_MODEL` in `src/model.rs`; `tiny` is what the interop runs read with. Every transcriber loads the model of voice activity, one for each track.

## whisper.cpp, and what it needs to build

**Every reader needs whisper.cpp to be built**: the model of voice activity that cuts the sound runs on it (why, at the top of `src/vad.rs`).

| Crate | Used for | License | Needs to build |
|---|---|---|---|
| `whisper-rs` 0.16.0 (`whisper-rs-sys` 0.15.0; why, at the top of `src/whisper.rs`) | whisper.cpp 1.8.3, built from the source it carries | Unlicense; whisper.cpp and ggml are MIT | A C and a C++ compiler, and **cmake**, which `mise.toml` pins (4.4.4) and CI installs through mise. **libclang**, which `bindgen` loads and without which the build stops ("Unable to find libclang"). On macOS it is in the Command Line Tools; on Linux `scripts/ci/lib.sh` installs `libclang-dev` in CI and asks for it elsewhere. The crate also carries bindings made beforehand, which `WHISPER_DONT_GENERATE_BINDINGS` has it use without libclang; they built on macOS too (tried once). WHY NOT those: they were generated on one system (their first lines name glibc's types), and the crate's default makes them for the system it builds on. On macOS with the feature `metal`, for which the build embeds the Metal library |
| `opus-rs` 0.1.39 | Decoding Opus | BSD-3-Clause | Nothing but Rust (why this decoder: `crates/watch/src/bin/xmoq-watch/sound.rs`) |

**Metal on macOS, the processor elsewhere** (`Cargo.toml`). On Linux whisper.cpp reads on the processor; it was built there by CI and not measured. How fast each model reads is in the records ([one voice](../../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md), [voices at once](../../docs/experiments/2026-10-25-how-many-voices-one-transcriber-reads.md)).
