# Opus decoded and played with Rust crates

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal; open question "CLI decoder/audio backend"

## Objective

The terminal player of `spikes/terminal-player` hands its sound to ffmpeg, and plays on macOS alone. Whether a Rust viewer can decode an Opus track and play it on the machine's output with crates, and what the candidates need and cost. Material for the open question; no crate is chosen.

## Environment

One machine, macOS on Apple silicon, loopback. A page of Chromium encodes a tone as Opus and publishes it on the track `audio` (the page of `interop/`, started by `spikes/kitty-graphics/publish-page.ts`), through `xmoq-relay`; `xmoq-moqt-client` subscribes and prints the Objects; `spikes/rust-audio` (`opus-play`) decodes and plays them. Rust 1.99.0 (`rust-toolchain.toml`), `--release`.

## Candidates

Read on crates.io and in the repositories on 2026-10-12.

| Crate | What it is | License | Needs to build | Notes |
|---|---|---|---|---|
| `opus` 0.4.0 | Safe bindings to libopus, which `opusic-sys` 0.7.5 carries and builds | MIT/Apache-2.0; `opusic-sys` and libopus BSD-3-Clause | a C compiler and **cmake** | Since 2016, 2.8 million downloads, last release 2026-08. Without cmake the build fails in `opusic-sys` |
| `opus-decoder` 0.1.1 | A decoder in Rust alone, `#![forbid(unsafe_code)]`, one dependency (`thiserror`) | MIT OR Apache-2.0 | nothing but Rust | First and only release 2026-03, 166,000 downloads. Its README says it passes the 12 test vectors of RFC 8251 (not run here). The README's badge points to a repository that does not exist; the one of `Cargo.toml` (`TadeuszWolfGang/Rusopus`) does |
| `cpal` 0.18.2 | Output (and input) of the machine's sound | Apache-2.0 | macOS: nothing more (CoreAudio through `objc2` crates). Linux: ALSA's headers and `pkg-config` (`alsa-sys`), read in its `Cargo.toml`, not built here | Since 2014, last release 2026-08. `rodio` is built on it |

Seen and not tried: `audiopus_sys` (cmake as well, last release 2021), `unsafe-libopus` (libopus put through c2rust, 2024), `mousiki` and several more decoders in Rust of 2025 and 2026 with few downloads. No second output crate was tried: the others found are built on `cpal`.

## Commands

```sh
spikes/rust-audio/play-page.sh 500 0                                        # opus-decoder
FEATURES=libopus mise exec cmake@3.31.6 -- spikes/rust-audio/play-page.sh 500 0   # libopus
```

`opus-play` reads the Objects, decodes each packet to 48 kHz float samples, and keeps what waits in a queue the output's callback takes from. It starts playing when 100 ms wait; a callback that finds too little after that is an underrun, gets silence and waits for 100 ms again; more than a second waiting is dropped. The volume is 0 unless given: the peak and the RMS are of the samples the output asked for, before the volume. cmake was not on the machine; `mise exec` gave it to that one command, and `mise.toml` is as it was.

## Observed results

One run each, 500 packets of 20 ms (10 seconds), volume 0. **Nobody listened.**

| | `opus-decoder` 0.1.1 | `opus` 0.4.0 (libopus) |
|---|---|---|
| Packets decoded / refused | 500 / 0 | 500 / 0 |
| Sound handed to the output | 10.00 s | 10.00 s |
| Underruns, of 938 callbacks | 0 | 0 |
| Peak / RMS of what was handed over | 0.5032 / 0.3526 | 0.5032 / 0.3526 |
| Time in the decoder, all packets | 1,274.8 ms | 20.5 ms |
| CPU of the process (user + sys) | 1.33 s | 0.08 s |
| Maximum resident set | 14.1 MB | 13.6 MB |

- Both play the track without an underrun, and hand over the same tone: a sine of half the full scale has an RMS of 0.354.
- **The decoder in Rust took about 60 times the time of libopus**: 2.5 ms for a packet of 20 ms, an eighth of one core, against 0.04 ms. Enough for one track; not measured on a slower machine.
- The device took 48 kHz float samples, 2 channels; the one channel of the track was written to both. A device that does not take 48 kHz ends the spike: nothing resamples.

## Not done

- **Sound and picture by one clock.** The picture is another process (`spikes/kitty-graphics`); the two would have to be one program that holds both to the Timestamps of LOC. `opus-play` does not read the Timestamp.
- An Object that does not come: no concealment is asked of the decoder, the packets are played one after the other.
- Linux and Windows: nothing was built or run there.
- The broadcast of the demo (`mise run demo`), stereo, other frame lengths, a link that loses.
- Listening.

## Next action

The open question stays open. What it turns on, from this: whether cmake and a C compiler are accepted for the terminal client (then libopus, the reference, at a sixtieth of the cost), or a build with Rust alone is wanted (then a decoder that is half a year old); and ALSA's headers on Linux for the output either way.
