# WebCodecs per browser, as of 2026-10-10

Everything below was fetched on 2026-10-10 in this task. Every row's "Read" column is that date. The "Source" column holds a key into the source list at the end of this section, which gives the full URL. Quotes are verbatim; `[...]` marks an omission.

Three kinds of source are used, and they do not carry the same weight:

- **Docs**: specs, release notes, vendor blogs, MDN, chromestatus.
- **Compat tables**: MDN browser-compat-data (BCD) and caniuse. A version number from these is marked "(compat table)".
- **Engine source at `main`**: Chromium `79e6a209`, WebKit `bf102c0e`, Firefox `6411c7c9` (all committed 2026-10-10). This is the tip of development, not the shipped stable build. Anything taken only from `main` is marked "(source, main)". I did not run any browser.

WPT results are from wpt.fyi aligned stable runs of 2026-10-06 (Chrome 154 Linux, Edge 154 Windows, Firefox 157 Linux, Safari 27.0 macOS). A `0/N` there can mean "the CI machine lacks the hardware", not "the browser lacks the feature".

## Sources

| Key | URL |
|---|---|
| BCD-VE | https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/VideoEncoder.json |
| BCD-VD | https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/VideoDecoder.json |
| BCD-AE | https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/AudioEncoder.json |
| BCD-AD | https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/AudioDecoder.json |
| BCD-EVC | https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/EncodedVideoChunk.json |
| BCD-VF | https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/VideoFrame.json |
| BCD-BR | https://raw.githubusercontent.com/mdn/browser-compat-data/main/browsers/{chrome,chrome_android,edge,firefox,firefox_android,safari,safari_ios}.json |
| CANIUSE | https://raw.githubusercontent.com/Fyrd/caniuse/main/features-json/webcodecs.json |
| SPEC | https://www.w3.org/TR/webcodecs/ (W3C Working Draft, 7 October 2026) |
| REG | https://www.w3.org/TR/webcodecs-codec-registry/ (Registry Draft, 24 September 2026) |
| REG-AVC | https://www.w3.org/TR/webcodecs-avc-codec-registration/ (Group Note Draft, 8 June 2026) |
| REG-HEVC | https://www.w3.org/TR/webcodecs-hevc-codec-registration/ (Group Note Draft, 8 June 2026) |
| REG-VP8 / REG-VP9 / REG-AV1 | https://www.w3.org/TR/webcodecs-vp8-codec-registration/ , .../webcodecs-vp9-codec-registration/ , .../webcodecs-av1-codec-registration/ |
| REG-OPUS / REG-AAC | https://www.w3.org/TR/webcodecs-opus-codec-registration/ , https://www.w3.org/TR/webcodecs-aac-codec-registration/ |
| MDN-API | https://raw.githubusercontent.com/mdn/content/main/files/en-us/web/api/webcodecs_api/index.md |
| MDN-VECONF | https://raw.githubusercontent.com/mdn/content/main/files/en-us/web/api/videoencoder/configure/index.md |
| MDN-VEENC | https://raw.githubusercontent.com/mdn/content/main/files/en-us/web/api/videoencoder/encode/index.md |
| MDN-VCODECS | https://raw.githubusercontent.com/mdn/content/main/files/en-us/web/media/guides/formats/video_codecs/index.md |
| MDN-FF130 | https://raw.githubusercontent.com/mdn/content/main/files/en-us/mozilla/firefox/releases/130/index.md |
| MDN-FF137 | https://raw.githubusercontent.com/mdn/content/main/files/en-us/mozilla/firefox/releases/137/index.md |
| CS-WC | https://chromestatus.com/feature/5669293909868544 (WebCodecs) |
| CS-HEVCDEC | https://chromestatus.com/feature/5186511939567616 (Enable HEVC hardware decoding) |
| CS-HEVCRTC | https://chromestatus.com/feature/5153479456456704 (H265 in WebRTC) |
| CS-QP | https://chromestatus.com/feature/5783986600673280 (Per-frame quantizer) |
| CS-HINT | https://chromestatus.com/feature/5103493249761280 (VideoEncoderConfig.contentHint) |
| CS-AV1ENC | https://chromestatus.com/feature/6206321818861568 (AV1 Encoder) |
| CS-VIOL | https://chromestatus.com/feature/5667793157488640 (Deprecating minor WebCodecs spec violations) |
| BLINKDEV | https://groups.google.com/a/chromium.org/g/blink-dev/c/YJ1QijNiHeM/m/3EhMjMVwAgAJ |
| CHROME-ART | https://developer.chrome.com/docs/web-platform/best-practices/webcodecs ("Last updated 2025-01-22 UTC") |
| CHROMEDASH | https://chromiumdash.appspot.com/fetch_releases?channel={Stable,Beta,Dev,Canary}&platform={Windows,Android}&num=1 |
| EDGEUPD | https://edgeupdates.microsoft.com/api/products |
| MOZ-PD | https://product-details.mozilla.org/1.0/firefox_versions.json and .../mobile_versions.json |
| MOZ-130 | https://www.mozilla.org/en-US/firefox/130.0/releasenotes/ |
| MOZ-157 | https://www.mozilla.org/en-US/firefox/157.0/releasenotes/ and https://www.mozilla.org/en-US/firefox/android/157.0/releasenotes/ |
| CR-VE | https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/modules/webcodecs/video_encoder.cc |
| CR-VEIDL | .../third_party/blink/renderer/modules/webcodecs/video_encoder_config.idl and video_encoder_encode_options.idl |
| CR-VD | .../third_party/blink/renderer/modules/webcodecs/video_decoder.cc and video_decoder_config.idl |
| CR-AE | .../third_party/blink/renderer/modules/webcodecs/audio_encoder.cc and https://raw.githubusercontent.com/chromium/chromium/main/media/mojo/clients/mojo_audio_encoder.cc |
| CR-SW | https://raw.githubusercontent.com/chromium/chromium/main/media/base/media_switches.cc |
| CR-GNI | https://raw.githubusercontent.com/chromium/chromium/main/media/media_options.gni |
| FF-PREF | https://raw.githubusercontent.com/mozilla-firefox/firefox/main/modules/libpref/init/StaticPrefList.yaml |
| FF-UTIL | https://raw.githubusercontent.com/mozilla-firefox/firefox/main/dom/media/webcodecs/WebCodecsUtils.cpp |
| FF-VE | .../dom/media/webcodecs/VideoEncoder.cpp and .../dom/webidl/VideoEncoder.webidl |
| FF-VD | .../dom/media/webcodecs/VideoDecoder.cpp and .../dom/webidl/VideoDecoder.webidl |
| FF-AE / FF-AD | .../dom/media/webcodecs/AudioEncoder.cpp , AudioDecoder.cpp |
| FF-PEM | .../dom/media/platforms/PEMFactory.cpp, ffmpeg/FFmpegEncoderModule.cpp, wmf/WMFEncoderModule.cpp, agnostic/gmp/GMPEncoderModule.cpp |
| BMO-n | https://bugzilla.mozilla.org/show_bug.cgi?id=n (read through the REST API, `/rest/bug/n` and `/rest/bug/n/comment`) |
| WK-VE | https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebCore/Modules/webcodecs/WebCodecsVideoEncoder.cpp, WebCodecsVideoEncoder.idl, WebCodecsVideoEncoderConfig.idl, WebCodecsVideoEncoderEncodeOptions.idl, BitrateMode.idl |
| WK-VD | .../Source/WebCore/Modules/webcodecs/WebCodecsVideoDecoder.cpp, WebCodecsVideoDecoderConfig.idl |
| WK-PLAT | .../Source/WebCore/platform/VideoEncoder.cpp, VideoDecoder.cpp; .../Source/WebKit/WebProcess/GPU/media/RemoteVideoCodecFactory.cpp |
| WK-AUDIO | .../Source/WebCore/Modules/webcodecs/WebCodecsAudioEncoder.cpp; .../Source/WebCore/platform/audio/cocoa/AudioDecoderCocoa.cpp |
| WK-PREF | https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WTF/Scripts/Preferences/UnifiedWebPreferences.yaml |
| WKB-n | https://bugs.webkit.org/show_bug.cgi?id=n (read through `/rest/bug/n/comment`) |
| SRN-x | https://developer.apple.com/documentation/safari-release-notes/safari-x-release-notes (read as JSON from `https://developer.apple.com/tutorials/data/documentation/safari-release-notes/safari-x-release-notes.json`) |
| STP-n | https://developer.apple.com/documentation/safari-technology-preview-release-notes/stp-release-n (same JSON route) |
| WKBLOG-16.4 | https://webkit.org/blog/13966/webkit-features-in-safari-16-4/ |
| WKBLOG-17.0 | https://webkit.org/blog/14445/webkit-features-in-safari-17-0/ |
| WKBLOG-26.0 | https://webkit.org/blog/17333/webkit-features-in-safari-26-0/ |
| WPT | https://wpt.fyi/results/webcodecs?label=master&label=stable&aligned (data from `POST https://wpt.fyi/api/search`, run ids 6218298311311360, 5139408432136192, 4877098740350976, 5164107446878208) |
| GH-n | https://github.com/w3c/webcodecs/issues/n |

## 0. Current versions and channels

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Chrome desktop / Android | BCD: 155 current, 156 beta, 157 nightly | BCD-BR | 2026-10-10 | `chrome 155 current 2026-10-06`, `156 beta 2026-10-20`, `157 nightly 2026-11-03` (same for `chrome_android`) |
| Chrome desktop / Android | chromiumdash: Stable already 156 | CHROMEDASH | 2026-10-10 | `Stable Windows 156.0.8078.12` (2026-10-07), `Stable Android 156.0.8078.25`, `Beta Windows 156.0.8078.17`, `Dev 157.0.8092.0`, `Canary 157.0.8096.0` |
| Edge | Stable 155 on Windows, 154 on Android | EDGEUPD | 2026-10-10 | `Stable Windows 155.0.4283.45` (2026-10-10), `Stable Android 154.0.4258.38`, `Beta Windows 156.0.4314.15`, `Canary Windows 157.0.4329.0` |
| Firefox desktop | Stable 157.0.1, Beta 158.0b5, Nightly 160.0a1 | MOZ-PD | 2026-10-10 | `"LATEST_FIREFOX_VERSION": "157.0.1"`, `"LATEST_FIREFOX_DEVEL_VERSION": "158.0b5"`, `"FIREFOX_NIGHTLY": "160.0a1"` |
| Firefox Android | Stable 157.0.1, Beta 158.0b5, Nightly 160.0a1 | MOZ-PD | 2026-10-10 | `"version": "157.0.1"`, `"beta_version": "158.0b5"`, `"nightly_version": "160.0a1"` |
| Safari macOS / iOS / iPadOS | Stable 27.0; beta 27.2 | SRN-27, SRN-27_2 | 2026-10-10 | "Released September 14, 2026 — 27.0 (20625.1.29)"; "Safari 27.2 beta is available for iOS 27.2 beta, iPadOS 27.2 beta, visionOS 27.2 beta, macOS 27.2 beta, macOS 26, and macOS Sequoia." |
| Safari Technology Preview | Latest listed release is 253 | STP index (https://developer.apple.com/documentation/safari-technology-preview-release-notes) | 2026-10-10 | index lists `stp-release-253` as the highest number |

## 1. Interfaces: stable support, versions, Workers

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Chrome / Edge, desktop + Android | `VideoEncoder`, `VideoDecoder`, `AudioEncoder`, `AudioDecoder`, `EncodedVideoChunk`, `VideoFrame` since 94 (compat table; chromestatus agrees) | BCD-VE, BCD-VD, BCD-AE, BCD-AD, BCD-EVC, BCD-VF; CS-WC | 2026-10-10 | BCD: `chrome: 94`, `chrome_android: mirror`, `edge: mirror`. chromestatus: `"status": {"text": "Enabled by default", "milestone_str": "94"}`, `"desktop": 94, "android": 94, "webview": 94` |
| Firefox desktop | All six since 130 (compat table and release note) | BCD-*; MOZ-130; MDN-FF130 | 2026-10-10 | BCD: `firefox: 130`. Release note: "Enabled the Web Codecs API on desktop platforms, allowing low-level access to audio and video encoders and decoders." |
| Firefox Android | Not in release or beta. Pref is on in Nightly builds only. "Ship Web Codecs on Android" is still open | FF-PREF; BMO-1934008; MDN-FF130; BCD-VE | 2026-10-10 | Pref: `- name: dom.media.webcodecs.enabled` / `#if defined(MOZ_WIDGET_ANDROID)` / `value: @IS_NIGHTLY_BUILD@` / `#else` / `value: true`. Bug 1934008: status `NEW`, summary "Ship Web Codecs on Android". MDN: "Android support is enabled in the Nightly release." BCD: `firefox_android: False` for VideoEncoder, VideoDecoder, AudioEncoder, AudioDecoder, EncodedVideoChunk |
| Firefox Android, Nightly | Even with the pref on, support checks return false on Android (source, main) | FF-VE, FF-VD, FF-AE | 2026-10-10 | `// TODO: Enable WebCodecs on Android (Bug 1840508)` / `if (IsOnAndroid()) {` / `return false;` (in `CanEncode` for video, `CanDecode` for video, `CanEncode` for audio). Bug 1840508 is `RESOLVED DUPLICATE` of 1934008 |
| Safari macOS / iOS / iPadOS | Video interfaces (`VideoEncoder`, `VideoDecoder`, `EncodedVideoChunk`, `VideoFrame`) since 16.4 | SRN-16_4; WKBLOG-16.4; BCD-VE | 2026-10-10 | Release note: "Added video-only support for Web Codecs." Blog: "Safari 16.4 adds support for the video portion of Web Codecs API." BCD: `safari: 16.4`, `safari_ios: mirror` |
| Safari macOS / iOS / iPadOS | `AudioEncoder`, `AudioDecoder` since 26.0 | SRN-26; WKBLOG-26.0; BCD-AE | 2026-10-10 | Release note: "Added support for WebCodec's AudioEncoder and AudioDecoder. (142916087)". Blog: "Safari 26.0 expands support for WebCodecs API by adding AudioEncoder and AudioDecoder." BCD: `safari: 26` |
| Safari, Lockdown Mode | WebCodecs is disabled in Lockdown Mode | SRN-16_5; WK-PREF | 2026-10-10 | 16.5 notes, section "Lockdown Mode / New Restrictions": "Disabled WebCodecs API". Pref `WebCodecsVideoEnabled`: `disableInLockdownMode: true` |
| Safari Technology Preview | STP 251 adds `ImageDecoder`. No new encoder/decoder codec in STP 246-253 | STP-251 | 2026-10-10 | "Added support for the WebCodecs `ImageDecoder` API. (184857272)" |
| Safari 27.2 beta / iOS 27.2 beta | Release notes contain no WebCodecs item | SRN-27_2 | 2026-10-10 | (no line matches WebCodecs, VideoEncoder, VideoDecoder, AV1, VP9, HEVC, H264; 46 paragraphs read) |
| Chrome Beta 156 / Canary 157 | No per-channel difference found in the sources read. Chromium `main` has `referenceBuffers` / `updateBuffer` encode options that are not in the spec; chromestatus lists "Manual reference frame control in VideoEncoder" as "Proposed" | CR-VEIDL; chromestatus API (`https://chromestatus.com/api/v0/features?q=webcodecs`) | 2026-10-10 | IDL: `sequence<VideoEncoderBuffer> referenceBuffers;` `VideoEncoderBuffer updateBuffer;`. chromestatus: `Manual reference frame control in VideoEncoder | Proposed` |
| All (spec) | Exposed in Window and DedicatedWorker, secure context | SPEC; MDN-API | 2026-10-10 | Spec IDL: `[Exposed=(Window,DedicatedWorker), SecureContext]`. MDN: `{{AvailableInWorkers("window_and_dedicated")}}` |
| Firefox | Same exposure (source, main) | FF-VE | 2026-10-10 | `[Exposed=(Window,DedicatedWorker), SecureContext, Func="nsRFPService::ExposeWebCodecsAPI"]` |
| Safari | Same exposure (source, main) | WK-VE | 2026-10-10 | `EnabledBySetting=WebCodecsVideoEnabled,` `Exposed=(Window,DedicatedWorker),` |
| Chrome, Edge, Firefox, Safari | Worker variants of the WPT suite run in all four | WPT | 2026-10-10 | Subtests passed / total over the 121 `.worker` variants: Chrome 154 `1220/1333`, Edge 154 `1220/1333`, Firefox 157 `1090/1335`, Safari 27.0 `719/1191` |
| BCD | BCD has no worker sub-entries for these interfaces | BCD-VE | 2026-10-10 | (no key containing "worker" in VideoEncoder.json, VideoDecoder.json, AudioEncoder.json, VideoFrame.json) |

## 2. Video codecs

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Chrome / Edge | Software encoders are bundled for AV1, VP8, VP9 and H.264 (OpenH264). HEVC has no software encoder (source, main) | CR-VE | 2026-10-10 | `case media::VideoCodec::kAV1: result = CreateAv1VideoEncoder();` / `case media::VideoCodec::kVP8: case media::VideoCodec::kVP9: result = CreateVpxVideoEncoder();` / `case media::VideoCodec::kH264: result = CreateOpenH264VideoEncoder();` / `default: break;` |
| Chrome Android (32-bit ARM), iOS | OpenH264 is not built in, so H.264 encoding depends on the platform encoder there (source, main) | CR-GNI | 2026-10-10 | `media_use_openh264 = true` / `if (is_ios \|\| (is_android && current_cpu == "arm") \|\| !proprietary_codecs) {` / `media_use_openh264 = false` |
| Chrome / Edge | HEVC decode: hardware only, since 107 | CS-HEVCDEC; CS-HEVCRTC | 2026-10-10 | "Enables support for decoding HEVC video on platforms where hardware (e.g., GPU, media accelerator, etc) for decoding HEVC is available (Android 5.0+, macOS 11+, with supported hardware on Windows 8+ and ChromeOS)." `"milestone_str": "107"`. Also: "if it is available in hardware (we will not provide a software implementation). The codec is already available in WebCodecs (M130)" |
| Chrome / Edge | HEVC encode in WebCodecs since M130, platform encoders only, Windows / macOS / Android | BLINKDEV; CR-SW | 2026-10-10 | Markus Handell, 2025-02-11: "Support for HEVC platform encoding was added in WebCodecs in Chromium M130." "HEVC encoding is only supported if the user's device and operating system provide the necessary capabilities." "The support is currently limited to Windows, macOS and Android hosts." Source: `// Enables HEVC hardware accelerated encoding for Windows, Apple, and Android.` `BASE_FEATURE(kPlatformHEVCEncoderSupport, base::FEATURE_ENABLED_BY_DEFAULT);` |
| Chrome | HEVC encode profiles accepted: Main, Main10, RExt (source, main); high-bit-depth encode is off by default | CR-VE; CR-SW | 2026-10-10 | `*js_error_message = "Unsupported hevc profile.";` for anything other than `HEVCPROFILE_MAIN`, `HEVCPROFILE_MAIN10`, `HEVCPROFILE_REXT`. `BASE_FEATURE(kPlatformHEVCHbdEncoderSupport, base::FEATURE_DISABLED_BY_DEFAULT);` |
| Edge on Windows | HEVC needs hardware support and the HEVC Video Extensions (MDN statement about Edge in general; I found no Microsoft page that says this for WebCodecs specifically) | MDN-VCODECS | 2026-10-10 | "Edge (Chromium) supports HEVC for devices with hardware support on Windows 10 1709+ when HEVC video extensions from the Microsoft Store is installed, and has the same support status as Chrome on other platforms." |
| Chrome | AV1 software decoder (dav1d) and libvpx are built in (source, main) | CR-GNI | 2026-10-10 | `enable_dav1d_decoder = use_blink`; `# Enable usage of libvpx within the media library. Used for software based decoding of VP9 and VP8A type content.` |
| Chrome 154 Linux / Edge 154 Windows | WPT encode-then-decode passes for H.264 (avc and annexb), VP8, VP9 profile 0 and 2, AV1. HEVC fails on these CI machines | WPT | 2026-10-10 | `full-cycle-test.https.any.html?h264_avc 5/5`, `?h264_annexb 5/5`, `?vp8 4/4`, `?vp9_p0 4/4`, `?vp9_p2 4/4`, `?av1 4/4`, `?h265_hevc 0/4`, `?h265_annexb 0/4` (identical for Chrome and Edge) |
| Firefox desktop | Codec strings accepted: `vp8`, `vp09.*`, `av01.*`, `avc1.*`, `avc3.*`. HEVC only behind a Nightly-only pref, and only on macOS or Linux (source, main) | FF-UTIL; FF-PREF | 2026-10-10 | `// H265 is supported only on MacOS in Nightly for now.` / `return StaticPrefs::dom_media_webcodecs_h265_enabled() && StaticPrefs::media_hevc_enabled() && (IsOnMacOS() \|\| IsOnLinux());`. Pref: `- name: dom.media.webcodecs.h265.enabled` / `value: @IS_NIGHTLY_BUILD@`. Bug 1942421 "Enable H265 Decoder on Windows for WebCodecs" is `NEW` |
| Firefox desktop | H.264: "dependent upon the operating system's built-in or preinstalled codecs" (MDN, general statement) | MDN-VCODECS | 2026-10-10 | "Firefox support for AVC is dependent upon the operating system's built-in or preinstalled codecs for AVC and its container in order to avoid patent concerns." |
| Firefox desktop | H.264 encode goes through platform encoder modules (Apple, WMF, FFmpeg). The OpenH264 (GMP) encoder module is off by default (source, main) | FF-PEM; FF-PREF | 2026-10-10 | `mCurrentPEMs.AppendElement(new WMFEncoderModule());`, `new AppleEncoderModule()`; `if (StaticPrefs::media_gmp_encoder_enabled()) {` / `auto pem = MakeRefPtr<GMPEncoderModule>();`. Pref: `- name: media.gmp.encoder.enabled` / `value: false`. Bug 2049470: "that avcC is exactly the WebCodecs `VideoEncoder` `decoderConfig.description` produced on Linux (FFmpeg) and Windows (WMF)" |
| Firefox desktop | VP8, VP9, AV1 encode through bundled libvpx and libaom (source, main) | FF-PEM | 2026-10-10 | `// We only support L1T2 and L1T3 ScalabilityMode in VPX and AV1 encoders via libvpx and libaom for now.` |
| Firefox 157 Linux | WPT encode-then-decode passes for H.264, VP8, VP9 profile 0, AV1. Fails for VP9 profile 2, 4:4:4, HEVC | WPT | 2026-10-10 | `?h264_avc 5/5`, `?h264_annexb 5/5`, `?vp8 4/4`, `?vp9_p0 4/4`, `?av1 4/4`, `?vp9_p2 0/5`, `?av1_444_high 0/4`, `?h265_hevc 0/4` |
| Firefox | HEVC playback (not WebCodecs): hardware and software on Windows / macOS / Linux, hardware only on Android | MDN-FF137; MDN-VCODECS | 2026-10-10 | "HEVC (H.265) is now hardware enabled on Android, and hardware and software enabled on Linux. This adds to existing hardware and software support on Windows and macOS." |
| Safari | Codec strings accepted for encode and decode: `vp8`, `avc1.`, `vp09.`, `hev1.` / `hvc1.` (setting), `av01.0` (setting). `avc3.` is not in the list (source, main) | WK-VE; WK-VD | 2026-10-10 | `return config.codec.startsWith("vp8"_s) \|\| config.codec.startsWith("avc1."_s) \|\| (config.codec.startsWith("vp09."_s) && isSupportedVP9EncoderCodec(config.codec, settings)) \|\| (config.codec.startsWith("hev1."_s) && settings.webCodecsHEVCEnabled) \|\| (config.codec.startsWith("hvc1."_s) && settings.webCodecsHEVCEnabled) \|\| (config.codec.startsWith("av01.0"_s) && settings.webCodecsAV1Enabled);` (the decoder check has the same shape) |
| Safari | HEVC in WebCodecs since 17.4 | SRN-17_4; WK-PREF | 2026-10-10 | "Added WebCodecs HEVC support. (112067287)". Pref `WebCodecsHEVCEnabled`: `"PLATFORM(COCOA)": true` |
| Safari | AV1 in WebCodecs since 17.5, only where an AV1 hardware decoder exists | SRN-17_5; WK-PREF | 2026-10-10 | "Added support for WebCodecs AV1 when an AV1 hardware decoder is available. (123433815)". Pref `WebCodecsAV1Enabled`: `status: preview`, `"USE(GSTREAMER)": true` / `default: false` |
| Safari | Which devices have AV1 hardware decode | WKBLOG-17.0; MDN-VCODECS | 2026-10-10 | Blog: "Safari 17.0 adds support for AV1 video on devices with hardware decoding support, like iPhone 15 Pro and iPhone 15 Pro Max." MDN: "support in Safari is limited to devices that feature a hardware decoder, meaning M3 MacBooks and later, iPhone 15 Pro, and iPhone 16 and later." |
| Safari | AV1 encode: a libaom-based encoder path exists behind the same `webCodecsAV1Enabled` gate, 8-bit 4:2:0 only (source, main). Not confirmed on a device | WK-PLAT | 2026-10-10 | `callback(makeUnexpected("AV1 encoding only supports 8-bit 4:2:0"_s));` / `LibWebRTCVPXVideoEncoder::create(LibWebRTCVPXVideoEncoder::Type::AV1, config, [...]` |
| Safari | VP8 / VP9: software (libvpx) encode and decode; VP9 also has a hardware decoder | WK-PLAT; SRN-17; SRN-17_5 | 2026-10-10 | `LibWebRTCVPXVideoEncoder::create(LibWebRTCVPXVideoEncoder::Type::VP8, [...]`, `Type::VP9`, `Type::VP9_P2`. 17.0: "Added support for WebCodecs temporal `scalabilityMode` for software codecs". 17.5: "Fixed WebCodecs to correctly use the VP9 hardware decoder. (123475343)" |
| Safari iOS / iPadOS | VP8 / VP9 and WebM arrived on iOS in 17.4 (general media, not stated for WebCodecs) | SRN-17_4 | 2026-10-10 | "Added support for VP8/VP9 and WebM on iOS and iPadOS. (64825245)" |
| Safari | H.264 uses hardware encoders (at least on some devices) | SRN-17_2; WPT | 2026-10-10 | "Fixed WebCodecs hardware encoders losing a frame. (115252749)". WPT Safari 27.0: `video-encoder-h26x-annexb.https.any.html?h264_annexb_hardware 1/1`, `?h264_annexb_software 1/1` |
| Safari 27.0 macOS | WPT encode-then-decode passes for H.264, HEVC, VP8, VP9 profile 0. AV1 fails on this CI machine | WPT | 2026-10-10 | `?h264_avc 5/5`, `?h264_annexb 5/5`, `?h265_hevc 5/5`, `?h265_annexb 4/5`, `?vp8 4/4`, `?vp9_p0 4/4`, `?vp9_p2 0/5`, `?av1 0/4`; `videoDecoder-codec-specific.https.any.html?av1 0/22` |
| All (spec) | The registry does not make any codec mandatory | REG | 2026-10-10 | "This registry maintains a mapping between codec strings and registration specifications as described below." (no support requirement in the text read) |

## 3. Audio: Opus and AAC

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Chrome / Edge | Opus encode: bundled software encoder. AAC encode: platform encoder only (source, main) | CR-AE | 2026-10-10 | `if (codec != media::AudioCodec::kOpus) return nullptr; auto software_encoder = std::make_unique<media::AudioOpusEncoder>();` and `if (codec != media::AudioCodec::kAAC) return nullptr;` in `CreatePlatformAudioEncoder` |
| Chrome / Edge | AAC encode is available on Windows (not N SKUs), macOS, Android; off by default elsewhere, which includes Linux (source, main) | CR-SW; CR-AE | 2026-10-10 | `// Allows usage of OS-level (platform) audio encoders.` `BASE_FEATURE(kPlatformAudioEncoder,` `#if BUILDFLAG(IS_WIN) \|\| BUILDFLAG(IS_MAC) \|\| BUILDFLAG(IS_ANDROID)` `base::FEATURE_ENABLED_BY_DEFAULT` `#else` `base::FEATURE_DISABLED_BY_DEFAULT`. Also: `// Windows AAC encoder relies on the MediaFoundation, which is not installed for Windows N Sku.` |
| Chrome / Edge | AAC encode constraints (source, main) | CR-AE | 2026-10-10 | `"Unsupported number of channels.", {1, 2, 6},` and `"Unsupported sample rate.", {44100, 48000},` |
| Chrome 154 / Edge 154 | Opus and AAC decode pass WPT (one subtest fails in each) | WPT | 2026-10-10 | `audioDecoder-codec-specific.https.any.html?opus 9/10`, `?mp4_aac 9/10`, `?adts_aac 9/10` |
| Firefox desktop | Encode: Opus and Vorbis only. No AAC encode (source, main) | FF-AE | 2026-10-10 | `static bool IsAudioEncodeSupported(const nsAString& aCodec) {` [...] `return aCodec.EqualsLiteral("opus") \|\| aCodec.EqualsLiteral("vorbis");` |
| Firefox desktop | Decode: Opus and AAC both accepted (source, main); WPT passes | FF-UTIL; WPT | 2026-10-10 | `return aCodec.EqualsLiteral("flac") \|\| aCodec.EqualsLiteral("mp3") \|\| IsAACCodecString(aCodec) \|\| aCodec.EqualsLiteral("vorbis") \|\| aCodec.EqualsLiteral("opus") [...]`. WPT Firefox 157: `?opus 9/10`, `?mp4_aac 10/10`, `?adts_aac 10/10` |
| Firefox Android | AAC decode explicitly refused (source, main) | FF-AD | 2026-10-10 | `if (IsOnAndroid() && IsAACCodecString(aConfig.mCodec)) {` / `return false;` |
| Safari 26+ | Encode list includes AAC (`mp4a.40.2` and others) and Opus (source, main) | WK-AUDIO | 2026-10-10 | `bool isMPEG4AAC = codec == "mp4a.40.2"_s \|\| codec == "mp4a.40.02"_s \|\| codec == "mp4a.40.5"_s \|\| codec == "mp4a.40.05"_s \|\| codec == "mp4a.40.29"_s \|\| codec == "mp4a.40.42"_s;` / `bool isCodecAllowed = isMPEG4AAC \|\| codec == "mp3"_s \|\| codec == "opus"_s [...]` |
| Safari 27.0 | Opus and AAC decode pass WPT | WPT | 2026-10-10 | `?opus 9/10`, `?mp4_aac 10/10`, `?adts_aac 10/10` |
| Safari | Opus encode bitrate must be 6000-510000 (source, main); Firefox has the same check | WK-AUDIO; FF-AE | 2026-10-10 | WebKit: `if (config.codec == "opus"_s && config.bitrate && (*config.bitrate < 6000 \|\| *config.bitrate > 510000))`. Firefox: `"Invalid config: bitrate value outside of [6k, 510k] for opus"` |
| All | `opus` encoder config: Chrome 110, Firefox 130, Safari 26 (compat table). `application` and `signal`: Chrome 126 only | BCD-AE | 2026-10-10 | `configure.opus: chrome 110, firefox 130, safari 26`; `configure.opus.opus_application: chrome 126, firefox False, safari False`; `opus_signal` same |
| All (spec) | Opus: chunk format follows `description` (absent means raw Opus packets). Default frame duration 20 ms | REG-OPUS | 2026-10-10 | "If a description has not been set, the bitstream is assumed to be in opus format." `OpusBitstreamFormat format = "opus";` `[EnforceRange] unsigned long long frameDuration = 20000;` |
| All (spec) | AAC: default encoder output is raw AAC (`aac`), not ADTS. Decoder without `description` assumes ADTS | REG-AAC | 2026-10-10 | `AacBitstreamFormat format = "aac";` "If description is present, it is assumed to be an AudioSpecificConfig [...] and the bitstream is assumed to be in aac format. If the description is not present, the bitstream is assumed to be in adts format." |
| WPT, all four | `audio-encoder-codec-specific` | WPT | 2026-10-10 | Chrome 154 `3/4`, Edge 154 `4/4`, Firefox 157 `2/4`, Safari 27.0 `2/4` |

## 4. H.264 / HEVC bitstream format

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Spec | Encoder default for H.264 is `"avc"` | REG-AVC | 2026-10-10 | `dictionary AvcEncoderConfig { AvcBitstreamFormat format = "avc"; };` "format, of type AvcBitstreamFormat, defaulting to "avc"" |
| Spec | Meaning of the two H.264 formats | REG-AVC | 2026-10-10 | annexb: "SPS and PPS data are included periodically throughout the bitstream." avc: "SPS and PPS data are not included in the bitstream and are instead emitted via the [[output callback]] as the VideoDecoderConfig.description of the EncodedVideoChunkMetadata.decoderConfig." |
| Spec | Decoder side: `description` present means avc format, absent means Annex B | REG-AVC | 2026-10-10 | "If the description is present, it is assumed to be an AVCDecoderConfigurationRecord [...] and the bitstream is assumed to be in avc format." "If the description is not present, the bitstream is assumed to be in annexb format." |
| Spec | Key chunk in Annex B must carry parameter sets | REG-AVC | 2026-10-10 | "then the EncodedVideoChunk is expected to contain both a primary coded picture that is an instantaneous decoding refresh (IDR) picture, and all parameter sets necessary to decode all video data NAL units in the EncodedVideoChunk." |
| Spec | HEVC mirrors this; default `"hevc"` | REG-HEVC | 2026-10-10 | `dictionary HevcEncoderConfig { HevcBitstreamFormat format = "hevc"; };` "If the description is not present, the bitstream is assumed to be in annexb format." |
| Chrome / Edge | Honors `avc.format` and `hevc.format` (source, main). WPT passes both formats for H.264 | CR-VE; WPT | 2026-10-10 | `case V8AvcBitstreamFormat::Enum::kAnnexb: result->options.avc.produce_annexb = true;` and the same for `V8HevcBitstreamFormat`. WPT: `?h264_annexb 5/5`, `?h264_avc 5/5` |
| Chrome / Edge | Decoding avc-format H.264 or hevc-format HEVC without `description` fails with a key-frame error | CR-VD | 2026-10-10 | `"A key frame is required after configure() or flush(). If you're using AVC formatted H.264 you must fill out the description field in the VideoDecoderConfig."` |
| Firefox | Has `avc.format` only. `VideoEncoderConfig` has no `hevc` member (source, main) | FF-VE | 2026-10-10 | `dictionary AvcEncoderConfig { AvcBitstreamFormat format = "avc"; };` and `VideoEncoderConfig` ends with `// AVC (H264)-specific` / `AvcEncoderConfig avc;` |
| Firefox | In `avc` format the emitted `description` (avcC) is malformed on Linux and Windows: bug open since Firefox 149 | BMO-2031056; BMO-2049470 | 2026-10-10 | 2031056 (NEW, "WebCodecs H.264 VideoEncoder malformed AVCC SPS"): "On Firefox 149.0.2 it outputs: [...] SPS[0] (26 bytes): 67 67 64 00 0d". 2049470 (NEW): "the avcC's embedded SPS ends up double-headed -- e.g. `67 67 42 00 1e ...` instead of `67 42 00 1e ...`." "The encoded bitstream (the frames themselves) is unaffected -- only the out-of-band avcC (`AVCDecoderConfigurationRecord`) is malformed." "Firefox's own macOS `AppleVTDecoder` rejects it" |
| Firefox | Annex B input with bytes before the first start code was not detected as Annex B; patch attached 2026-09-23, bug still ASSIGNED | BMO-1918769 | 2026-10-10 | "Annex B bitstreams with extra bytes before the first NALU start code are now detected and decoded as Annex B." |
| Firefox 157 | WPT passes both H.264 formats | WPT | 2026-10-10 | `?h264_annexb 5/5`, `?h264_avc 5/5`, `videoDecoder-codec-specific.https.any.html?h264_annexb 22/22`, `?h264_avc 22/22` |
| Safari | Honors `avc.format` and `hevc.format` in `main` | WK-VE | 2026-10-10 | `bool useAnnexB = (config.avc && config.avc->format == AvcBitstreamFormat::AnnexB) \|\| (config.hevc && config.hevc->format == HevcBitstreamFormat::AnnexB);` |
| Safari | Open bug says `hevc.format: "annexb"` is ignored and `avc.format` is what works for H.265 (filed 2024-10-23, still NEW) | WKB-281945 | 2026-10-10 | "it seems like `{avc: {format: "annexb"}}` (the H264 version) is what actually works in Safari." |
| Safari | Decoder treats H.264 / H.265 without `description` as Annex B (source, main) | WK-PLAT | 2026-10-10 | `bool isAnnexB = (*type == WebCore::VideoCodecType::H264 \|\| *type == WebCore::VideoCodecType::H265) && config.description.isEmpty();` |
| Safari | Past fix in the HEVC Annex B path | SRN-17_4 | 2026-10-10 | "Fixed an HEVC decoder issue when translating annexb data. (116768196)" |
| Safari 27.0 | WPT passes both formats for H.264 and HEVC decode | WPT | 2026-10-10 | `videoDecoder-codec-specific.https.any.html?h264_annexb 22/22`, `?h264_avc 22/22`, `?h265_annexb 22/22`, `?h265_hevc 22/22` |
| Spec discussion | `avc3` / `hev1`: encoders always emit avc1 / hvc1-style streams | GH-857 | 2026-10-10 | dalecurtis: "WebCodecs VideoEncoder requires explicit config changes, so no matter what we always emit AVC1/HVC1 compliant streams." (issue open) |

## 5. `EncodedVideoChunkMetadata.decoderConfig`

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Spec | Emitted when the output config differs from the last one emitted, which includes the first output | SPEC | 2026-10-10 | "If outputConfig and [[active output config]] are not equal dictionaries: Assign outputConfig to chunkMetadata.decoderConfig. Assign outputConfig to [[active output config]]." |
| Spec | The config must be sufficient to decode | SPEC | 2026-10-10 | "The User Agent MUST ensure that the configuration is completely described such that outputConfig could be used to correctly decode output." |
| Spec | It is optional for the application to use it | SPEC | 2026-10-10 | "A VideoDecoderConfig that authors MAY use to decode the associated EncodedVideoChunk." |
| Chrome / Edge | Emitted on first output and on change (source, main) | CR-VE | 2026-10-10 | `// We need to emit new config in the following cases:` `//  1. We haven't emitted a decoder config yet (first output)` `//  2. Decoder config has changed.` |
| Chrome / Edge | `colorSpace` falls back to BT.601 when the encoder reports none (source, main) | CR-VE | 2026-10-10 | `// TODO(https://crbug.com/1241448): All encoders should output color space.` `// For now, fallback to 601 since that is correct most often.` |
| Chrome | Historical: `temporalLayerId` was placed on the top-level metadata as well as under `svc` | CS-VIOL | 2026-10-10 | "Chrome places the temporalLayerId directly on the top level EncodedVideoChunkMetadata dictionary (metadata.temporalLayerId). As of Chrome 98, either option is available." |
| Firefox | `colorSpace` in the emitted config is a constant | BMO-2056028 | 2026-10-10 | "The emitted `EncodedVideoChunkMetadata.decoderConfig.colorSpace` is a hardcoded constant — always `{ primaries: "bt709", transfer: "bt709", matrix: "bt709", fullRange: false }`" (NEW, filed 2026-07-18) |
| Firefox | `description` malformed for H.264 `avc` format | BMO-2049470 | 2026-10-10 | see section 4 |
| Firefox | `alphaSideData` not implemented | FF-VE | 2026-10-10 | `// Not implemented https://bugzilla.mozilla.org/show_bug.cgi?id=1867067` `// BufferSource alphaSideData;` |
| Safari | Emitted when there is a new active configuration; the code carries a FIXME about accuracy (source, main) | WK-VE | 2026-10-10 | `if (m_hasNewActiveConfiguration) {` [...] `// FIXME: Provide more accurate decoder configuration` |
| All three engines | `codedWidth` / `codedHeight` in the emitted config hold the configured size, not the padded coded size | GH-925 | 2026-10-10 | "In all permutations I have tried ((Chrome, Firefox, Safari) x (Windows, Mac)), the `codedWidth` and `codedHeight` properties in the `VideoDecoderConfig` emitted by `VideoEncoder` contain the *display* dimensions of the frame" (open) |
| Spec discussion | `optimizeForLatency` is not propagated into the emitted config | GH-725 | 2026-10-10 | "the value of optimizeForLatency in the decoder configuration spit out by the encoder does not reflect the value of latencyMode set in the encoder configuration." (open) |

## 6. Encoder and decoder options

BCD has no per-option entries for `VideoEncoder.configure` or `VideoDecoder.configure` other than `flip` and `rotation`. The only per-option entries in the six files are the audio ones. The rest of this table therefore comes from each engine's IDL at `main` and from WPT.

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| BCD | Keys present under `VideoEncoder`: constructor, `close`, `configure`, `dequeue_event`, `encode`, `encodeQueueSize`, `flush`, `isConfigSupported_static`, `reset`, `state`. No `latencyMode`, `scalabilityMode`, `bitrateMode`, `contentHint`, `hardwareAcceleration` | BCD-VE | 2026-10-10 | `rg "latencyMode\|scalabilityMode\|contentHint\|hardwareAcceleration\|optimizeForLatency"` over the six files returns no match; `bitrateMode` matches only in AudioEncoder.json |
| BCD | `VideoDecoder.configure` options `flip` / `rotation`: Chrome 138 only | BCD-VD | 2026-10-10 | `configure.flip_option: chrome 138, firefox False, safari False`; `rotation_option` same |
| BCD | Audio `bitrateMode` | BCD-AE | 2026-10-10 | `configure.bitrateMode: chrome 119, firefox 130, safari 26` |
| BCD | `dequeue` event | BCD-VE, BCD-VD | 2026-10-10 | `dequeue_event: chrome 106, firefox 130, safari 16.4` |
| Spec | Defaults | SPEC | 2026-10-10 | `VideoEncoderBitrateMode bitrateMode = "variable";` `LatencyMode latencyMode = "quality";` `boolean optimizeForLatency = false;` "hardwareAcceleration, of type HardwareAcceleration, defaulting to "no-preference"" |
| Spec | `latencyMode: "realtime"` | SPEC | 2026-10-10 | "User Agents MAY sacrifice quality to improve latency. User Agents MAY drop frames to achieve the target bitrate and/or framerate." In `quality`: "User Agents MUST not drop frames to achieve the target bitrate and/or framerate." |
| Spec | `optimizeForLatency` | SPEC | 2026-10-10 | "Hint that the selected decoder SHOULD be configured to minimize the number of EncodedVideoChunks that have to be decoded before a VideoFrame is output." |
| Spec | `hardwareAcceleration` is a hint | SPEC | 2026-10-10 | "prefer-hardware and prefer-software are hints. While User Agents SHOULD respect these values when possible, User Agents may ignore these values in some or all circumstances for any reason." |
| Spec | `contentHint` must not cause rejection | SPEC | 2026-10-10 | "The User Agent MUST NOT refuse the configuration if it doesn't support this content hint." |
| Chrome / Edge | IDL has `hardwareAcceleration`, `bitrateMode` (incl. `quantizer`), `scalabilityMode`, `latencyMode`, `contentHint`, `avc`, `hevc`; decoder has `optimizeForLatency` (source, main) | CR-VEIDL; CR-VD | 2026-10-10 | `enum VideoEncoderBitrateMode { "constant", "variable", "quantizer" };` `DOMString scalabilityMode;` `LatencyMode latencyMode = "quality";` `DOMString contentHint = "";` `boolean optimizeForLatency;` |
| Chrome / Edge | `scalabilityMode` values parsed: L1T1, L1T2, L1T3, and a `manual` keyword (source, main) | CR-VE | 2026-10-10 | `if (config->scalabilityMode() == "L1T1") {` [...] `"L1T2"` [...] `"L1T3"` [...] `} else if (config->scalabilityMode() == keywords::kManual) {` |
| Chrome | `contentHint`: chromestatus says "In development" with milestone 121; WPT passes on Chrome 154 | CS-HINT; WPT | 2026-10-10 | `"status": {"text": "In development"}`, `"desktop": 121`. WPT: `video-encoder-content-hint.https.any.html` Chrome `1/1`, Edge `1/1`, Firefox `1/1`, Safari `0/1` |
| Firefox | IDL has all of them; `avc` only, no `hevc` (source, main) | FF-VE | 2026-10-10 | `HardwareAcceleration hardwareAcceleration = "no-preference";` `DOMString scalabilityMode;` `VideoEncoderBitrateMode bitrateMode = "variable";` `LatencyMode latencyMode = "quality";` `DOMString contentHint;` |
| Firefox | `scalabilityMode`: only L1T2 and L1T3 pass the support check; SVC with AV1 needs constant bitrate; the WMF (Windows) encoder does SVC only for H.264 (source, main) | FF-VE; FF-PEM | 2026-10-10 | `if (!aConfig->mScalabilityMode->EqualsLiteral("L1T2") && !aConfig->mScalabilityMode->EqualsLiteral("L1T3")) {`; `// libaom only supports SVC in CBR mode.`; WMF: `if (aConfig.mScalabilityMode != ScalabilityMode::None && aConfig.mCodec != CodecType::H264) {` |
| Firefox | `optimizeForLatency` is accepted; an open bug reports buffering on Windows despite it | FF-VD; BMO-1953551 | 2026-10-10 | Bug (NEW): "WebCodec VideoDecoder is buffering frames with optimizeForLatency flag for some streams." "For some streams the VideoDecoder is buffering 5 frames before it pops the decoded frame" |
| Safari | IDL has `hardwareAcceleration`, `scalabilityMode`, `bitrateMode`, `latencyMode`, `avc`, `hevc`. No `contentHint`. `bitrateMode` has no `quantizer` (source, main) | WK-VE | 2026-10-10 | `DOMString scalabilityMode;` `BitrateMode bitrateMode = "variable";` `LatencyMode latencyMode = "quality";` `AvcEncoderConfig avc;` `HevcEncoderConfig hevc;` and `enum BitrateMode { "constant", "variable" };` |
| Safari | `scalabilityMode`: L1T1, L1T2, L1T3; anything else is a TypeError (source, main). Shipped for software codecs in 17.0 and for H.264 L1T2 in 17.2 | WK-VE; SRN-17; SRN-17_2 | 2026-10-10 | `else if (config.scalabilityMode != "L1T1"_s) return Exception { ExceptionCode::TypeError, "Scalabilty mode is not supported"_s };`. "Added support for WebCodecs temporal `scalabilityMode` for software codecs". "Added support for H264 L1T2 for WebCodecs. (114940765)" |
| Safari | `latencyMode: "realtime"` is passed to the encoder; a past bug required framerate and bitrate | WK-VE; SRN-17_4 | 2026-10-10 | `config.latencyMode == LatencyMode::Realtime`. "Fixed VideoEncoder produces no frames with latencyMode "realtime" when framerate/bitrate are not given. (118725549)" |
| Safari | `optimizeForLatency` is in the IDL, but VideoToolbox decoders do not yet act on it (open bug, 2026-09-25) | WK-VD; WKB-325278 | 2026-10-10 | `boolean optimizeForLatency;`. Bug title: "VTB Decoders should set kVTDecompressionPropertyKey_RealTime when WebCodecsVideoDecoderConfig.optimizeForLatency is true" |
| Safari | `hardwareAcceleration: "prefer-software"` on the decoder selects a non-hardware path; on macOS, VP9 then uses the local software decoder (source, main) | WK-VD; WK-PLAT | 2026-10-10 | `.decoding = config.hardwareAcceleration == HardwareAcceleration::PreferSoftware ? VideoDecoder::HardwareAcceleration::No : VideoDecoder::HardwareAcceleration::Yes`; `#if PLATFORM(MAC)` `if (*type == WebCore::VideoCodecType::VP9 && config.decoding == WebCore::VideoDecoder::HardwareAcceleration::No) return true;` |
| Safari | `alpha: "keep"` is rejected (source, main) | WK-VE | 2026-10-10 | `return Exception { ExceptionCode::NotSupportedError, "Alpha keep is not supported"_s };` |
| WPT, temporal SVC | Per codec | WPT | 2026-10-10 | `temporal-svc-encoding.https.any.html?vp8`: all four `2/2`. `?vp9`: all four `2/2`. `?av1`: Chrome `2/2`, Edge `2/2`, Firefox `2/2`, Safari `0/2`. `?h264`: Chrome `2/2`, Edge `2/2`, Firefox `0/2`, Safari `0/2` |

## 7. Key frame requests and per-frame QP

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Spec | `keyFrame: true` is a MUST | SPEC | 2026-10-10 | "A value of true indicates that the given frame MUST be encoded as a key frame. A value of false indicates that the User Agent has flexibility to decide whether the frame will be encoded as a key frame." |
| Chrome / Edge | `keyFrame` in IDL (source, main) | CR-VEIDL | 2026-10-10 | `boolean keyFrame = false;` |
| Firefox | `keyFrame` in IDL (source, main) | FF-VE | 2026-10-10 | `dictionary VideoEncoderEncodeOptions { boolean keyFrame = false;` |
| Safari | `keyFrame` is the only encode option (source, main) | WK-VE | 2026-10-10 | `dictionary WebCodecsVideoEncoderEncodeOptions { boolean keyFrame = false; };` |
| All four | General encoder WPT file passes fully. I did not find a documented difference in `keyFrame` behavior | WPT | 2026-10-10 | `video-encoder.https.any.html`: Chrome `9/9`, Edge `9/9`, Firefox `9/9`, Safari `9/9` |
| Spec | Per-frame quantizer ranges | REG-AVC; REG-VP9; REG-AV1 | 2026-10-10 | AVC: "the quantizer threshold can be varied from 0 to 51." VP9: "from 0 to 63". AV1: "the quantizer index can be varied from 0 to 255" |
| MDN | MDN still gives 0-63 for AV1, which disagrees with the registration | MDN-VEENC | 2026-10-10 | under `av1`: "Frame quantizer value 0 to 63." |
| Chrome / Edge | Per-frame QP since 117 for AV1, VP9, AVC; `hevc` option also in IDL at `main` | CS-QP; CR-VEIDL | 2026-10-10 | "Adds "quantizer" VideoEncoderBitrateMode for VideoEncoder. This allows to specify quantizer parameter for each frame for AV1, VP9, and AVC video codecs." `"milestone_str": "117"`. IDL: `VideoEncoderEncodeOptionsForHevc hevc;` |
| Firefox | IDL has `avc.quantizer` only, and the `quantizer` bitrate mode; the tracking bug for implementing it is still NEW. WPT per-frame QP passes for AV1 and VP9 anyway | FF-VE; BMO-1871457; WPT | 2026-10-10 | IDL: `dictionary VideoEncoderEncodeOptionsForAvc { unsigned short? quantizer; };`. Bug: "defer the `quantizer` [1] bitrate implementation later. This is the tracking bug." WPT Firefox 157: `per-frame-qp-encoding.https.any.html?av1 2/2`, `?vp9_p0 2/2`, `?vp9_p2 2/2` |
| Safari | No per-frame QP: no `quantizer` bitrate mode and no codec-specific encode options (source, main); WPT fails | WK-VE; WPT | 2026-10-10 | `enum BitrateMode { "constant", "variable" };`. WPT Safari 27.0: `per-frame-qp-encoding.https.any.html?vp9_p0 0/2`, `?av1 0/2` |

## 8. `isConfigSupported()`

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Spec | What "supported" means | SPEC | 2026-10-10 | "If the User Agent can provide a codec to support all entries of the config, including applicable default values for keys that are not included, return true." |
| Spec | It is best effort and can change | SPEC | 2026-10-10 | "Support for a given configuration can change dynamically if the hardware is altered (e.g. external GPU unplugged) or if essential hardware resources are exhausted. User Agents describe support on a best-effort basis given the resources that are available at the time of the query." |
| Spec | Unknown members are dropped from the returned config, not rejected | SPEC | 2026-10-10 | "The returned VideoDecoderSupport config will contain only the dictionary members that User Agent recognized. Unrecognized dictionary members will be ignored. Authors can detect unrecognized dictionary members by comparing config to their provided config." |
| Spec | Invalid config rejects | SPEC | 2026-10-10 | "If config is not a valid VideoDecoderConfig, return a promise rejected with TypeError." |
| Spec | Decoder check is on exact profile and level; encoder check allows a lower level | SPEC | 2026-10-10 | "can't provide a codec that can decode the exact profile (where present), level (where present), and constraint bits (where present)"; encoder: "can't provide a codec that can encode to a level less than or equal to the level indicated by config.codec" |
| Safari | `supported: true` for HEVC, then decode failed (REOPENED) | WKB-262950 | 2026-10-10 | "(await VideoDecoder.isConfigSupported(decoder_config)).supported // returns True But then, decoding HEVC failed with error: Unhandled Promise Rejection: EncodingError: Decoder failure." |
| Safari | AV1 reported unsupported in windows opened from another page, supported when loaded directly (NEW, 2026-07-02) | WKB-318440 | 2026-10-10 | "VideoDecoder.isConfigSupported() returns {supported: false} for an AV1 configuration in any browsing context created from another page [...] The same configuration returns {supported: true} when the page is loaded directly." |
| Safari vs Chrome | Unsupported `scalabilityMode`: once rejected in Safari and resolved false in Chromium; reported aligned on 2023-11-15. WebKit `main` still throws TypeError for unknown modes in its config conversion | GH-744; WK-VE | 2026-10-10 | "On Chromium, when an unsupported `scalabilityMode` value is provided, the promise resolves with `supported` set to "false", but on Safari, the promise is rejected." Later: "there is no longer a discrepancy between Chromium and Safari with respect to handling of `scalabilityMode`." |
| Firefox | `supported: true` with `prefer-hardware`, then decode failed (ASSIGNED) | BMO-1918769 | 2026-10-10 | "The decoder config with codec "avc1.4d001f" and prefer-hardware enabled returns true when we check if it is supported." [...] "The video decoder complains "DOMException: The given encoding is not supported."" |
| Firefox | Decoder support check does not consult hardware; bug open | BMO-1968022; BMO-1932392; FF-VD | 2026-10-10 | 1968022 title (ASSIGNED): "Implement proper VideoDecoder.isConfigSupport w/ hardware support check". 1932392 title (UNCONFIRMED): "VideoDecoder.isConfigSupported() incorrectly reports true". Source: `// TODO (1880326): code below is wrongly using the logic of HTMLMediaElement for determining if a codec can be played` |
| Firefox | HEVC reported unsupported on Windows Nightly although hardware decode exists | BMO-1999168 | 2026-10-10 | "It returns a promise that tells me don't support this config" [...] "In `about:support`, under **Media / Codec Support Information**, it shows that HEVC decoding is supported" |
| Firefox | Applies resist-fingerprinting adjustments before answering (source, main) | FF-VE | 2026-10-10 | `ApplyResistFingerprintingIfNeeded(aConfig, aGlobal);` |

## 9. Cross-browser interop problems

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Firefox as publisher, H.264 `avc` format | Malformed avcC in `decoderConfig.description` on Linux and Windows. A viewer that parses the description gets the wrong profile and size | BMO-2049470 | 2026-10-10 | "A spec-strict decoder configured with such a description decodes a wrong profile/size; Firefox's own macOS `AppleVTDecoder` rejects it" |
| Chrome, Safari | H.264 encode requires even width and height | CR-VE; WK-VE | 2026-10-10 | Chromium: `*js_error_message = "H264 only supports even sized frames.";`. WebKit: `return Exception { ExceptionCode::TypeError, "H264 only supports even sized frames"_s };` |
| Firefox | Odd-sized frames: Windows encoder declines them for H.264; odd-sized `VideoFrame` WPT fails | FF-PEM; WPT; BMO-1899811 | 2026-10-10 | `// which can express an odd width; H.264 cannot and is declined above.` WPT `videoFrame-odd-size.any.html`: Chrome `2/2`, Edge `2/2`, Firefox `0/2`, Safari `2/2`. Bug 1899811 (NEW): "Allow odd-sized VideoFrame with subsampling" |
| Safari | `VideoFrame` constructor with odd coded size (NEW) | WKB-293425 | 2026-10-10 | title: "WebCodecs VideoFrame constructor doesn't work with odd codedWidth/codedHeight" |
| Safari | Maximum encode dimension 32767 (source, main) | WK-VE | 2026-10-10 | `constexpr size_t maxFrameDimension = 32767;` |
| Safari | `avc3.` codec strings are not accepted; Firefox accepts `avc1` and `avc3` (source, main) | WK-VE; FF-UTIL | 2026-10-10 | WebKit: `config.codec.startsWith("avc1."_s)`. Firefox: `StringBeginsWith(aCodecString, u"avc1"_ns) \|\| StringBeginsWith(aCodecString, u"avc3"_ns)` |
| Firefox | H.264 codec string is checked strictly in the decoder (source, main) | FF-VD | 2026-10-10 | `ExtractH264CodecDetails(aConfig.mCodec, profile, constraint, level, H264CodecStringStrictness::Strict);` |
| Firefox | `vp9` / `av1` short strings rejected; must be `vp09.` / `av01.`; VP8 must be exactly `vp8` | FF-UTIL | 2026-10-10 | `// The only codec string accepted for vp8 is "vp8"`; `// Gecko allows codec string starts with vp9 or av1 but Webcodecs requires to starts with av01 and vp09.` |
| Safari | Decoder output order with B-frames was wrong before 27.0; an H.264 ordering fix also landed in 26.4 | SRN-27; SRN-26_4 | 2026-10-10 | 27.0: "Fixed an issue where the WebCodecs `VideoDecoder` API output frames in an incorrect order for videos containing B-frames. (145093697)". 26.4: "Fixed an issue where `WebCodecs VideoDecoder` could output H264 frames in the wrong order. (168046597)" |
| All | H.264 decode latency depends on the platform decoder and on bitstream reorder signalling, not on `optimizeForLatency` alone | GH-732; GH-900 | 2026-10-10 | jyavenard: "the WMF (Windows) decoder has a default latency of about 25+ frames, and if configure for low-latency will still be around 8 frames on Windows 8 and 10." sandersdan: "It is possible to specify a bitstream_restriction, which can limit the size of the buffer (max_dec_frame_buffering) and signal the maximum time that a frame can wait around to be output (max_num_reorder_frames)." dalecurtis (GH-900): "Providing inputs until you get an output one per dequeue is the only surefire approach at the moment." |
| Chrome on macOS | Report of about 3 s H.264 decode delay with a hardware decoder (closed; a Chromium bug was filed) | GH-899 | 2026-10-10 | "the decoder takes about couple of seconds(around 3 seconds most of the times) to decode the frames, irrespective of the profile I use" |
| Firefox Windows, Safari macOS | Decoder buffers several frames at low frame rates despite `optimizeForLatency` | BMO-1953551 | 2026-10-10 | "We encounter the problem only for some streams and only in Firefox on Windows and Safari on Mac." |
| All | Key chunk definition for H.264 is IDR; I-frames with recovery point SEI are under discussion, and Chromium accepts them from 145 according to a commenter | GH-650; REG-AVC | 2026-10-10 | Vanilagy, 2025-12-05: "doing so only makes sense on Chromium version 145 and up" (issue open) |
| Firefox | Encoder color handling: RGB input converted with BT.601 while the config says BT.709; AV1 bitstream carries no color description | BMO-2056028; BMO-2057760 | 2026-10-10 | 2057760 title (UNCONFIRMED): "VideoEncoder converts RGB input with a BT601 matrix but emits decoderConfig.colorSpace claiming BT709" |
| Firefox | Decoder reports full-range H.264 as limited range (NEW) | BMO-2055639 | 2026-10-10 | title: "VideoDecoder reports full-range H.264 (video_full_range_flag=1) frames as limited range" |
| Safari | Color fixes in 27.0 | SRN-27 | 2026-10-10 | "Fixed an issue where I420 BT709 `VideoFrame` was encoded in an incorrect color space when encoding to VP9. (169425608)"; "Added support for overriding color space for hardware video decoders in WebCodecs. (178717498)" |
| Safari | Open color bugs | WKB-309518; WKB-325914 | 2026-10-10 | titles: "WebCodecs VideoDecoder reports incorrect VideoFrame.colorSpace for BT.2020/HLG (HDR) video"; "WebCodecs allows partial color spaces for video decoding/encoding" |
| Safari | VP9 encoder drops frames in CBR (NEW, 2026-10-01) | WKB-325913 | 2026-10-10 | "WebCodecs VP9 encoder is silently dropping frames due to CBR. This can be exercised when using frames with random noise, making them difficult to compress." |
| Safari | Encoder bitrate control and quality reports (NEW) | WKB-274822; WKB-258060 | 2026-10-10 | 274822: "the bitrate cannot be controlled if set 2000K bitrate." 258060 title: "The quality of video encode by VideoEncoder of Webcodecs on mac safari is poor" |
| Safari | HEVC decoded output differs from Chrome / FFmpeg (NEW); HEVC stream starting at a CRA frame fails (NEW) | WKB-276152; WKB-297788 | 2026-10-10 | titles: "Webcodecs HEVC decoded result is different from Chrome / FFMPEG"; "VideoDecoder error when starting to decode an H.265 stream from a non-IDR CRA_NUT frame" |
| Chrome, Edge, Firefox, Safari | High bit depth encode: only Chromium passes WPT. WebKit has it behind a non-default pref | WPT; WK-PREF; WKB-325753 | 2026-10-10 | `video-encoder-hbd.https.any.html?h264`: Chrome `9/9`, Edge `9/9`, Firefox `0/9`, Safari `0/9`. Pref `WebCodecsHighBitDepthEnabled`: `defaultValue: false` |
| Chrome 154 / Edge 154 | `video-encoder-h264.https.any.html?high-6.2` fails; Firefox passes | WPT | 2026-10-10 | Chrome `0/1`, Edge `0/1`, Firefox `1/1`, Safari `0/1` |
| Firefox | Windows: batch encoding is enabled only on Windows; elsewhere batch size is 1 (source, main) | FF-PREF | 2026-10-10 | `# Bug 1984936: Batch encoding is not optimized on non-Windows platforms yet.` `value: 1` |

## 10. Licensing and availability statements

| Browser | Finding | Source | Read | Verbatim |
|---|---|---|---|---|
| Chrome | HEVC: hardware only, no software implementation | CS-HEVCRTC; BLINKDEV | 2026-10-10 | "if it is available in hardware (we will not provide a software implementation)". Dale Curtis, 2025-02-12: "Especially so in the case of HEVC where it requires hardware support." |
| Chrome | HEVC hardware decode platforms | CS-HEVCDEC; MDN-VCODECS | 2026-10-10 | "(Android 5.0+, macOS 11+, with supported hardware on Windows 8+ and ChromeOS)". MDN: "Chrome support HEVC for devices with hardware support on Windows 8+, Linux and ChromeOS, for all devices on macOS Big Sur 11+ and Android 5.0+." |
| Chrome | HEVC encode: ChromeOS and Linux not covered | BLINKDEV | 2026-10-10 | "ChromeOS and Linux support may follow in the future when/if their platform codec availability allows." |
| Chrome | OpenH264 is the bundled H.264 software encoder and depends on `proprietary_codecs` (so not in plain Chromium builds without it) | CR-GNI | 2026-10-10 | `# Enable usage of OpenH264 within the media library. Used for software based encoding of H264 content.` |
| Edge | HEVC needs the Microsoft Store extension on Windows | MDN-VCODECS | 2026-10-10 | "when HEVC video extensions from the Microsoft Store is installed" |
| Firefox | H.264 through OS codecs | MDN-VCODECS | 2026-10-10 | "Firefox support for AVC is dependent upon the operating system's built-in or preinstalled codecs for AVC and its container in order to avoid patent concerns." |
| Firefox | HEVC on Windows software path needs a paid extension | MDN-VCODECS | 2026-10-10 | "Windows from Firefox 134 using either hardware (on devices that support it, where the range is the same as Edge) or software (on Windows the user must pay for and install an extension)" |
| Firefox Android | No bundled software H.264 / HEVC | FF-PEM | 2026-10-10 | `// Because we don't provide software implementations of H264 or HEVC on Android, we must use the platform software encoders even if true hardware encoding support is missing.` |
| Safari | AV1 only with hardware decode | WKBLOG-17.0; SRN-17_5; MDN-VCODECS | 2026-10-10 | see section 2 |
| Spec | Hardware acceleration and robustness | SPEC | 2026-10-10 | "For decoding, hardware acceleration is often less robust to inputs that are mislabeled or violate the relevant codec specification." "Hardware acceleration often has higher startup latency but more consistent throughput performance." |
| MDN | H.264 licensing pointer | MDN-VCODECS | 2026-10-10 | "be aware of its licensing requirements before choosing to use AVC in your project!" |

## Matrix: codec x browser x encode / decode

"SW" = bundled or OS software codec, "HW" = platform hardware codec. Footnotes give the source key and section.

### Video

| Codec | Chrome / Edge desktop | Chrome / Edge Android | Firefox desktop (157) | Firefox Android (157) | Safari macOS (27) | Safari iOS / iPadOS (27) |
|---|---|---|---|---|---|---|
| H.264 encode | yes (HW or SW) [1] | yes; SW absent on 32-bit ARM builds, so HW only there [2] | yes (platform encoders; HW or SW depends on OS) [5] | no [8] | yes (HW seen; SW variant passes WPT) [9] | yes per source; HW/SW unknown [9] |
| H.264 decode | yes [3]; HW vs SW not sourced | yes; HW vs SW not sourced | yes [6] | no [8] | yes [10] | yes per source [10] |
| VP8 encode | yes (SW) [1] | yes (SW) [1] | yes (SW) [5] | no [8] | yes (SW) [11] | yes per source (SW) [11] |
| VP8 decode | yes [3] | unknown (not sourced) | yes [6] | no [8] | yes [10] | yes per source [11] |
| VP9 encode | yes (SW; HW not sourced) [1] | yes (SW) [1] | yes (SW); profile 2 fails WPT [5] | no [8] | yes (SW); profile 2 fails WPT [11] | yes per source (SW) [11] |
| VP9 decode | yes [3] | unknown (not sourced) | yes [6] | no [8] | yes (HW or SW) [11] | yes per source [11] |
| AV1 encode | yes (SW) [1] | unknown [4] | yes (SW) [5] | no [8] | device-dependent: only where AV1 is enabled (HW-decoder devices); unverified [12] | same [12] |
| AV1 decode | yes (SW available) [3] | unknown (not sourced) | yes [6] | no [8] | HW only [12] | HW only [12] |
| HEVC encode | HW only, Windows / macOS; no on Linux / ChromeOS [13] | HW only [13] | no (Nightly-only pref, macOS / Linux) [7] | no [8] | yes [14]; HW/SW not sourced | yes per source [14] |
| HEVC decode | HW only; Edge on Windows also needs the HEVC Video Extensions [13] | HW only [13] | no (Nightly-only pref, macOS / Linux) [7] | no [8] | yes [14] | yes per source [14] |

### Audio

| Codec | Chrome / Edge desktop | Chrome / Edge Android | Firefox desktop (157) | Firefox Android (157) | Safari macOS / iOS (26+) |
|---|---|---|---|---|---|
| Opus encode | yes (SW) [15] | yes (SW) [15] | yes [16] | no [8] | yes [17] |
| Opus decode | yes [15] | unknown (not sourced) | yes [16] | no [8] | yes [17] |
| AAC (`mp4a.40.2`) encode | Windows (not N) and macOS: yes (platform). Linux: no [15] | yes (platform) [15] | no [16] | no [8] | yes [17] |
| AAC decode | yes [15] | unknown (not sourced) | yes [16] | no; refused explicitly [16] | yes [17] |

Footnotes:

1. CR-VE: bundled software encoders for AV1, VP8/VP9, H.264 (section 2). Hardware H.264 encode: WPT `h264_annexb_hardware` fails on the Linux and Windows CI machines (`0/1`), so hardware encode is not demonstrated by the data read; the encoder code does consult platform encoders (`is_platform_encoder`).
2. CR-GNI: `is_android && current_cpu == "arm"` disables OpenH264 (section 2).
3. WPT Chrome 154 Linux and Edge 154 Windows: `videoDecoder-codec-specific.https.any.html?h264_avc 22/22`, `?vp8 22/22`, `?vp9 22/22`, `?av1 22/22`. CR-GNI for dav1d and libvpx.
4. CS-AV1ENC lists `"desktop": 90, "android": null` for the (WebRTC-oriented) AV1 encoder. I did not find a source that states whether libaom is built into Chrome for Android.
5. FF-PEM, WPT Firefox 157 Linux (section 2). Which encoder serves H.264 on Linux was not determined beyond "FFmpeg" (BMO-2049470).
6. WPT Firefox 157 Linux: `videoDecoder-codec-specific.https.any.html?h264_avc 22/22`, `?h264_annexb 22/22`, `?vp8 22/22`, `?vp9 22/22`, `?av1 22/22`.
7. FF-UTIL, FF-PREF (section 2).
8. FF-PREF, BMO-1934008, FF-VE, FF-VD, FF-AE (section 1).
9. SRN-17_2, WPT Safari 27.0 (section 2).
10. WPT Safari 27.0 macOS: `videoDecoder-codec-specific.https.any.html?h264_avc 22/22`, `?h264_annexb 22/22`, `?vp8 22/22`, `?vp9 22/22`.
11. WK-PLAT, SRN-17, SRN-17_4, SRN-17_5 (section 2). iOS cells rest on WebKit source and release notes; the WPT run is macOS only.
12. SRN-17_5, WK-PREF, WK-PLAT (section 2). WPT Safari 27.0 run: all AV1 tests `0/N`, consistent with a CI Mac without an AV1 hardware decoder.
13. CS-HEVCDEC, CS-HEVCRTC, BLINKDEV, CR-SW, MDN-VCODECS (sections 2 and 10).
14. SRN-17_4; WPT Safari 27.0: `full-cycle-test.https.any.html?h265_hevc 5/5`, `videoDecoder-codec-specific.https.any.html?h265_hevc 22/22`.
15. CR-AE, CR-SW, WPT (section 3).
16. FF-AE, FF-AD, FF-UTIL, WPT (section 3).
17. WK-AUDIO, SRN-26, WPT (section 3). Encode cells rest on the allow-list in WebKit source; WPT `audio-encoder-codec-specific` is `2/4` on Safari 27.0 and I did not open the subtests.

## Disagreements between sources

| Topic | Source A | Source B |
|---|---|---|
| Chrome stable version | BCD-BR: `chrome 155 current 2026-10-06`, `156 beta 2026-10-20` | CHROMEDASH: `Stable Windows 156.0.8078.12` dated 2026-10-07 |
| Chrome version used by other data | CANIUSE lists `and_chr` 154 as its only entry and Chrome up to 157 | WPT "stable" run of 2026-10-06 is Chrome 154.0.8037.92 |
| Firefox Nightly version | BCD-BR: `firefox 159 nightly`, `158 beta 2026-10-13` | MOZ-PD: `"FIREFOX_NIGHTLY": "160.0a1"`, `"NEXT_RELEASE_DATE": "2026-10-23"` |
| Firefox Android `VideoFrame` | BCD-VF: `firefox_android: mirror` (so 130) | BCD-VE / BCD-VD / BCD-AE: `firefox_android: False`; FF-PREF: `dom.media.webcodecs.enabled` is Nightly-only on Android. `VideoFrame` is gated by `Func="mozilla::dom::VideoFrame::PrefEnabled"`, which I did not open |
| Firefox Android overall | CANIUSE: `and_ff 157: n` | BCD-VF as above |
| Safari full support version | CANIUSE: `safari 16.4: a #1` ("Video-only support"), `26.0: y` | Agrees with SRN-16_4 and SRN-26; listed only to note that caniuse's "partial" means "no audio" |
| Firefox per-frame QP | BMO-1871457 "Support Quantizer bitrate mode in VideoEncoder" is NEW; FF-VE IDL comments mark `quantizer` as AVC-specific | WPT Firefox 157: `per-frame-qp-encoding.https.any.html?av1 2/2`, `?vp9_p0 2/2` |
| AV1 quantizer range | REG-AV1: "from 0 to 255" (changed by GH-904, closed 2025-12-04) | MDN-VEENC: "Frame quantizer value 0 to 63" |
| Safari `hevc.format: "annexb"` | WKB-281945 (NEW): the `hevc` option is ignored | WK-VE at `main` reads `config.hevc->format == HevcBitstreamFormat::AnnexB`; WPT Safari 27.0 `full-cycle-test.https.any.html?h265_annexb 4/5` |
| Chrome `contentHint` | CS-HINT: "In development" | CR-VEIDL has `DOMString contentHint = "";` with no runtime flag, and WPT passes on Chrome 154 |
| Firefox HEVC in WebCodecs | Source comment: "H265 is supported only on MacOS in Nightly for now." | The code on the next line also allows Linux: `(IsOnMacOS() \|\| IsOnLinux())` |
| Safari `scalabilityMode` rejection | GH-744: discrepancy with Chromium reported gone on 2023-11-15 | WK-VE at `main` still returns a `TypeError` for unknown modes from `createVideoEncoderConfig`; whether `isConfigSupported` reaches that path was not traced |
| Edge Android version | EDGEUPD: Stable Android 154 | BCD-BR `edge` 155 current (BCD has no separate Edge Android entry) |

## Not found / could not fetch

- **BCD per-option data** for `latencyMode`, `scalabilityMode`, video `bitrateMode`, `contentHint`, `hardwareAcceleration`, `optimizeForLatency`, `avc`, `hevc`, `keyFrame`: these keys do not exist in the six BCD files. Section 6 uses engine IDL instead, which describes `main`, not a release.
- **Since-which-version for individual options** in Chrome and Firefox (for example when `latencyMode` shipped): not found in a doc source. Only Safari's release notes date some of them (17.0, 17.2, 17.4).
- **Chrome / Edge on Android, per-codec decode and AV1 encode**: no Android-specific source was read. WPT has no Android run in the aligned set.
- **Chrome H.264 / VP9 / AV1 hardware encode by platform**: not sourced. The WPT hardware-encode subtest fails on both Chromium CI machines.
- **Chrome H.264 decode, hardware vs software**: not sourced.
- **Edge-specific WebCodecs documentation** (HEVC Video Extensions in the WebCodecs path, AAC): no Microsoft page found. The only statement is MDN's, about Edge media in general. The Edge web-platform release notes were not read.
- **Chromium issue tracker** (crbug.com / issues.chromium.org): not fetched. Chromium-side bugs appear here only through code comments and w3c/webcodecs issues (for example crbug 436302044 mentioned in GH-899, crbug 1241448 in CR-VE).
- **Chrome for Developers release notes for 155 / 156**: BCD has an empty `release_notes` field for them and I did not fetch the pages.
- **Safari on iOS / iPadOS, measured behavior**: every iOS cell comes from WebKit source and Safari release notes, which cover all Apple platforms together. No iOS test run was available.
- **Safari H.264 / HEVC hardware vs software selection**: not sourced beyond the 17.2 note and the WPT hardware / software subtests.
- **Where WebKit turns `webCodecsAV1Enabled` on at runtime**: the default is `false` on Cocoa and the release note says it depends on a hardware decoder; the code that sets it was not located.
- **Firefox source files** `AppleEncoderModule.cpp` and `FFmpegDataEncoder.cpp`: HTTP 429 from raw.githubusercontent.com; not read. The exact H.264 encoder used by Firefox on Linux and macOS is therefore not confirmed from code.
- **firefox-source-docs**: no WebCodecs page was found or read.
- **Shipped-build verification**: all "(source, main)" rows describe the development tip on 2026-10-10. I did not check the release branches (Chrome 155 / 156, Firefox 157, Safari 27.0) and did not run any browser.
- **WPT subtest names**: only pass counts per test file were read, not which subtest failed.
- **Bugzilla comment bodies** for 1934008 and 1968022: the first comments are empty; status and titles only.
- **MDN "Codec String Support Table"** linked from MDN (`https://webcodecsfundamentals.org/datasets/codec-support-table/`): not fetched; it is a third-party dataset.
