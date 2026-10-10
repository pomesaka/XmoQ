# WebTransport and WebCodecs per browser: published sources, and Firefox measured

- Date: 2026-10-10
- Commit SHA: the commit that adds this record. Nothing in `crates/`, `packages/` or `interop/` was run or changed for it
- Roadmap item: groundwork for Phase 1 ("Browser WebCodecs publisher", "Decode and render video"). Material for the open decision "Codec/profile, Track/Object packaging, audio pipeline" in [open questions](../open-questions.md). No roadmap item is closed by this record, and nothing is decided in it

## Objective

Collect, per browser, what is published about WebTransport and WebCodecs today: which browsers can run XmoQ at all, and which codecs a publisher in one browser can encode for viewers in another. The first part is reading, not measuring. Chromium was measured before, in [the WebCodecs loopback record](2026-10-10-webcodecs-loopback.md) and the WebTransport records of the same day. The last section of the results adds runs in the Firefox build of Playwright, made after the reading; the WebKit build of Playwright did not open a page on this machine.

Browsers: Chromium (Chrome, Edge), Firefox, Safari on macOS, Safari and WebKit on iOS and iPadOS; the stable version and what the pre-release channels have.

## Environment

| Item | Value |
|---|---|
| How | Every source was fetched over HTTPS on 2026-10-10, by two research passes run for this record, one for each API. No statement here is written from memory |
| Kinds of source | Specifications (W3C); release notes and vendor blogs; compatibility tables (MDN browser-compat-data, caniuse); the engines' source and IDL at the tip of `main`; bug trackers (Bugzilla, bugs.webkit.org, w3c GitHub issues); web-platform-tests results (wpt.fyi) |
| Full source tables | [`spikes/browser-support/webtransport-sources.md`](../../spikes/browser-support/webtransport-sources.md) and [`spikes/browser-support/webcodecs-sources.md`](../../spikes/browser-support/webcodecs-sources.md): one row per statement, with its URL and a verbatim quote. The tables below name the kind of source; the URL is in those files |
| Engine source read | Chromium `main` (157), Firefox `main` (160.0a1), WebKit `main`, all of 2026-10-10. **This is the tip of development, not a shipped build.** A row that rests on it alone says "main" |
| Browsers on this machine | Google Chrome 155.0.8059.39, Chromium 156.0.8078.4 (Playwright), Safari 17.4. Safari 17.4 is older than the first Safari with WebTransport (26.4) and was not run |
| Browsers installed for the runs | Into Playwright's cache, with `bun x playwright-core install firefox webkit` (`playwright-core` 1.64.0): Firefox 157.0 (Playwright build 1555) and WebKit build 2251. **Neither is a shipped browser.** The Firefox is a build of 157.0 with Playwright's patches; the stable Firefox is 157.0.1. The WebKit is the last build Playwright made for macOS 14 ("a frozen webkit browser which does not receive updates anymore on mac14-arm64"); Playwright reports its version as 27.2, the version of its current WebKit, and what the frozen build really is was not found out |
| Machine, relay | macOS 14.4 arm64, loopback. `xmoq-relay` 0.0.0 at `a440f0c`, debug build, `web-transport-quinn` 0.13.2, `quinn` 0.11.12, `quinn-proto` 0.11.19, the certificate of the interop runs |

Nine statements were fetched a second time for this record, straight from the source, and agree with the tables: the versions of WebTransport, `serverCertificateHashes`, `VideoEncoder` and `AudioEncoder` in browser-compat-data; Chromium's IDL of `WebTransportError`; "Added support for WebTransport" in the Safari 26.4 release notes; the Firefox preference for WebCodecs on Android; the AV1 line of the Safari 17.5 release notes; the status of Bugzilla 2049470 and 2078582; and the WebTransport draft version in Chromium's client. The other statements are as the research passes recorded them.

## Commands

For the reading, none for the tree. The sources were fetched with `curl` and a web fetch tool; the nine checks above were of this form:

```sh
curl -sL https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/WebTransport.json | jq '.api.WebTransport.__compat.support'
curl -sL https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/modules/webtransport/web_transport_error.idl
curl -sL https://developer.apple.com/tutorials/data/documentation/safari-release-notes/safari-26_4-release-notes.json
```

For the runs, from the repository root, after `cargo build --locked -p xmoq-relay` and one interop run for the certificate:

```sh
cd interop && bun x playwright-core install firefox webkit && cd ..
bun spikes/browser-support/probe.ts <chromium|firefox|webkit>
bun spikes/browser-support/wk-min.ts
bun spikes/browser-support/connect.ts <chromium|firefox> 47400
bun spikes/browser-support/moqt.ts <chromium|firefox> --port 47410
bun spikes/browser-support/moqt.ts <chromium|firefox> --port 47420 --objects 10000 --bytes 2000
bun spikes/browser-support/error-and-codecs.ts <chromium|firefox>
bun spikes/browser-support/moqt.ts <chromium|firefox> --port 47430 --close-publisher
bun spikes/browser-support/codecs.ts          # twice
```

## Observed results

### Versions on 2026-10-10

| Browser | Stable | Pre-release |
|---|---|---|
| Chrome | 155.0.8059.40 since 2026-10-06; 156.0.8078.12 is on the stable channel for 0.5% of installs | Beta 156, Canary 157 |
| Edge | 155.0.4283.45; Android 154 | Beta 156, Canary 157 |
| Firefox | 157.0.1 (2026-10-09), desktop and Android | Beta 158.0b5, Nightly 160.0a1 |
| Safari | 27.0 (2026-09-14), for macOS 27, macOS 26, macOS Sequoia, iOS 27, iPadOS 27 | 27.2 beta; Technology Preview 254 (2026-10-08) |

### WebTransport

| | Chrome, Edge | Firefox | Safari (macOS, iOS, iPadOS) |
|---|---|---|---|
| In stable since | 97 (compat table, chromestatus) | 114 (compat table, release note); preference `network.webtransport.enabled`, on by default | 26.4, 2026-03-24 (release note: "Added support for WebTransport") |
| `serverCertificateHashes` | 100 | 125 (compat table only) | 26.4 (compat table only) |
| Limits on the pinned certificate, in the code | 14 days; SHA-256 only; RSA still accepted with a TODO | 14 days; SHA-256 only; RSA refused | two weeks; SHA-256; no check of the key type found (main) |
| `protocols` option and `protocol` attribute | 143 | 155 (2026-09-01) | 26.4 (compat table only) |
| Datagrams | 97 | 114 | 26.4; an open report says `datagrams` is undefined on iOS 26.6 (WebKit 322201) |
| Over HTTP/2 | no | no (open bugs) | yes, as a fallback; `requireUnreliable` switches it off |
| Workers | yes | yes | yes |
| `WebTransportError` constructor (main) | one dictionary | one dictionary | `(options)` or `(message, options)` |
| `streamErrorCode` (main) | constructor clamps to 0 to 255; the attribute is 32 bits | 0 to 255 in both | 32 bits, as specified |
| `closeCode` (main) | 32 bits | 32 bits | 32 bits |
| Tests of web-platform-tests under `/webtransport/` passed, stable | 602 of 1261 | 933 of 1261 | 908 of 1261 |

What follows for XmoQ:

- **All three engines have WebTransport in stable, with certificate pinning and protocol negotiation.** XmoQ needs `protocols` to negotiate `moqt-18`, so the oldest browsers that can run it are Chrome 143, Firefox 155 and Safari 26.4. Whether Safari 26.4 or 27 on macOS Sequoia has WebTransport is not stated anywhere read: the release notes list Sequoia, and WebKit's source enables the API for OS 26 and later only.
- **The form of `WebTransportError` that `@xmoq/moqt` builds is right for all three as read from the IDL**: it tries the form of the specification first, which WebKit takes, and then one dictionary, which Chromium and Firefox take. The W3C working group decided to keep the specification and let browsers drop the old form (w3c/webtransport 715, open). Only Chromium was run.
- **A reset or STOP_SENDING code above 255 cannot be sent from Chromium or Firefox.** XmoQ's own codes are `0x12` or less; `SubgroupWriter.reset()` already refuses more.
- **Firefox 157 may not connect to the own relay.** An unconfirmed report (Bugzilla 2078582) says a session with `serverCertificateHashes` to an HTTP/3 server fails in 157 and worked in 156. The change it is traced to (neqo pull request 3756) makes Firefox require three things of the server: the HTTP/3 datagram setting, QUIC datagrams, and reliable stream reset in the transport parameters. The own relay announces datagrams (`crates/moqt-io/src/wt.rs`); whether `quinn` 0.11.12 announces reliable stream reset was not looked at, and Firefox was not run.
- **Safari stops receiving after 16 MB or 7600 streams in a session**, by an open report made with a MoQ player (WebKit 319818, "Flow control never refills"). One Object a stream reaches 7600 streams quickly. Not run here.
- **Chromium offers draft 02 of WebTransport over HTTP/3 by default** and draft 07 behind a feature that is off; Firefox's library carries the draft 02 setting; which draft Safari speaks was not found, as it is in a closed framework. The interop runs show that `web-transport-quinn` 0.13.2 and Chromium agree.

### WebCodecs

| | Chrome, Edge | Firefox | Safari |
|---|---|---|---|
| Video interfaces in stable since | 94 | 130, desktop only | 16.4 |
| Audio interfaces in stable since | 94 | 130, desktop only | 26.0 |
| Android or iOS | Android as desktop (compat table) | **not in Firefox for Android**: the preference is on in Nightly only, and there the support checks still answer no (main; Bugzilla 1934008 open) | iOS and iPadOS as macOS, by the release notes; no measurement of iOS was found |
| Workers | window and dedicated worker | the same | the same |

Codecs through WebCodecs. "SW" is a software codec in the browser or the OS, "HW" a hardware codec of the platform. A cell without a qualifier was not sourced further.

| Codec | Chrome, Edge desktop | Firefox desktop 157 | Safari 27 |
|---|---|---|---|
| H.264 encode | yes, SW or HW | yes, through the platform's encoders | yes |
| H.264 decode | yes | yes | yes |
| VP8 encode and decode | yes, SW | yes, SW | yes, SW |
| VP9 profile 0 encode | yes, SW | yes, SW | yes, SW |
| VP9 decode | yes | yes | yes |
| AV1 encode | yes, SW | yes, SW | only where AV1 is enabled, see decode; not verified |
| AV1 decode | yes, SW available | yes | **only on devices with an AV1 hardware decoder** (release note of 17.5) |
| HEVC encode | HW only, Windows and macOS; not on Linux | no (Nightly only) | yes |
| HEVC decode | HW only; Edge on Windows also needs the HEVC Video Extensions | no (Nightly only) | yes |
| Opus encode and decode | yes | yes | yes |
| AAC encode | Windows and macOS through the OS; not on Linux | **no** | yes |
| AAC decode | yes | yes | yes |

The round trip tests of web-platform-tests pass for H.264, VP8 and VP9 in Chrome 154, Edge 154, Firefox 157 and Safari 27.0 (runs of 2026-10-06). For Chromium on this machine, [the loopback record](2026-10-10-webcodecs-loopback.md) has what was measured, including that hardware encoding exists for H.264 and HEVC only.

**H.264, VP8 and VP9 profile 0 are the video codecs every desktop browser both encodes and decodes; Opus is the audio codec.** AV1 leaves out Safari on devices without a hardware decoder, HEVC leaves out Firefox and Chromium without hardware.

### The points of the open codec question

| Point | What the sources say |
|---|---|
| H.264 bitstream format | The default of the encoder is `avc` (length-prefixed, with a `description`); a decoder that gets no `description` takes the stream for Annex B (AVC registration). All three engines accept `annexb` (main; web-platform-tests pass both in all four browsers). **Firefox writes a malformed `description` in `avc` format on Linux and Windows** (Bugzilla 2049470 and 2031056, open), so a Firefox publisher is safe only with `annexb`. Safari does not accept `avc3` codec strings (main) |
| HEVC format | The same two forms. Firefox has no `hevc` option. An open WebKit report says `hevc.format: "annexb"` is ignored (281945), while its `main` reads the option and the Annex B test passes 4 of 5 |
| `decoderConfig` for a late joiner | The specification has it emitted with the first output and whenever the configuration changes, and it has to be enough to decode. That is all three engines' behaviour as read from `main`. The config reports the configured size, not the padded one, in all three (w3c/webcodecs 925). With Annex B, every key chunk carries the parameter sets, so a late joiner needs no `description` |
| `latencyMode` | In the IDL of all three (main). Compat tables have no entry for it, so no version is known |
| `optimizeForLatency` | In the IDL of all three. Safari's VideoToolbox decoders do not act on it yet (WebKit 325278, open); Firefox on Windows buffers despite it (Bugzilla 1953551, open). Decode latency for H.264 also depends on the reorder signalling in the bitstream (w3c/webcodecs 732, 900) |
| Asking for a key frame | `encode(frame, { keyFrame: true })` is a MUST of the specification and in all three IDLs; no difference was found documented |
| Temporal layers | `L1T2` and `L1T3` in all three; Firefox on Windows for H.264 only |
| Odd sizes | H.264 encoding needs an even width and height in Chrome and Safari; Firefox on Windows declines odd sizes |
| `isConfigSupported` | Best effort by the specification. Reports of "supported" followed by a failure: Safari with HEVC (WebKit 262950, reopened), Firefox with `prefer-hardware` H.264 (Bugzilla 1918769) |
| Colour | Firefox converts RGB input with BT.601 while the config says BT.709, and reports full-range H.264 as limited (Bugzilla 2056028, 2055639, open); Safari has open colour bugs |

### Choosing a video codec

The decision is the project owner's. This table puts the options side by side; "breadth" is who can publish and who can view, from the tables above.

| Option | Breadth | Latency | Work |
|---|---|---|---|
| H.264, Annex B | Widest: encode and decode in Chrome, Edge, Firefox desktop and Safari. Not Firefox for Android | Chromium, measured at 1280x720: 6.7 ms a frame in software, 8.5 ms with the hardware encoder. Its software decoder gave no output for the first chunks until `optimizeForLatency` was set. Elsewhere, decode delay depends on the platform decoder and on the reorder signalling in the bitstream (w3c/webcodecs 732, 900) | Annex B needs no `description`, so late join and a change of configuration need no extra message, and Firefox's `avc` bug is avoided. The level has to fit the size (loopback record: level 3.1 strings are refused at 1080p). Even sizes only |
| H.264, `avc` format | The same, minus Firefox publishers on Linux and Windows until its bug is fixed | The same | The `description` has to reach a late joiner |
| VP8 | Encode and decode in every desktop browser, software only | Chromium, measured: 4.8 ms a frame at 1280x720, the shortest of the four; decode 1.7 ms, the longest. Playwright's Firefox: 7 ms | Simplest bitstream: no `description`, no formats |
| VP9 profile 0 | Encode and decode in every desktop browser; software encode | Chromium, measured: 4.9 ms a frame; fewer bits than VP8 and H.264 at the same setting in that run. Playwright's Firefox: 9 ms | No `description`. An open WebKit report has its VP9 encoder dropping frames in constant bitrate (325913) |
| AV1 | Viewers on Safari only with an AV1 hardware decoder | Chromium, measured: 5.2 ms a frame. Its decoder held the first chunks like the H.264 one. **Playwright's Firefox: 47 ms a frame, too slow for 30 frames a second at this size** | Would need a second codec for the Safari devices left out |
| HEVC | No Firefox; Chromium only with hardware | Chromium with the hardware encoder, measured: 9.2 ms a frame; 112 ms with the software one of the platform | Leaves out too many viewers to be the only codec |

The times are the median of the serial encode at 1280x720 in browser A of [the loopback record](2026-10-10-webcodecs-loopback.md): one machine, one run, a generated picture. They show that no codec is out of reach for one 720p stream on that machine, and nothing about Firefox, Safari or a phone.

A recommendation, for the owner to take or leave: **H.264 in Annex B with Opus** reaches every desktop browser that has WebTransport, keeps late join free of a separate configuration message, and has hardware encoders and decoders on the platforms read about. VP8 and VP9 reach the same browsers in software and have the simpler bitstream. Between Chromium and Playwright's Firefox on this Mac, all three were encoded in under 10 ms a frame and decoded by the other browser. The choice between them is not settled by anything here: Safari rests on published sources alone, Firefox on one build on one machine, and no source read compares their latency across browsers.

### Runs in the Playwright builds of Firefox and WebKit

Chromium 156.0.8078.4 ran the same scripts beside Firefox, as the known case.

**WebKit: no result.** The build launched, and `browser.newPage()` did not return in 40 seconds, under Bun and under Node alike (`wk-min.ts`). Nothing about WebKit below is measured. Not tried: a window that is not headless, and another version of macOS.

**Firefox 157.0 (Playwright build):**

| Question | Result |
|---|---|
| Are the APIs there? | `WebTransport`, `WebTransportError`, `VideoEncoder`, `VideoDecoder`, `AudioEncoder`, `AudioDecoder` are functions in a page on `http://localhost` |
| Does it open a session to the own relay with `serverCertificateHashes` and `protocols: ["moqt-18"]`? | Yes: `ready` resolved, `protocol` was "moqt-18", the relay printed `SESSION_OPENED` and sent SETUP, and `close({ closeCode: 0, reason })` reached it as `ClosedByPeer { code: 0, reason: "probe done" }`. The failure of Bugzilla 2078582 did not happen in this build |
| Does `@xmoq/moqt` move an Object? | Yes, with the interop page as publisher and as subscriber through the own relay: PUBLISH, SUBSCRIBE, one Object, PUBLISH_DONE with Stream Count 1, and `done()` resolved with `fin`. The bytes the pages logged are the same as in Chromium |
| 10,000 Objects of 2000 bytes, one a stream (20 MB) | All arrived, in Firefox and in Chromium; the relay opened and finished 10,000 streams and reset none, and the subscription ended with PUBLISH_DONE and all Groups. Neither stopped at 7600 streams or 16 MB, the numbers of the WebKit report |
| `new WebTransportError("m", { streamErrorCode: 1 })`, the form of the specification | TypeError ("Argument 1 can't be converted to a dictionary"), as in Chromium |
| `new WebTransportError({ message: "m", streamErrorCode: 1 })` | an error with the message, `source` "stream" and code 1, as in Chromium |
| `streamErrorCode` of 256, 65536, 4294967295; of -1 | 255 each; 0. The same as Chromium: 8 bits |

What the relay tells such a client, read in the sources of the versions in `Cargo.lock`: the HTTP/3 settings of `web-transport-proto` 0.6.2 carry extended CONNECT, the HTTP/3 datagram setting in its RFC and its older form, and the WebTransport settings of draft 02 and of the later drafts (`src/settings.rs`); the relay keeps QUIC datagrams announced (`crates/moqt-io/src/wt.rs`); `quinn-proto` 0.11.19 has no transport parameter for reliable stream reset (`src/transport_parameters.rs`). So the relay does not offer the third thing neqo pull request 3756 is said to require, and this Firefox connected all the same. Whether this build contains that change, and whether Firefox 157.0.1 behaves in the same way, was not found out.

One difference at the end of a run: when the driver closed the Firefox pages without the application closing its sessions, the relay closed both sessions itself with `PROTOCOL_VIOLATION`, "the peer closed its control stream"; with Chromium it saw the sessions end (`ClosedByPeer { code: 0 }`). Firefox ended the streams before the session. A page that calls `close()` first was the case of `connect.ts` and ended cleanly.

`isConfigSupported` at 1280x720, as `no-preference` / `prefer-hardware` / `prefer-software`:

| Config | Chromium 156 encode | Chromium 156 decode | Firefox 157.0 encode | Firefox 157.0 decode |
|---|---|---|---|---|
| `avc1.42E01F`, `avc` and `annexb` | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc1.640033`, `avc` and `annexb` | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `vp8` | yes / no / yes | yes / no / yes | yes / no / yes | yes / yes / yes |
| `vp09.00.31.08` | yes / no / yes | yes / yes / yes | yes / no / yes | yes / yes / yes |
| `av01.0.05M.08` | yes / no / yes | yes / yes / yes | yes / no / yes | yes / yes / yes |
| `hvc1.1.6.L93.B0` | yes / yes / yes | yes / yes / no | no / no / no | no / no / no |
| `opus` | yes | yes | yes | yes |
| `mp4a.40.2` | yes | yes | no | yes |

This agrees with what was read: Firefox has no HEVC and no AAC encoder, and everything else of the table of codecs.

**The publisher's page closed without `close()`** (`moqt.ts --close-publisher`): the subscriber got PUBLISH_DONE `TRACK_ENDED` with Stream Count 1 and its one Group, with Firefox as with Chromium, and its session went on. What differs is at the relay only: with Chromium it printed `CONNECTION_ENDED session=0 ClosedByPeer { code: 0 }`, with Firefox `CLOSE_SESSION session=0 code=0x3 reason="protocol violation: the peer closed its control stream"`.

### Encoding and decoding in Chromium and in Firefox, and across the two

`codecs.ts`, with the values of [the loopback record](2026-10-10-webcodecs-loopback.md): 1280x720, 30 frames a second, 2,764,800 bit/s, `latencyMode: "realtime"`, a generated picture, 90 frames with `keyFrame: true` at frames 0, 30 and 60. One frame is submitted and its output awaited before the next. Hardware acceleration was left at `no-preference`, and which encoder each browser used was not read out. Run twice; the times are min / median / p95 / max in milliseconds of the first run, and the medians of the second are in parentheses. **Firefox reports time in whole milliseconds.**

| Codec | Chromium 156 encode | Firefox 157.0 encode | Key chunks at | `decoderConfig` | `description` |
|---|---|---|---|---|---|
| H.264 `avc1.42E01F`, `avc` | 7.9 / 9.0 / 11.0 / 285.8 (6.4) | 4 / 7 / 8 / 140 (6) | 0, 30, 60 in both | with the first output only, in both | Chromium 34 bytes, Firefox 35 bytes |
| H.264 `avc1.42E01F`, `annexb` | 5.9 / 6.7 / 8.8 / 30.4 (8.7) | 5 / 7 / 8 / 30 (6) | 0, 30, 60 | first output only | none, in both |
| VP8 | 4.0 / 4.8 / 5.1 / 9.4 (4.9) | 5 / 7 / 9 / 20 (8) | 0, 30, 60 | first output only | none |
| VP9 `vp09.00.31.08` | 4.4 / 5.2 / 6.3 / 19.3 (5.2) | 5 / 9 / 13 / 22 (10) | 0, 30, 60 | first output only | none |
| AV1 `av01.0.05M.08` | 4.2 / 5.1 / 6.0 / 11.8 (5.3) | **24 / 47 / 55 / 56 (47)** | 0, 30, 60 | first output only | none |

Every stream was then decoded in both browsers, configured with the `decoderConfig` the encoder gave (the codec string alone where there was no `description`), with `optimizeForLatency`, one chunk at a time:

| Encoded in | Decoded in | H.264 `avc` | H.264 `annexb` | VP8 | VP9 | AV1 |
|---|---|---|---|---|---|---|
| Chromium | Chromium | 90 of 90, median 1.2 | 90 of 90, 1.1 | 90 of 90, 2.2 | 90 of 90, 1.5 | 90 of 90, 1.0 |
| Chromium | Firefox | 90 of 90, 1 | 90 of 90, 1 | 90 of 90, 2 | 90 of 90, 1 | 90 of 90, 1 |
| Firefox | Chromium | 90 of 90, 2.0 | 90 of 90, 2.1 | 90 of 90, 1.8 | 90 of 90, 1.5 | 90 of 90, 0.9 |
| Firefox | Firefox | 90 of 90, 1 | 90 of 90, 1 | 90 of 90, 1 | 90 of 90, 1 | 90 of 90, 1 |

All 20 combinations gave 90 frames of 1280x720 and no error, in both runs.

- **Every one of the five streams made in one browser decodes in the other, on this Mac.** That includes H.264 in `avc` format from Firefox: its `description` is `01 42 00 1f ff e1 00 14 27 42 00 1f …`, with one NAL header byte (`27`) after the length, and Chromium decoded the stream with it. The malformed `description` of Bugzilla 2049470 is reported for Linux and Windows; **on macOS it was not seen**.
- **A key frame came where it was asked for, in both browsers and all five codecs,** and nowhere else in these 90 frames.
- **`decoderConfig` came once, with the first output**, in both. A viewer that joins later does not get it from the encoder again; with Annex B, VP8, VP9 and AV1 there was no `description` to miss.
- **Firefox encodes AV1 at 47 ms a frame here, which is more than the 33 ms of a frame at 30 a second.** Its other four encoders, and all five of Chromium, took 5 to 10 ms.
- Chromium's medians are those of the loopback record within its spread (there: H.264 6.7 software and 8.5 hardware, VP8 4.8, VP9 4.9, AV1 5.2), so the smaller script measures the same thing.

An answer of "yes" from `isConfigSupported` was followed by a working encoder and decoder in each of these cases.

## Limitations and blockers

- **The Firefox that was run is Playwright's build of 157.0, not the shipped 157.0.1,** on one Mac, over loopback, headless. A result here is not a result for the stable Firefox, for Linux or Windows, or for Firefox for Android.
- **WebKit was not measured at all** (above), so the two points that matter most for Safari stay open: whether it connects to the own relay, and whether it stops after 7600 streams. A window that is not headless was not tried, as it would open on the user's screen. It needs a machine with Safari 26.4 or later.
- The run of 10,000 streams took 10 seconds in Firefox and 68 in Chromium. Most of that is the driver, which sends one Object per call into the page and takes the page's log lines; it is not a measure of either browser.
- Frames were encoded and decoded in Chromium and in Playwright's Firefox only, on one Mac, with a generated picture and 90 frames, twice. The picture is not a camera's, and no audio was run. Which encoder Firefox used for each codec, hardware or software, was not read out.

- **In the tables read from sources, nothing was run.** Every row for Firefox and Safari there is what a source says, and many rest on the engines' `main`, which is ahead of the shipped builds (Chrome 155, Firefox 157, Safari 27.0). Release branches were not read.
- **iOS and iPadOS rest on release notes and WebKit source that cover all Apple platforms together.** No test run on iOS was found. All browsers on iOS use WebKit; no primary source was found on a change of that in the EU.
- **Chrome and Edge on Android**: no source for decode per codec or for AV1 encode was found.
- **Hardware or software** for Chrome's H.264 decode and for Safari's H.264 and HEVC was not sourced.
- The versions in which `latencyMode`, `scalabilityMode`, `hardwareAcceleration` and `optimizeForLatency` shipped are not known: the compat tables have no entries for them.
- The Chromium issue tracker could not be searched without a browser, so Chromium bugs appear only where another source names them.
- Compat tables and release notes disagree in places; the two source files end with a list of the disagreements. The ones that touch a table above are named in it.
- The bug reports were read, not reproduced. Two are unconfirmed or contested by their trackers (Bugzilla 2078582, WebKit 281945).
- The pass counts of web-platform-tests come from machines that may lack hardware codecs: a failure there can be the machine and not the browser.
- `spikes/browser-support/` holds the two source tables as the research passes wrote them, and the scripts of the runs. They are outside lint and review.

## Open questions raised

- Does the shipped Firefox 157.0.1 connect to the own relay? Playwright's 157.0 did, although `quinn-proto` 0.11.19 announces no reliable stream reset.
- Should a browser that is closed without `close()` end its sessions cleanly at the relay? With Firefox the relay closed them as a protocol violation.
- Does Safari 27 run a MoQ session of one Object a stream past 7600 streams? If the open flow control report holds, the packaging of Objects into streams matters for Safari more than the codec.
- Which browsers does Phase 1 have to support? Firefox for Android has no WebCodecs, and iOS was not measured by anyone found.
- If H.264: which profile and level, and is a publisher without hardware H.264 (Chromium on Linux encodes in software) in scope?
- Should the interop matrix get a Firefox page? Playwright's Firefox ran the interop page as it is. Its WebKit build for macOS 14 did not open a page, so a WebKit page needs another machine or another driver.
