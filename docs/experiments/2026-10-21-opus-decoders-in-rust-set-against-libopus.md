# Opus decoders in Rust set against libopus

- Date: 2026-10-21
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal; open question on the CLI's decoders

## Objective

`xmoq-watch` decodes its sound with `opus-decoder` 0.1.1, which overflows on packets a browser encodes ([record of 2026-10-12](2026-10-12-the-opus-decoder-that-overflows-set-against-libopus.md)). The user chose to try other decoders in Rust alone before deciding. This record lists the decoders in Rust that do not need C, sets each against libopus sample by sample on the same kind of packets as the record of 2026-10-12, and measures their CPU time. Nothing is decided here, and `crates/watch` is unchanged.

## Environment

One machine, macOS on Apple silicon (Apple M3 Pro), rustc 1.99.0. Packets captured anew with `spikes/opus-compare/capture.sh` (the files of 2026-10-12 were not kept): 9,000 (3 minutes) of the Opus track of the demo's publishing page in headless Chromium (one channel; a tone with a beep every 2 seconds), and 9,000 of `audio_monotonic_128kbps_opus` of `mlmpub` (moqlivemock v0.16.1; two channels). `spikes/opus-candidates` decodes each file with every decoder, one decoder of each kind for the whole file, to float samples at 48 kHz, and sets them against libopus through the crate `opus` 0.4.0 (`opusic-sys` 0.7.5).

## The decoders

Found on crates.io (search for "opus" and "opus decoder", by downloads) and read on GitHub with `gh api`, both on 2026-10-21. Every crate was read as published (the `.crate` file). Bindings to libopus (`opus`, `audiopus`, `opusic-c`, `opus-codec`, `shiguredo_opus` and others) are left out, as they need a C compiler. `symphonia` 0.6.1 has no Opus decoder of its own: its adapter for Opus is libopus.

| Crate | Version, released | Licence | What it is | Repository on 2026-10-21 | `unsafe` in `src/` |
|---|---|---|---|---|---|
| `opus-decoder` (now) | 0.1.1, 2026-03-12; 2 releases | MIT OR Apache-2.0 | Own decoder | 1 contributor, last commit 2026-07-11; the fix of the overflow (pull request #1) open since 2026-04-15 | none (`forbid(unsafe_code)`) |
| `opus-rs` | 0.1.39, 2026-10-20; 37 releases since 2026-02-28 | BSD-3-Clause | Port of libopus 1.6 (float), encoder and decoder, `no_std` | 6 contributors, 100 or more commits in the last 90 days, no open issues, 53 stars | 181 blocks, 71 functions (fixed-capacity buffers, SIMD, FFT, range coder) |
| `rusty-opus` | 1.0.2, 2026-10-17; 9 releases since 2026-07-29 | BSD-3-Clause | Derived from `opus-rs` (its README) | 4 contributors, 43 commits in 90 days, 4 open issues | 104 blocks, 69 functions |
| `opus-pure` | 0.2.2, 2026-09-28; 4 releases since 2026-08-27 | BSD-3-Clause | Fork of `opus-rs` (its `ATTRIBUTION.md`), with Ogg | 2 contributors, 19 commits in 90 days | 77 blocks, 55 functions |
| `ropus` | 0.12.18, 2026-05-11; 5 releases | BSD-3-Clause | Port of libopus, fixed point, said to be bit-exact against the fixed-point reference | 1 contributor, 100 or more commits in 90 days, last 2026-09-13 | 11 blocks, none in the decoder's files |
| `ruopus` | 0.1.2, 2026-07-01; 2 releases | MIT | Own decoder | 1 contributor, last commit 2026-07-07 | 24 blocks, 15 functions (SIMD) |
| `unsafe-libopus` | 0.2.0, 2024-11-21; 5 releases | BSD-3-Clause | libopus turned into Rust by c2rust | 1 contributor, last commit 2024-07-08 | the whole API is `unsafe fn` (559) on raw pointers |
| `mousiki` | 0.2.1, 2026-03-26; 3 releases | MIT | Port of libopus; its README says its own `decoder::Decoder` is SILK only, so the C-style `opus_decoder` was used | **the repository named on crates.io (`cijiugechu/mousiki`) answers 404** | 24 blocks (NEON) |

`ropus` has a build script that compiles C with `cc`, only when the reference sources lie beside the crate; from crates.io they do not, it compiles nothing, and its neural concealment has no weights. No other crate has a build script or a C dependency.

## Commands

```sh
spikes/opus-compare/capture.sh 9000
cd spikes/opus-candidates
mise exec cmake@3.31.6 -- cargo build --release
mise exec cmake@3.31.6 -- cargo build --profile checked    # optimized, with overflow checks
target/release/opus-candidates ../opus-compare/captured/chromium.packets 1 3
target/release/opus-candidates ../opus-compare/captured/mlmpub.packets 2 3
target/checked/opus-candidates ../opus-compare/captured/chromium.packets 1 1
target/checked/opus-candidates ../opus-compare/captured/mlmpub.packets 2 1
NEAREST=mousiki target/release/opus-candidates ../opus-compare/captured/chromium.packets 1
```

The third argument is how many times each decoder decodes the whole file; the CPU time of the thread is taken around each pass, and the least is shown. A panic is caught, counted with where it happened, and the decoder is made anew.

## Observed results

Expectations were written before the runs; where they were wrong is said below the tables.

Chromium, 1 channel, 8,640,000 samples:

| Decoder | Panics, release / checked | Packets more than 0.001 (-60 dB) apart | Greatest difference | RMS of the difference (libopus 0.215) | CPU s for 180 s of sound | Times libopus |
|---|---|---|---|---|---|---|
| libopus | 0 / 0 | | | | 0.121 | 1 |
| `opus-decoder` | 0 / **169**, all at `celt/vq.rs:118` | 0 | 0.000035 (-89 dB) | 0.0000090 | 11.1 | 92 |
| `opus-rs` | 0 / 0 | 0 | below 0.000001 (-131 dB) | 0.00000002 | 0.147 | 1.2 |
| `rusty-opus` | 0 / 0 | 0 | below 0.000001 (-131 dB) | 0.00000002 | 0.145 | 1.2 |
| `opus-pure` | 0 / 0 | 0 | below 0.000001 (-131 dB) | 0.00000002 | 0.128 | 1.1 |
| `ropus` | 0 / 0 | 0 | 0.000428 (-67 dB) | 0.000042 | 0.151 | 1.2 |
| `ruopus` | 0 / 0 | 0 | 0.000008 (-103 dB) | 0.00000051 | 11.8 | 98 |
| `unsafe-libopus` | 0 / 0 | 0 | below 0.000001 (-133 dB) | 0.00000002 | 0.117 | 1.0 |
| `mousiki` | 0 / 0 | **8,999** | 0.61 (-4 dB) | 0.040 | 0.129 | 1.1 |

mlmpub, 2 channels, 17,280,000 samples:

| Decoder | Panics, release / checked | Packets more than 0.001 apart | Greatest difference | RMS of the difference (libopus 0.030) | CPU s for 180 s | Times libopus |
|---|---|---|---|---|---|---|
| libopus | 0 / 0 | | | | 0.143 | 1 |
| `opus-decoder` | 0 / 0 | 0 | 0.000019 (-95 dB) | 0.0000063 | 19.4 | 136 |
| `opus-rs` | 0 / 0 | 0 | below 0.000001 (-151 dB) | below 0.00000001 | 0.212 | 1.5 |
| `rusty-opus` | 0 / 0 | 0 | below 0.000001 (-151 dB) | below 0.00000001 | 0.211 | 1.5 |
| `opus-pure` | 0 / 0 | 0 | below 0.000001 (-151 dB) | below 0.00000001 | 0.157 | 1.1 |
| `ropus` | 0 / 0 | 0 | 0.000044 (-87 dB) | 0.0000082 | 0.225 | 1.6 |
| `ruopus` | 0 / 0 | 0 | 0.000001 (-122 dB) | 0.00000013 | 20.5 | 144 |
| `unsafe-libopus` | 0 / 0 | 0 | below 0.000001 (-153 dB) | below 0.00000001 | 0.161 | 1.1 |
| `mousiki` | 0 / 0 | **4,500** | 0.085 (-21 dB) | 0.011 | 0.216 | 1.5 |

No decoder refused a packet or decoded one to another length than libopus. The three passes of each decoder were within 7% of each other.

- **`opus-rs`, `rusty-opus` and `opus-pure` decode within -130 dB of libopus, do not overflow with the checks on, and take 1.1 to 1.5 times libopus's CPU time** (0.15 s for 3 minutes of one channel, 60 to 90 times less than `opus-decoder`). Their samples are the same to the printed digits; `rusty-opus` and `opus-pure` derive from `opus-rs`.
- `unsafe-libopus` is as close and as fast, but it is libopus's C as Rust: every call is `unsafe` on a raw pointer, and its last release is of 2024-11.
- `ropus` is within -67 dB, the error of decoding in fixed point; no packet is more than 0.001 apart.
- `ruopus` is close (-103 dB) and as slow as `opus-decoder`.
- `mousiki` decodes something else than libopus: 8,999 of 9,000 browser packets differ by up to 0.61, and in the two-channel file 4,500 of 9,000 (half of them). Its output is not libopus's shifted in time or of the other sign (no shift within ±2,000 samples and no sign brings it nearer). Why was not looked into; with its repository gone it was not taken further.
- `opus-decoder` overflowed at 169 of these 9,000 browser packets (136 of the other 9,000 on 2026-10-12), always at `vq.rs:118`. In the release build its samples are as close to libopus as on 2026-10-12. The checked run's differences for `opus-decoder` (1,281 packets) follow from making the decoder anew after each panic and are not counted as its error.
- Against the expectations: `opus-decoder` was expected at about 60 times libopus's CPU time, as the record of 2026-10-12 says; here it is 92 and 136 times. Where the difference comes from was not looked into. `mousiki` was expected close to libopus. The others are as expected.

## Not measured

- Loss: no packet was left out, so concealment was not compared. `xmoq-watch` does not ask its decoder to conceal today.
- Speech and music a microphone gives; other bitrates and frame lengths; packets of other encoders than Chromium's and moqlivemock's.
- Whether the `unsafe` code of `opus-rs` and its forks is sound: it was counted, not read.
- A build of `xmoq-watch` with any of these decoders, and a person listening.
- `opus-rs` in a debug build of the whole program (only optimized builds with and without overflow checks ran).

## Next action

The user decides whether the sound's decoder changes (to `opus-rs` or one of its forks), stays `opus-decoder`, or becomes libopus.
