# The Opus decoder that overflows, set against libopus

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal; open question on the CLI's decoders

## Objective

The crate `opus-decoder` 0.1.1, which `xmoq-watch` decodes sound with, shifts a `u8` by 8 or more on packets a browser encodes (`src/celt/vq.rs:118`, `mask |= 1 << i`): a build that checks for overflow panics there, and one that does not goes on with the shift wrapped. Whether what it decodes then is what libopus decodes, and what there is to choose from. Nothing is decided here.

## Environment

One machine, macOS on Apple silicon. Packets captured with the own Rust client: 9,000 (3 minutes) of the Opus track of the demo's publishing page in headless Chromium 156 (one channel; a tone with a beep every 2 seconds), and 9,000 of `audio_monotonic_128kbps_opus` of `mlmpub` (moqlivemock v0.16.1; two channels). `spikes/opus-compare` decodes each file with `opus-decoder` 0.1.1 and with libopus through the crate `opus` 0.4.0 (`opusic-sys` 0.7.5), to float samples at 48 kHz, one decoder of each kind for the whole file.

## Commands

```sh
spikes/opus-compare/capture.sh 9000
cd spikes/opus-compare
mise exec cmake@3.31.6 -- cargo build --profile checked     # optimized, with overflow checks
mise exec cmake@3.31.6 -- cargo build --release
target/checked/opus-compare captured/chromium.packets 1    # which packets overflow
target/release/opus-compare captured/chromium.packets 1    # the samples, set against libopus
```

The build with checks catches the panic of every packet and writes down its number; the decoder is made anew after one. The release build decodes every packet with both decoders and takes the difference of every sample.

## Observed results

| | Chromium, 1 channel | mlmpub, 2 channels |
|---|---|---|
| Packets, and samples compared | 9,000; 8,640,000 | 9,000; 17,280,000 |
| Packets at which `opus-decoder` overflows, all at `vq.rs:118` | **136** | 0 |
| Packets either decoder refused, or decoded to another length | 0 | 0 |
| Packets with a sample more than 0.001 of full scale (-60 dB) apart | **0** | 0 |
| The greatest difference of a sample | 0.000034 (-89 dB) | 0.000019 |
| RMS of the difference; RMS of what libopus decoded | 0.0000090; 0.215 | 0.0000063; 0.030 |

- **In a release build the overflow changed nothing that can be measured in these packets**: the 136 packets that overflow are as close to libopus as the 8,864 that do not, and all of it is within what two decoders in floating point differ by.
- That is what was seen, not what has to be. The line sets a bit for every block of a band that has a pulse; with more than 8 blocks, the wrapped shift sets one of the lower 8 bits where the reference would set a higher one. The bits are looked at again only where a band is tested for having collapsed, so a packet needs both to sound different. None of the 136 did.
- About 1 packet in 66 of the browser's overflows: a debug build of anything that uses the decoder stops within the first seconds (0.3 s when `xmoq-watch` was first built for the interop runs).

## Upstream

Read on 2026-10-12 with `gh api` (`TadeuszWolfGang/Rusopus`, the repository of the crate's `Cargo.toml`) and on crates.io.

- 0.1.1 of 2026-03-12 is the newest release. The default branch has one commit since, of 2026-07-11, which makes the decoder faster and leaves `vq.rs:118` as it is.
- **Pull request #1, "fix: prevent panic in extract_collapse_mask when block index >= 8", is open since 2026-04-15 and not merged** (one comment, last touched 2026-10-05). It leaves the bit out where the index is 8 or more. Its author found the panic in an audio pipeline of their own.
- Issue #4 asks for a release of the faster FFT; issue #3 is about wasm.

## What there is to choose from

| | What it needs | What it leaves |
|---|---|---|
| Stay, as now | The profile of the root `Cargo.toml` that builds this one crate without overflow checks in a debug build as well (in place since the interop runs use the viewer) | A decoder whose one known fault is open upstream for half a year; any other overflow in it is as invisible |
| Stay, and catch a panic of the decoder | `catch_unwind` around the decode in `sound.rs`, a new decoder after one, and a panic hook that does not put the terminal back for a panic that is caught (today's does, for every panic) | The same decoder; a packet that panics is a gap in the sound and not the end of the viewer. In a release build this site does not panic at all |
| Stay, with the pull request's fix | A `[patch.crates-io]` to a commit of that pull request, or a copy of the crate in the repository | A dependency that is no release; the fix is not the maintainer's |
| libopus (the crate `opus`) | cmake and a C compiler: cmake in `mise.toml` (it was run here with `mise exec cmake@3.31.6`), and on the machines of CI. `opusic-sys` builds the libopus it carries (BSD-3-Clause) | The reference decoder, at a sixtieth of the time ([record](2026-10-12-opus-decoded-and-played-with-rust-crates.md)); a build that is no longer Rust alone, which is what was decided against on 2026-10-12 |
| Another decoder in Rust | Trying it as this one was tried: none was. On crates.io today, by the downloads it counts: `opus-rs` 0.1.37 (192,000, last release 2026-10-07), `unsafe-libopus` 0.2.0 (libopus through c2rust; 78,000, 2024-11), `rusty-opus` 1.0.1 (61,000, 2026-10-08), `ruopus` 0.1.2 (8,000, 2026-07), `mousiki` 0.2.1 (2,800, 2026-03). `opus-decoder` has 166,000 | Not known |

## Not measured

- Speech and music a microphone gives; other bitrates and frame lengths; packets of another encoder than Chromium's and moqlivemock's.
- Loss: no packet was left out, so the concealment of the two decoders was not compared.
- Whether the bit the wrapped shift sets can be heard in a packet made for it.
- The pull request's fix, built and compared.

## Next action

The decision on the sound's decoder (2026-10-12) is to be looked at again with this.
