# @xmoq/tracks

The tracks of an endpoint over a session of [`@xmoq/moqt`](../moqt/README.md), whatever they carry: one track kept on one session, asked for again when its subscription ends or is refused, and the tracks a connection serves to its peer. What a track carries (video, sound, a catalog, lines) is the business of the packages above.

| Export | What it is |
|---|---|
| `keepTrackOnSession(options)` | Plays one track on one session for as long as the session lasts: when its subscription ends, or a SUBSCRIBE for it is refused (REQUEST_ERROR), it is asked for again on the same session after `againMs`, or after the Retry Interval of the refusal (transport §10.6.2). Its states are `KeptTrackState` |
| `stayOnSession(options)` | What `keepTrackOnSession` and `keepPlaying` of [`@xmoq/media`](../media/README.md) ask again by: the pauses of `createAskAgain` of `@xmoq/moqt`, with no I/O and no timer of its own |
| `ServedTrack`, `serveTracks(connection, tracks)` | A track that answers the SUBSCRIBE and FETCH requests for it, and the `serve` handlers of a connection that hand each request of the peer to the first track that takes it and refuse the rest with DOES_NOT_EXIST. The publishers of `@xmoq/media` are such tracks, and so is the catalog publisher of [`@xmoq/msf`](../msf/README.md) |

It is in layer 1 (`scripts/ci/check-layers.sh`), on `@xmoq/moqt` alone. The table of how a kept track goes on after each end (`fixtures/kept-track/`, which the Rust viewers' kept track reads as well) is to be read by its tests when it is there.
