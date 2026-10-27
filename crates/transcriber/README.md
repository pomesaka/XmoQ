# xmoq-transcriber

The transcriber. It is a client of the relay beside it, as the gatherer of the chat is. It listens to the audio tracks of the broadcasts announced under a prefix, reads their words with a reader, whisper.cpp by default (`--engine`, [`xmoq-speech`](../speech/README.md)), and publishes them as one track for each prefix. An agent reads the transcript by subscribing to that track, and so can any viewer.

```sh
mise run speech-model                       # the default model, once
mise run speech-model -- silero-v5.1.2      # and the model of voice activity
mise run transcriber -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --namespace xmoq/broadcast/demo
```

It runs until Ctrl-C or SIGTERM, and connects again after a session ends, as the gatherer does, also after a GOAWAY (`Pause::SERVING` of [`xmoq-again`](../again/src/lib.rs)). The model is loaded once, at the start, and stays over the sessions. With a flag it does not know it prints its flags, what each is without it and their ranges (`USAGE` in `src/args.rs`). **Building it needs a C++ compiler and cmake**, for whisper.cpp ([`xmoq-speech`](../speech/README.md)).

## What it listens to, and what it publishes

| | |
|---|---|
| Listens to | Every namespace announced under a prefix of `--namespace` (up to 8, none under another), found with one `SUBSCRIBE_NAMESPACE` for each prefix. Of each, the first Opus track with LOC packaging that its catalog names (MSF §5), read with a SUBSCRIBE and a Joining FETCH, as the terminal viewer reads it. A catalog that names another audio track later moves the transcriber to that track |
| Publishes | The track `speech` of `xmoq/transcript/<prefix>` for each prefix, with PUBLISH, from the start of each session. One piece is one Group, and its payload is in [`fixtures/transcript-tracks/`](../../fixtures/transcript-tracks/README.md) |
| Answers | A FETCH of a transcript, from the latest `--keep` pieces it holds in memory. The pieces of the range that were let go of get one End of Unknown Range indicator, which in Descending order names no number before the first piece, as the Rust client answers (`plan_kept` of `xmoq-publisher`; `Transcript::fetch`) |

WHY these, and WHY NOT the other choices (every namespace of the relay; a room that says where the broadcast is; a track `transcript` in the broadcast's namespace; PUBLISH_NAMESPACE): at the top of `src/bin/xmoq-transcriber/session.rs` and of `crates/transcript-tracks/src/names.rs`.

**The pieces are numbered from the time the transcriber started**, in milliseconds since the Unix epoch, and up by one from there. A transcriber that starts again therefore goes on above the Group IDs it gave before, which a relay and its subscribers may still have (why, at `Transcript::new` in `src/transcript.rs`). **Nothing is kept on disk**: the pieces are lost when it stops. Whether and for how long a transcript is to be kept is [undecided](../../docs/open-questions.md). A piece that is not sent live is kept in memory, and a FETCH gets it.

**Readers for every track** (why, at the top of `src/bin/xmoq-transcriber/reader.rs`): each takes the next piece that was cut. With `--readers` more than one, a later piece can be done before an earlier one, of the same voice as well, and is numbered first. A piece that finds `--waiting` pieces waiting for it is dropped and counted (`PIECE_DROPPED`). Each `SPEECH` line has how long the piece waited, how long the reader took, and how long after the end of the piece its words were there (`waited_ms`, `took_ms`, `behind_ms`); `behind_ms` holds where the broadcast's Timestamps are wall-clock time, as the demo's pages make them.

**`--engine command`**: how the sound is cut, and the track and the numbering of the words, are the same with every reader (`Engine` of `xmoq-speech`). The program and its arguments are split at spaces (a program whose arguments have spaces is put in a script). Each piece is written as a WAV file (16 kHz, 16 bits, one channel) to a directory of the transcriber's own under the system's temporary directory, which goes when the transcriber ends (on SIGTERM and Ctrl-C too; not after SIGKILL), as does a program that runs then; the file is removed after the program. The words are its standard output, trimmed. A program that runs over `--engine-timeout-ms` is killed with the processes it started (it runs in a process group of its own), and the piece is not read (`PIECE_NOT_READ`); an exit that is not 0, output over `--engine-most-output` and output that is not UTF-8 are the same. `--model` is refused with it.

**`--auth-key-hex`**: the transcriber makes `sub` in `<prefix>/*` for its SETUP, its SUBSCRIBE_NAMESPACE, SUBSCRIBEs and FETCHes, and `pub` in `xmoq/transcript/<prefix>` for its PUBLISH. For the demo alone, as the gatherer's (provisional).

What it prints, one line for each thing it does (`MODEL_LOADED`, `VAD_LOADED`, `WATCHING`, `BROADCAST`, `AUDIO_SUBSCRIBED`, `TRANSCRIPT_PUBLISHED`, `PIECE_CUT`, `SPEECH`, `PIECE_DROPPED`, `FETCH_ANSWERED`, ...), is what `interop/transcriber.ts` waits for.

## Not done

- Through `moqtail`'s relay no run has a voice reach it: the run there (`--case catalog-refused` of `interop/transcriber.ts`) ends once the page takes the SUBSCRIBE for its catalog again, since that relay answers the Joining FETCH for the catalog with nothing.
- On Linux, only the interop job of CI builds it and runs it, with the model `tiny` (`scripts/ci/interop.sh`). How fast the larger models read on the processor of a Linux machine was not measured.
- A catalog that cannot be read, or that names no Opus track, is passed over (`CATALOG_UNREAD`, `CATALOG_WITHOUT_AUDIO`): the subscription to the catalog and the audio track listened to before, where there is one, stay as they were, and the catalog is not asked for again for it. A catalog track that is malformed (§2.4.2) is asked for again, up to `--ask-again-times` times in a row (`keeping` in `src/listen.rs`).
