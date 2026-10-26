# xmoq-transcriber

The transcriber (decided on 2026-10-25). It is a client of the relay beside it, as the gatherer of the chat is. It listens to the audio tracks of the broadcasts announced under a prefix, reads their words with a reader, whisper.cpp by default (`--engine`, [`xmoq-speech`](../speech/README.md)), and publishes them as one track for each prefix. An agent reads the transcript by subscribing to that track, and so can any viewer.

```sh
mise run speech-model                       # the default model, once
mise run speech-model -- silero-v5.1.2      # and the model of voice activity
mise run transcriber -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --namespace xmoq/demo
```

The demo (`mise run demo`) starts it beside the relay, and the watching page shows its transcript as captions (`demo/serve.ts`, `demo/captions.ts`).

It runs until Ctrl-C or SIGTERM, and connects again after a session ends, as the gatherer does, also after a GOAWAY (`Pause::SERVING` of [`xmoq-again`](../again/README.md)). The model is loaded once, at the start, and stays over the sessions. With a flag it does not know it prints its flags (`USAGE` in `src/args.rs`). **Building it needs a C++ compiler and cmake**, for whisper.cpp ([`xmoq-speech`](../speech/README.md)).

## What it listens to, and what it publishes

| | |
|---|---|
| Listens to | Every namespace announced under a prefix of `--namespace` (up to 8, none under another), found with one `SUBSCRIBE_NAMESPACE` for each prefix. Of each, the first Opus track with LOC packaging that its catalog names (MSF §5), read with a SUBSCRIBE and a Joining FETCH, as the terminal viewer reads it. A catalog that names another audio track later moves the transcriber to that track |
| Publishes | The track `speech` of `xmoq/transcript/<prefix>` for each prefix, with PUBLISH, from the start of each session. One piece is one Group, and its payload is in [`xmoq-transcript-tracks`](../transcript-tracks/README.md) |
| Answers | A FETCH of a transcript, from the latest `--keep` pieces it holds in memory. The pieces of the range that were let go of get one End of Unknown Range indicator, which in Descending order names no number before the first piece, as the Rust client answers (`plan_kept` of `xmoq-track-serving`; `Transcript::fetch`) |

WHY these, and WHY NOT the other choices (every namespace of the relay; a room that says where the broadcast is; a track `transcript` in the broadcast's namespace; PUBLISH_NAMESPACE): at the top of `src/bin/xmoq-transcriber/session.rs` and of `crates/transcript-tracks/src/names.rs`.

**The pieces are numbered from the time the transcriber started**, in milliseconds since the Unix epoch, and up by one from there. A transcriber that starts again therefore goes on above the Group IDs it gave before, which a relay and its subscribers may still have (why, at `Transcript::new` in `src/transcript.rs`). **Nothing is kept on disk**: the pieces are lost when it stops. Whether and for how long a transcript is to be kept is [undecided](../../docs/open-questions.md).

**Readers for every track** (`src/bin/xmoq-transcriber/reader.rs`): threads, one by default (`--readers`), each with a reader of `--engine`, which the binary knows by the trait `Words` of `xmoq-speech` alone. Readers of whisper.cpp share the one model. Each takes the next piece that was cut. A piece that finds `--waiting` pieces waiting for it is dropped and counted (`PIECE_DROPPED`). How long a piece waited, how long the reader took, and how long after the end of the piece its words were there are in each `SPEECH` line (`waited_ms`, `took_ms`, `behind_ms`). `behind_ms` is `at` minus `endUs` (both in milliseconds), and holds where the broadcast's Timestamps are wall-clock time, as the demo's pages make them.

**How many voices at once**: with the default model (`small-q5_1`) on a Mac (one machine, loopback), one transcriber kept up with 8 people speaking at the same moment with `--language auto`, and 16 with `--language en`. With 16 and `auto` the words came later and later, and pieces were dropped. With `--model large-v3-turbo-q5_0` it kept up with 2 with `auto` and 4 with `en`; `--readers 2` read no more than one reader on Metal ([record](../../docs/experiments/2026-10-25-how-many-voices-one-transcriber-reads.md)). Why the default is `small-q5_1`: [`xmoq-speech`](../speech/README.md).

A subscription to a broadcast's catalog or sound that ends or is refused while the broadcast stays announced is asked for again (`ASKING_AGAIN ... track=catalog` or `track=audio`), by the rule every client asks by ([`xmoq-again`](../again/README.md)), so that a broadcast whose catalog was refused before its publisher had it is heard once it has (why, at `catalog_over` in `session.rs`). There is no count of times, as the gatherer has for a participant (why, at `audio_over`). A catalog that is malformed is not asked for again, and the broadcast is let go of until its namespace is announced again. A broadcast for which the session takes no more SUBSCRIBEs (after a GOAWAY) is let go of too (`BROADCAST_NOT_ASKED`), so that it holds no place of `--tracks`.

## Flags

The flags, what each is without it and their ranges are `USAGE` in `src/args.rs`, which the transcriber prints when a flag is wrong. What `USAGE` does not say:

- **`--engine command`**: how the sound is cut, and the track and the numbering of the words, are the same with every reader (`Engine` of [`xmoq-speech`](../speech/README.md)). The program and its arguments are split at spaces (a program whose arguments have spaces is put in a script). Each piece is written as a WAV file (16 kHz, 16 bits, one channel) to a directory of the transcriber's own under the system's temporary directory, which goes when the transcriber ends (on SIGTERM and Ctrl-C too; not after SIGKILL), as does a program that runs then; the file is removed after the program. The words are its standard output, trimmed. A program that runs over `--engine-timeout-ms` is killed with the processes it started (it runs in a process group of its own), and the piece is not read (`PIECE_NOT_READ`); an exit that is not 0, output over `--engine-most-output` and output that is not UTF-8 are the same. Both values are **provisional**. `--model` is refused with it.
- **The models**: `large-v3-turbo-q5_0` reads with punctuation, and fewer voices at once; a language code reads every voice in that language; why voice activity by a model ([`xmoq-speech`](../speech/README.md)). The model of voice activity is loaded once at the start, so that a missing one stops the transcriber there, and once more for each broadcast.
- **`--readers`** more than one: a later piece can be done before an earlier one, of the same voice as well, and is numbered first.
- **`--auth-key-hex`**: the transcriber makes `sub` in `<prefix>/*` for its SETUP, its SUBSCRIBE_NAMESPACE, SUBSCRIBEs and FETCHes, and `pub` in `xmoq/transcript/<prefix>` for its PUBLISH. For the demo alone, as the gatherer's (provisional).
- **`--tracks`**: a broadcast over it is logged as `BROADCAST_IGNORED`.

## What it holds

What the relay and the publishers send has a bound that no flag removes. **All are provisional.**

| What | Bound | Where |
|---|---|---|
| A catalog | 1 MiB, as the terminal viewer's | `CATALOG_LIMIT`, `session.rs` |
| Objects that come before their `SUBSCRIBE_OK` | 64; the oldest is dropped | `session.rs` |
| Pieces that wait for the `PUBLISH_OK` of their transcript | 64; the oldest is not sent live | `session.rs` |
| Subgroup streams of pieces being written | 64; the rest are not sent live (`SPEECH_NOT_SENT`) | `session.rs` |
| FETCH requests answered at once | 32; one more is refused with `EXCESSIVE_LOAD` | `MAX_ANSWERED_AT_ONCE` of [`xmoq-track-serving`](../track-serving/README.md) |
| Bytes of payload the FETCH answers being written hold together | 16 MB; a FETCH whose answer would go past it is refused with `EXCESSIVE_LOAD` | `MAX_BYTES_HELD`, `session.rs` |
| A piece | 15 s of samples, 960 KB | `Cut`, `xmoq-speech` |

A piece that is not sent live is kept, and a FETCH gets it.

## What is where

The library (`src/*.rs`) decides and does no input or output: what the flags mean (`args`), and the numbering and keeping of a transcript, and what a FETCH of it gets (`transcript`). The binary (`src/bin/xmoq-transcriber/`): `session.rs` (the requests and the answers on one session), `reader.rs` (the threads of the readers), `main.rs` (opening the reader, connecting again, the signals). What it prints, one line for each thing it does (`MODEL_LOADED`, `VAD_LOADED`, `WATCHING`, `BROADCAST`, `AUDIO_SUBSCRIBED`, `TRANSCRIPT_PUBLISHED`, `PIECE_CUT`, `SPEECH`, `PIECE_DROPPED`, `FETCH_ANSWERED`, ...), is what `interop/transcriber.ts` waits for.

## Not done

- Through `moqtail`'s relay no run has a voice reach it: the run there (`--case catalog-refused` of `interop/transcriber.ts`) ends once the page takes the SUBSCRIBE for its catalog again, since that relay answers the Joining FETCH for the catalog with nothing.
- On Linux, only the interop job of CI builds it and runs it, with the model `tiny` (`scripts/ci/interop.sh`). How fast the larger models read on the processor of a Linux machine was not measured.
- A broadcast whose catalog cannot be read, or names no Opus track, is not asked for its catalog again until its namespace is announced again.

## Checks

`interop/transcriber.ts` (`mise run interop`, the pairing `own relay: transcriber: ...`): a page publishes a voice and silence, over and over (`fixtures/speech/`). The transcriber reads it with the smallest model, and a live reader of the transcript gets two pieces with the words. A later reader gets them by a FETCH ([record](../../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)). With `--auth yes` (the pairing `own relay: transcriber: with tokens, ...`) the relay, the transcriber and the room's tokens have one key: the page announces with `pub` for its namespace, and a reader with `sub` for the transcript gets the words.
