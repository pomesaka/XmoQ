# xmoq-transcriber

The transcriber (decided on 2026-10-25). It is a client of the relay beside it, as the gatherer of the chat is. It listens to the audio tracks of the broadcasts announced under a prefix, reads their words with whisper.cpp ([`xmoq-speech`](../speech/README.md)), and publishes them as one track for each prefix. An agent reads the transcript by subscribing to that track, and so can any viewer.

```sh
mise run speech-model                       # the default model, once
mise run speech-model -- silero-v5.1.2      # and the model of voice activity
mise run transcriber -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --namespace xmoq/demo
```

The demo (`mise run demo`) starts it beside the relay, and the watching page shows its transcript as captions (`demo/serve.ts`, `demo/captions.ts`).

It runs until Ctrl-C or SIGTERM, and connects again after a session ends (`backoff` of [`xmoq-track-serving`](../track-serving/README.md)). The model is loaded once, at the start, and stays over the sessions. With a flag it does not know it prints its flags (`USAGE` in `src/args.rs`). **Building it needs a C++ compiler and cmake**, for whisper.cpp ([`xmoq-speech`](../speech/README.md)).

## What it listens to, and what it publishes

| | |
|---|---|
| Listens to | Every namespace announced under a prefix of `--namespace` (up to 8, none under another), found with one `SUBSCRIBE_NAMESPACE` for each prefix. Of each, the first Opus track with LOC packaging that its catalog names (MSF §5), read with a SUBSCRIBE and a Joining FETCH, as the terminal viewer reads it. A catalog that names another audio track later moves the transcriber to that track |
| Publishes | The track `speech` of `xmoq/transcript/<prefix>` for each prefix, with PUBLISH, from the start of each session. One piece is one Group, and its payload is in [`xmoq-transcript-tracks`](../transcript-tracks/README.md) |
| Answers | A FETCH of a transcript, from the latest `--keep` pieces it holds in memory. A range that begins before them gets them and an End of Unknown Range indicator, as the gatherer answers (`fetch` of `xmoq-track-serving`) |

WHY these, and WHY NOT the other choices (every namespace of the relay; a room that says where the broadcast is; a track `transcript` in the broadcast's namespace; PUBLISH_NAMESPACE): at the top of `src/bin/xmoq-transcriber/session.rs` and of `crates/transcript-tracks/src/names.rs`.

**The pieces are numbered from the time the transcriber started**, in milliseconds since the Unix epoch, and up by one from there. A transcriber that starts again therefore goes on above the Group IDs it gave before, which a relay and its subscribers may still have (why, at `Transcript::new` in `src/transcript.rs`). **Nothing is kept on disk**: the pieces are lost when it stops. Whether and for how long a transcript is to be kept is [undecided](../../docs/open-questions.md).

**One reader for every track** (`src/bin/xmoq-transcriber/reader.rs`): a thread with the model. It reads the pieces in the order they were cut. A piece that finds `--waiting` pieces waiting for it is dropped and counted (`PIECE_DROPPED`). How long a piece waited, how long whisper.cpp took, and how long after the end of the piece its words were there are in each `SPEECH` line (`waited_ms`, `took_ms`, `behind_ms`). `behind_ms` is `at` minus `endUs` (both in milliseconds), and holds where the broadcast's Timestamps are wall-clock time, as the demo's pages make them.

An audio subscription that ends or is refused while the broadcast stays announced is asked for again, by the rule every client asks by ([`xmoq-again`](../again/README.md)). There is no count of times, as the gatherer has for a participant (why, at `audio_over` in `session.rs`).

## Flags

| Flag | Default | What it sets | The flag's range |
|---|---|---|---|
| `--namespace` | (needed) | A prefix to listen under, its fields between slashes | 1 to 8; none the same as another, a prefix of it, or under it; 30 fields at the most |
| `--model` | `~/.cache/xmoq/whisper/ggml-large-v3-turbo-q5_0.bin` | The file of the model | |
| `--language` | `auto` | A language code of whisper's (`ja`, `en`), or `auto`, with which whisper.cpp finds the language of each piece first (twice as slow, [`xmoq-speech`](../speech/README.md)). With a code, every voice is read in that language: an English voice read with `ja` came out in Japanese | |
| `--voice` | `vad` | How a voice is told: by the model of voice activity, or by `loudness` (why the model: [`xmoq-speech`](../speech/README.md)) | `vad`, `loudness` |
| `--vad-model` | `~/.cache/xmoq/whisper/ggml-silero-v5.1.2.bin` | The file of the model of voice activity. Loaded once at the start, so that a missing one stops the transcriber there, and once more for each broadcast | |
| `--keep` | 1,000 | The latest pieces of each transcript kept for FETCH | 1 to 100,000 |
| `--waiting` | 16 | Pieces that wait for the reader | 1 to 256 |
| `--tracks` | 16 | Broadcasts listened to at once; one more is passed over (`BROADCAST_IGNORED`) | 1 to 100 |
| `--auth-key-hex` | none | The relay's key, of which the transcriber makes its tokens: `sub` in `<prefix>/*` for its SETUP, its SUBSCRIBE_NAMESPACE, SUBSCRIBEs and FETCHes, and `pub` in `xmoq/transcript/<prefix>` for its PUBLISH. For the demo alone, as the gatherer's (Q51, provisional) | |

## What it holds

What the relay and the publishers send has a bound that no flag removes. **All are provisional.**

| What | Bound | Where |
|---|---|---|
| A catalog | 1 MiB, as the terminal viewer's | `CATALOG_LIMIT`, `session.rs` |
| Objects that come before their `SUBSCRIBE_OK` | 64; the oldest is dropped | `session.rs` |
| Pieces that wait for the `PUBLISH_OK` of their transcript | 64; the oldest is not sent live | `session.rs` |
| Subgroup streams of pieces being written | 64; the rest are not sent live (`SPEECH_NOT_SENT`) | `session.rs` |
| FETCH requests answered at once | 32; one more is refused with `EXCESSIVE_LOAD` | `session.rs` |
| Bytes of payload the FETCH answers being written hold together | 16 MB; a FETCH whose answer would go past it is refused with `EXCESSIVE_LOAD` | `MAX_BYTES_HELD`, `session.rs` |
| A piece | 15 s of samples, 960 KB | `Cut`, `xmoq-speech` |

A piece that is not sent live is kept, and a FETCH gets it.

## What is where

The library (`src/*.rs`) decides and does no input or output: what the flags mean (`args`), and the numbering and keeping of a transcript (`transcript`). The binary (`src/bin/xmoq-transcriber/`): `session.rs` (the requests and the answers on one session), `reader.rs` (the thread of whisper.cpp), `main.rs` (the model, connecting again, the signals). What it prints, one line for each thing it does (`MODEL_LOADED`, `VAD_LOADED`, `WATCHING`, `BROADCAST`, `AUDIO_SUBSCRIBED`, `TRANSCRIPT_PUBLISHED`, `PIECE_CUT`, `SPEECH`, `PIECE_DROPPED`, `FETCH_ANSWERED`, ...), is what `interop/transcriber.ts` waits for.

## Not done

- Run through the own relay alone, not through `moqtail`'s.
- On Linux it was neither built nor run here. CI is to build it and to run the interop pairing with the smallest model. How fast the larger models read on the processor of a Linux machine was not measured.
- A broadcast whose catalog cannot be read, or names no Opus track, is not asked for its catalog again until its namespace is announced again.

## Checks

`interop/transcriber.ts` (`mise run interop`, the pairing `own relay: transcriber: ...`): a page publishes a voice and silence, over and over (`fixtures/speech/`). The transcriber reads it with the smallest model, and a live reader of the transcript gets two pieces with the words. A later reader gets them by a FETCH ([record](../../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)). With `--auth yes` (the pairing `own relay: transcriber: with tokens, ...`) the relay, the transcriber and the room's tokens have one key: the page announces with `pub` for its namespace, and a reader with `sub` for the transcript gets the words.
