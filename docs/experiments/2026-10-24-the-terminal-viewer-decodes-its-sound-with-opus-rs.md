# The terminal viewer decodes its sound with opus-rs

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal; the CLI's decoders

## Objective

The user decided (open question Q26) that `xmoq-watch` decodes its sound with `opus-rs` 0.1.39 in place of `opus-decoder` 0.1.1, on the comparison of [2026-10-21](2026-10-21-opus-decoders-in-rust-set-against-libopus.md). Whether the viewer plays with it as before.

## What changed

- `crates/watch/src/bin/xmoq-watch/sound.rs`, the one file that names the decoder: `opus_rs::OpusDecoder::new(48000, 1)` and `decode(packet, 5760, pcm)`, the samples of one channel at 48 kHz as before. Its WHY and WHY NOT are there.
- `crates/watch/Cargo.toml`: `opus-rs = "=0.1.39"`. The README's table of dependencies has its licence (BSD-3-Clause), its `unsafe` (counted, not read) and that a 0.x crate with 37 releases in eight months can change its API.
- The root `Cargo.toml` no longer builds one crate without overflow checks in a debug build: that was for `opus-decoder`, which overflowed on packets of a browser. `opus-rs` did not overflow on the same 9,000 packets with the checks on (2026-10-21).

## Commands

```sh
cargo build --release --locked -p xmoq-watch
cd interop
bun watch-rust.ts --watch page --port 48774
bun watch-rust.ts --watch from-mlmpub --port 48781
```

## Observed results

| Run | Frames drawn | Packets of sound | Refused by the decoder | Underruns | Peak / RMS of what was handed to the output |
|---|---|---|---|---|---|
| A page's broadcast through the own relay, 8 s (a tone with a beep every 2 s) | 237 | 399 | 0 | 0 | 0.683 / 0.206 |
| `mlmpub` of moqlivemock, 8 s | 189 | 385 | 0 | 0 | 0.074 / 0.030 |

- **Every packet decoded**, and the sound was not silence. The picture of the page came -8.9 ms after the sound at the median of 4 marks.
- With no sound output (`--sound off`, as the interop runs have it): **nobody listened.** Hearing the sound is the user's.
- A debug build of the viewer was not run; `mise run watch` builds it with `--release`.

## Decision or next action

None here: the output (`cpal`) is still open (Q37).
