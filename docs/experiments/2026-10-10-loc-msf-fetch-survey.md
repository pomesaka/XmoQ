# What following LOC and MSF takes: the drafts, FETCH, and who else implements them

- Date: 2026-10-10
- Commit SHA: the commit that adds this record. Nothing in `crates/`, `packages/` or `interop/` was changed for it, and nothing was run for sections 1 to 7. Section 8, added on 2026-10-11, ran the parts of `main` at `1af2dbf9`
- Roadmap item: groundwork for Phase 1. Material for how media is packaged; nothing is decided here, and no roadmap item is closed

## Objective

The project owner wants Phase 1 to package video as `draft-ietf-moq-loc` (LOC) and `draft-ietf-moq-msf` (MSF) say, with FETCH and a relay cache as far as they are needed. Find out from the texts and from other implementations what that obliges for the smallest case, one publisher, one video track, several viewers, loopback, no encryption, Chromium only, and where the texts leave a choice. **This record holds facts, choices and what is not known. It holds no decision.**

## Environment

| Item | Value |
|---|---|
| How | Reading. Two research passes were run for this record, one over the drafts and one over other implementations, both told to write nothing from memory and to quote. Their full tables are in [`spikes/msf-loc/msf-loc-moqt-sources.md`](../../spikes/msf-loc/msf-loc-moqt-sources.md) (541 lines) and [`spikes/msf-loc/implementations-sources.md`](../../spikes/msf-loc/implementations-sources.md) (161 lines), with a section, URL and quote a row |
| Drafts, fetched 2026-10-10 from `https://www.ietf.org/archive/id/` | `draft-ietf-moq-msf-01` (2 June 2026), `draft-ietf-moq-loc-04` (20 July 2026) and its revisions `-00` to `-03`, `draft-ietf-moq-transport-18` and `-19` |
| Implementations | Read on GitHub and in package registries on 2026-10-10. **None was built, run or connected to**, except moqlivemock in section 8, a day later |

**Checked a second time for this record, from the source:** MSF-01 §4.1, §5 (the catalog and the Joining FETCH sentence), §6, §6.1 and §6.2; the reference entries of MSF-01; the transport revision each LOC revision cites; LOC-04's table of property IDs; the two property tables of transport draft 18 (§15.8); draft 18 §9.1 on caching and the numbers of FETCH, FETCH_OK and FETCH_HEADER; that draft 18 has no FETCH_CANCEL; the latest release of `Eyevinn/moqlivemock`, its publisher's source (`internal/pub/pub.go`), its README, its `go.mod` and the flags of its three commands. **Everything else is as the research passes wrote it**, in particular the count of MSF's requirements, the differences between transport drafts 18 and 19, and every implementation other than moqlivemock.

## Commands

None for the tree. Drafts and repository files were fetched, for example:

```sh
curl -sL https://www.ietf.org/archive/id/draft-ietf-moq-msf-01.txt
gh api "repos/Eyevinn/moqlivemock/contents/internal/pub/pub.go?ref=v0.16.1" --jq .content | base64 -d
```

## Observed results

### 1. The versions do not line up

| Draft | Transport draft it cites | Note |
|---|---|---|
| MSF-01 | 18 | **cites LOC-02**, not LOC-04 |
| LOC-02 | 17 | Timestamp is property `0x06` |
| LOC-03 | **18** | Timestamp is `0x0A` in its text and `0x06` in its own IANA table |
| LOC-04 | 19 | Timestamp `0x10`, Timescale `0x08`, Video Frame Marking `0x09`, Video Config `0x0D`, Audio Level `0x0C`, Audio Config `0x0F` |

So "MSF-01 with LOC-04 on transport draft 18" is a combination no draft names. It is the one several implementations use (section 2).

- **LOC-04's properties can be carried on draft 18 as they are.** The name "Properties", the key-value encoding (types as deltas, a length for odd types only) and the variable-length integer have the same text in drafts 18 and 19 (research pass). None of LOC-04's six IDs is a property draft 18 defines itself (`0x02`, `0x04`, `0x06`, `0x0B`, `0x0E`, `0x22`, `0x30`, `0x3C`, `0x3E`). A relay has to pass and cache a property it does not know unchanged (draft 18 §2.5).
- **LOC-02's numbers cannot.** Its Timestamp `0x06` is SUBGROUP_DELIVERY_TIMEOUT in draft 18, a property of tracks only.
- The table of provisional registrations in drafts 18 and 19 has Timestamp `0x06` and Video Frame Marking `0x0A`, which is neither LOC-04 nor consistent with draft 18's own `0x06`.
- Drafts 18 and 19 differ little, by the research pass: the same message numbers, one message renamed, the same wire form of FETCH and of subgroup streams, a filter added. Not checked a second time.

Choices for the LOC revision: LOC-04's numbers on draft 18 (what other implementations do; no draft says so); LOC-02 as MSF-01 cites it (collides with draft 18; only moqtail does it); LOC-03, which cites draft 18 (contradicts itself on the Timestamp; nobody was found doing it); or moving the transport to draft 19 (leaves the peers of draft 18 behind).

### 2. Who else implements this on draft 18

**There are implementations of FETCH, LOC-04's numbers and an MSF-01 catalog on draft 18.**

| Implementation | What it could be for XmoQ | From |
|---|---|---|
| `Eyevinn/moqlivemock` v0.16.1 (Go, 30 September 2026; draft 18 only) | publisher (`mlmpub`), subscriber (`mlmsub`), relay with a cache (`mlmrel`). The closest | source and README read for this record |
| `Eyevinn/warp-player` (browser; draft 18 only) | a subscriber for an XmoQ publisher and relay | research pass |
| `openmoq/moq-playa` (browser) | publisher and subscriber; LOC-04 on its git `main` only, its npm package has LOC-01 | research pass |
| `moqtail` 0.15.0 and npm 0.13.0, the peers XmoQ has | **FETCH and Joining FETCH only.** Its LOC numbers are the old ones (Timestamp `0x06`, Frame Marking `0x0A`), and its catalog is CMSF and does not take `packaging: "loc"` | research pass |
| `moxygen`, `moqx` | a relay for FETCH and Joining FETCH; media is CMAF | research pass |
| `aiomoqt` (Python), `imquic` (C) | clients with the numbers and the catalog version in their code; their relays were not read | research pass |

- `moqtail`'s `main` moved to `moqt-22` on 2026-10-07; draft 18 is in its release tags and a `draft-18` branch.
- **Joining FETCH is gone from transport draft 20 on** (research pass). MSF-01's way to get a catalog holds while the transport is draft 18.
- The cross-implementation runner (`englishm/moq-interop-runner`) targets draft 18 with seven control-plane tests and none for FETCH, LOC or catalogs. No recorded result of two implementations exchanging LOC or an MSF catalog was found.
- Cloudflare's hosted relay is no peer for this: by its documentation, as read by another session of this project on 2026-10-10 (page dated 31 July 2026) and recorded in [MoQ library candidates](../moq-libraries.md), it speaks drafts 14 and 16 and has no FETCH.

**What moqlivemock v0.16.1 does, read in its source:**

- LOC Timestamp is property `0x10`; a comment tells how the number moved from `0x06` to `0x0A` to `0x10`.
- **It sends a LOC Group on one stream**: one `OpenSubgroup(groupNr, 0, …)` for the Group, and every frame of it written to that stream. **That is not what MSF-01 §6 says** (below).
- **Its Group ID is the wall-clock second**, and a Group is one second ("Each group number is `Unix_epoch_ms / 1000`").
- The decoder configuration is put in front of the payload of a key frame. No use of LOC's Video Config property was found.
- Its catalog is fetched with "a SUBSCRIBE (Filter Type = Largest Object) plus a relative joining FETCH at offset 0, per draft-ietf-moq-msf-01 §5". Its publisher answers FETCH for the catalog only.

### 3. What MSF-01 obliges for one video track

By the research pass's count MSF-01 has about 115 normative sentences: about 29 apply to publishing or playing one video track with a catalog (five of them are met by leaving a field out), about 71 are about features that would not be used (timelines, several qualities, encryption, URLs, log and metrics tracks), and about 15 need a decision. The count was not made a second time.

The ones that cannot be avoided, with the sentences checked for this record:

| Section | Requirement |
|---|---|
| §4.1 | "each EncodedAudioChunk or EncodedVideoChunk sample is placed in a separate MOQT Object. Samples that belong to the same Group of Pictures (GOP) MUST be placed within the same MOQT Group." |
| §5 | "The catalog track MUST have a case-sensitive Track Name of "catalog"." The first catalog Object of a Group is a whole catalog, later ones are deltas, and all of them "MUST be mapped to MOQT sub-group 0" |
| §5 | "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining FETCH (offset = 0) in order to obtain the latest complete catalog along with all subsequent catalog objects" |
| §6 | "Irrespective of the Section 4 in place, each MOQT Object MUST be mapped to a new MOQT Stream." |
| §6.1 | "Group IDs for a track MUST be unique and MUST increase monotonically. Within a continuous publishing session, each subsequent Group ID SHOULD increase by 1." After a restart the publisher "MUST ensure the new starting Group ID is greater than any previously published Group ID for that track" |
| §6.2 | "Object ID MUST be zero for the first Object within a Group and then MUST increase monotonically by one within that Group." |
| §5.2 | a video track has `codec` and `bitrate` (research pass) |

The smallest catalog the research pass could build from the required fields:

```json
{ "version": "draft-01",
  "tracks": [ { "name": "video", "packaging": "loc", "isLive": true,
                "codec": "avc1.42E01F", "bitrate": 1000000 } ] }
```

The value of `version` is "draft-XX" in MSF-01's text and "1" in every one of its examples; most implementations emit "draft-01" and the players take both (research pass).

**MSF-01 asks nothing** of relays, of cache duration, of delivery timeouts or priorities, of FETCH or filters for media tracks, or of timestamps on media Objects. Timelines are optional.

**LOC-04 has two MUSTs, both about encryption.** For an unencrypted video track nothing in it is mandatory: Timestamp, Video Config and Frame Marking are optional, `avc` and Annex B are both allowed with nothing to tell them apart, and how Groups and Object IDs are assigned is in its examples only (research pass).

Where a decision is needed:

| | What | The facts |
|---|---|---|
| 1 | **How Objects go onto streams** | §6 has a new stream for every Object. Draft 18 §2.2 forbids the Objects of one Subgroup on different streams, so that means a Subgroup an Object. §5 of the same draft puts every catalog Object of a Group into "sub-group 0", which with draft 18 §2.2 is one stream: the two do not both hold for a catalog Group of more than one Object. LOC-04's example (§4.2, not normative) has "one unidirectional QUIC stream … to deliver all the encoded video chunks within a MOQT group", and moqlivemock does that |
| 2 | Whether a player handles catalog deltas | MSF asks it of the receiver, also where the publisher never sends one |
| 3 | No change of a track's declared attributes | a change of size or codec means a new track name |
| 4 | Group IDs after a publisher restarts | section 5 below |
| 5 | Whether the end of a broadcast is told with an `isComplete` catalog | not a MUST |
| 6 | Whether `#msf:` URLs are taken | |
| 7 | Catalog compression | support for GZIP is a MUST, and the property that would carry it has the ID "TBD" |

### 4. FETCH and the cache in draft 18

- **A relay does not have to cache.** §9.1: "Relays MAY cache Objects, but are not required to." It may pass a FETCH on to a publisher (§9.5), and for Objects of the range it does not have it asks upstream (§10.12.3, research pass). It cannot present a missing Object as one that does not exist.
- **What MSF's Joining FETCH asks for.** Draft 18 has no field called "offset"; the one that fits is a Relative Joining Fetch (Fetch Type `0x2`) with Joining Start 0, which is what moqlivemock sends. With L the Largest Location the publisher reported in SUBSCRIBE_OK, the range is the Objects `{L.Group, 0}` to `{L.Group, L.Object}`: one Group, from its first Object to the largest one at the time of the subscription (§10.12.2.1, research pass).
- **Only the catalog needs it.** MSF-01 has no FETCH requirement for media tracks.

Three shapes that would meet MSF-01's MUST, from the least:

| Shape | Who keeps what |
|---|---|
| The relay keeps nothing and passes the FETCH on | the publisher answers a FETCH for its catalog. moqlivemock's publisher does this |
| The relay keeps the newest Group of every track | for a catalog that does not change, one Object. It can answer after the publisher has gone |
| The same, and players use a Joining FETCH on video too | the group of pictures in progress, about 350 kB at one second and 2.76 Mbit/s. Not asked for by MSF; it shortens the wait of a viewer that joins |

What a cache would leave to decide: its bounds (Groups and bytes a track), when it drops, whether it keeps a track after its publisher went, MAX_CACHE_DURATION, and the answer when the upstream FETCH fails. Defaults elsewhere, by the research pass: moqtail 1000 Groups and 30 minutes, moxygen 3 Groups, moqlivemock's relay 3 Groups (its `-cache-groups` flag, read for this record).

Messages and streams draft 18 has for it:

| Name | Number | |
|---|---|---|
| FETCH | `0x16` | on a request stream; Fetch Type `0x1` Standalone, `0x2` Relative Joining, `0x3` Absolute Joining |
| FETCH_OK | `0x18` | |
| REQUEST_ERROR | `0x5` | the refusal; INVALID_RANGE and INVALID_JOINING_REQUEST_ID among its codes |
| FETCH_HEADER | `0x05` | the type of the unidirectional stream that carries the Objects, which are laid out differently from a subgroup stream |

**Draft 18 has no FETCH_CANCEL**; a fetch is cancelled with STOP_SENDING on its request stream (§5.2). Beside these: LARGEST_OBJECT in SUBSCRIBE_OK and the Joining Location a publisher has to keep (§5.1), the subscription filter, which the own relay ignores today (a known gap), and Object Properties, which both own implementations read and write and no interop run has carried. Today both answer a FETCH with `NOT_SUPPORTED` and close the session for a FETCH_HEADER stream.

Fixtures: FETCH, FETCH_OK and FETCH_HEADER bytes could be taken from moqtail (the peer there is) or from moqlivemock; a LOC Object and a catalog only from moqlivemock.

### 5. Group IDs, and two publishers of one track

MSF-01 §6.1 and §6.2 are quoted above. For a publisher in a browser:

- It forgets its last Group ID when the page is loaded again, so the MUST after a restart is met most simply by taking the starting Group ID from the clock, as §6.1 suggests ("One approach is to use the current wall clock time in milliseconds since the Unix epoch as the starting Group ID"). Prior Group ID Gap is a SHOULD for a publisher that knows its previous ID, which such a page does not.
- **Two publishers of the same content do not get the same Group IDs that way.** Each starts at its own millisecond and counts up from there. The own relay treats Subgroups with the same Group and Subgroup ID from two sources as the same one and takes it from one of them; with different IDs it passes both.
- **They do when every Group ID is taken from the clock**, not only the first: moqlivemock numbers a Group by the whole second it starts in and makes a Group one second long. Two publishers with clocks that agree then give the group of pictures of a second the same ID, and §6.1 still holds (unique, increasing, by 1 while publishing goes on, larger after a restart).
- The same IDs are not the same bytes. Two encoders give different frames for the same second. With one stream an Object, the own relay can change sources between two frames of a group of pictures; with one stream a Group it changes at a key frame. MSF-01 says nothing about several publishers.

### 6. What one stream an Object meets in the own implementation

For 1280x720, 30 frames a second, a group of pictures of one second, about 10,000 bytes a frame:

| Limit (all provisional) | Value | With one stream an Object |
|---|---|---|
| Subgroups and Groups for which the relay remembers the source | 256 Subgroups, 8 Groups of a track | 8 Groups are 240 Subgroups; with two seconds a Group, 256 Subgroups are about 4 Groups |
| A slow subscriber is cut off at | 256 streams, 4096 Objects or 4 MiB waiting, a session | 256 streams are 8.5 seconds of video |
| Streams of the peer a browser client reads at once | 64 | 2.1 seconds |
| Streams of one peer the relay reads at once | 64 | 2.1 seconds, on the publisher's side |

What a page that does not read does in this shape and in the other was measured afterwards and is in [the backlog record](2026-10-10-subscription-backlog-in-chromium.md), section "Objects of the size of a video frame, in real time": it is cut off after 61 seconds with one stream an Object and after 171 with one stream a Group, and in both it is not told.

### 7. Taking moqlivemock as an interop peer: what it would need

Read from the repository at `v0.16.1`; nothing was built.

| | |
|---|---|
| Licence | MIT |
| Language and toolchain | Go; `go.mod` says `go 1.26.0`, the README "Go 1.25 or later". Go 1.26.0 is on this machine through `mise`; the project's `mise.toml` does not have it |
| Dependencies | `quic-go` 0.62.0, `webtransport-go` 0.12.0, `Eyevinn/moqtransport` 0.14.0 (a fork of `mengelbart/moqtransport`), `mp4ff`, and a test asset of ten seconds in the repository |
| Commands | `cmd/mlmpub` ("the server and publisher"), `cmd/mlmsub` (subscriber, can write what it gets to a file or to `ffplay`), `cmd/mlmrel` (relay; `-upstream`, `-cache-groups`, default 3), `cmd/mlmtest` (a client for the interop runner) |
| Run | `go run .` in a command's directory; `-addr` ("listen or connect address"), `-cert` and `-key` ("only used for server") |
| Certificates | `mkcert`, or a self-signed certificate with a fingerprint served on a side port (`-sideport`) |
| Namespaces | LOC with an MSF catalog under `mlm/msf/clear`; the prefix is set with `-nsprefix` |
| Transport | `moqt-18` only, over WebTransport and native QUIC |

Section 8 has what running it showed. Not found out when this section was written: how `mlmpub` is told to connect to a relay instead of listening (the flag texts imply it can); whether `mlmsub` and `mlmpub` accept the own relay's certificate by hash or need `mkcert`; how the interop directory would hold a Go program (the moqtail peers are copied sources that are built from `interop/peers/`). The README of moqlivemock names `draft-ietf-moq-msf-00` and `draft-mzanaty-moq-loc` in its LOC section and MSF-01 in its section on session setup, and its source LOC-04 for the Timestamp; which text it follows where it matters has to be read in the source.

### 8. moqlivemock run against the own parts (added 2026-10-11)

A spike to fill in what section 7 lists as not found out. **Nothing was added to `interop/`, and `crates/`, `packages/` and `interop/` were not changed.** The source was cloned into a scratch directory outside the repository at tag `v0.16.1` (commit `91d903e9933a452f8baaa860f9fcd20cf195e077`) and built with Go 1.26.0 from `mise`, which the project's `mise.toml` still does not have. The own parts were those of `main` at `1af2dbf9`; macOS 14.4 arm64, loopback, ports 47200 to 47220.

```sh
go build -o ../bin/mlmpub ./cmd/mlmpub        # and mlmsub, mlmrel
(cd cmd/mlmpub && mlmpub -addr 127.0.0.1:47200 -sideport 47201)
xmoq-moqt-client --mode subscribe --objects 1  --url https://127.0.0.1:47200/moq --cert-sha256 <fingerprint> --namespace mlm/msf/clear --track catalog --trace
xmoq-moqt-client --mode subscribe --objects 60 --url https://127.0.0.1:47200/moq --cert-sha256 <fingerprint> --namespace mlm/msf/clear --track video_400kbps_avc --trace
mlmrel -addr 127.0.0.1:47220 -upstream https://127.0.0.1:47200/moq           # then the same two subscriptions against port 47220
xmoq-relay --port 47210 ...; xmoq-moqt-client --mode publish --namespace xmoq/phase0 --track object ...
mlmsub -addr https://127.0.0.1:47210 -namespace xmoq/phase0 -catalog-track object -duration 8
```

| | Question | Found |
|---|---|---|
| 1 | Do the three commands build? | **Yes**, at the first try. Run from `cmd/mlmpub`, `mlmpub` finds its asset; without certificate files each server makes a self-signed one in memory |
| 2 | Can `mlmpub` connect to a relay? | **No.** It only listens: `cmd/mlmpub` has no call that dials, and its help text says it "acts as a MoQ server and publisher". The "listen or connect address" of `-addr` is misleading. **So `mlmpub` cannot publish through the own relay**, which does not connect to a publisher either. What can be run is an own client against `mlmpub`, an own client against `mlmrel` with `mlmpub` upstream, and `mlmsub` against the own relay |
| 3 | Certificates | No `mkcert` and no change to the machine's trust was needed. As servers, `mlmpub` and `mlmrel` print the SHA-256 of the certificate they made (ECDSA, 14 days), `mlmpub` also serves it at `/fingerprint` of `-sideport`, and the own Rust client pins it. As clients, `mlmsub` and `mlmrel` do not verify the server's certificate at all (`InsecureSkipVerify: true` in the source, no flag) |
| 4 | WebTransport | Both ways exist: `https://` in `-addr` or `-upstream` is WebTransport, at the path `/moq` on the servers. **Own client to moqlivemock server: works. moqlivemock client to the own relay: the WebTransport handshake fails**, below |
| 5 | The catalog and a LOC video track at an own subscriber | Received, directly from `mlmpub` and through `mlmrel`, below |
| 6 | `mlmsub` against the own relay | With the handshake worked around: SUBSCRIBE, then a Relative Joining FETCH, which the relay refuses, and `mlmsub` stops there. Below |

**The WebTransport handshake between a moqlivemock client and the own relay fails on the `:protocol` of the CONNECT request.** The Go library moqlivemock uses (`Eyevinn/webtransport-go`, a fork of `quic-go/webtransport-go` 0.12.0) sends `webtransport-h3`, the token of `draft-ietf-webtrans-http3-15`; `web-transport-proto` 0.6.2, under the own relay's `web-transport-quinn` 0.13.2, accepts `webtransport` alone (`src/connect.rs`: `if protocol != Some("webtransport")`). `mlmsub` reports `http3: parsing frame failed: Application error 0x0 (remote)`; the relay prints `HANDSHAKE_FAILED connection lost: failed to exchange h3 connect`, and with the cause printed for one run (a temporary change, taken back), `WrongProtocol(Some("webtransport-h3"))`. The Go library has a switch for the old token, `Transport.LegacyConnectProtocol`, whose comment names exactly this case; **no command of moqlivemock has a flag for it.** For the rest of item 6 the scratch copy of `mlmsub` was built with that field set, a change of one line. The other direction needs nothing: the Go server takes the old token the own client sends.

**What an own subscriber got from `mlmpub`** (and the same through `mlmrel`, which made no difference to what follows):

| | Catalog track | `video_400kbps_avc` |
|---|---|---|
| SUBSCRIBE_OK | Track Alias 0, LARGEST_OBJECT {0, 0} | Track Alias 0, no parameter |
| Objects | one: Group 0, Object 0, 2,715 bytes of JSON. **It came on the SUBSCRIBE alone, although the filter was Largest Object and the Object is the largest**: the publisher writes it to every new subscription "for backward compatibility with subscribe-only clients" (`internal/pub/pub.go`) | 60 asked for: 25 of Group 1791645264, 25 of 1791645265, 10 of the next. 25 frames a second |
| Streams | one, header type `0x1C` (explicit Subgroup ID 0, END_OF_GROUP, no properties) | **one stream a Group**, as read in the source and unlike MSF-01 §6. Header type `0x15`: properties, explicit Subgroup ID 0, Publisher Priority 128, neither END_OF_GROUP nor FIRST_OBJECT |
| Group ID | 0 | the wall-clock second |
| Object Properties | none | **one on every Object: `0x10`, an integer of 8 bytes, the wall-clock time in microseconds** (1791645264000000 on the first Object of Group 1791645264). Properties Length 9. No Video Config (`0x0D`) and no Frame Marking |
| Payload | | begins `00 00 00 19 67 4d 40 1f`: a length of 4 bytes and then the NAL unit, the SPS first on the key frame (20,434 bytes; the other frames about 900). Length-prefixed, not Annex B, with the parameter sets in the stream |

The catalog: `"version":"draft-01"`, a `generatedAt`, 13 tracks, all `"packaging":"loc"` and `"isLive":true`: nine video (AVC `avc3.4D401F`, HEVC, AV1, at three bitrates) and four audio (AAC, Opus). A video track has `name`, `role`, `codec`, `bitrate`, `width`, `height`, `framerate`, `renderGroup`, `altGroup`, `lang`; none has `initData` or `timescale`.

**The own Rust client read all of it without an error**: the SETUP, both SUBSCRIBE_OKs, a stream with 25 Objects, the property. That is the first time an own implementation read Objects and a property another implementation's publisher made up.

**What `mlmsub` sent the own relay**, from its own log of control messages (the built copy with the old token; an own publisher had the track `object` in `xmoq/phase0`, and `mlmsub` was told to take that track for its catalog):

| Step | Message |
|---|---|
| 1 | SETUP both ways; the session is established |
| 2 | SUBSCRIBE, Request ID 0. The relay answers SUBSCRIBE_OK, Track Alias 0, LARGEST_OBJECT {0, 0} |
| 3 | **FETCH, Request ID 2, Fetch Type 2 (Relative Joining), Joining Request ID 0, Joining Start 0** |
| 4 | The relay answers REQUEST_ERROR, code `0x3` (NOT_SUPPORTED), Retry Interval 0 |
| 5 | `mlmsub` logs `failed to retrieve catalog ... request rejected: NOT_SUPPORTED`, cancels the subscription (the relay sees a reset and STOP_SENDING with `0x1` on both request streams), asks for nothing else, and ends when its `-duration` is over. The relay closes the session then with "the peer closed its control stream" |

When the track had no Object yet, the SUBSCRIBE_OK carried no LARGEST_OBJECT and `mlmsub` sent no FETCH at all: `catalog subscription reported no content; cannot perform joining fetch`. `mlmsub` has `-catalog-mode subscribe`, which was not run.

**What this means for a FETCH in the own parts**, as facts: the one FETCH this peer sends is a Relative Joining FETCH with Joining Start 0 for a track with one Object; it is sent right after SUBSCRIBE_OK and only when that carried LARGEST_OBJECT; and a peer that is refused gives up the whole session's purpose, with no fallback to the Object the subscription delivers anyway.

Not run: `mlmsub` against `mlmpub` (both theirs); `mlmrel` with the own relay or an own publisher as its upstream (it dials and subscribes to all namespaces); the own browser client against `mlmpub`; audio, HEVC, AV1 and the CMAF namespaces; a FETCH from any own part; decoding the video.

## Limitations and blockers

- **Nothing was run for sections 1 to 7; section 8 was added a day later and is the one run.** Apart from it, whether any of these implementations exchanges an Object, a catalog or a FETCH with XmoQ is not known. That an implementation "has" a feature means identifiers and code paths were found, mostly by the research pass.
- **Only what is listed under "Checked a second time" was verified for this record.** The rest rests on the two research passes; their files say for each row where it comes from, and end with what they could not find.
- MSF-01 and LOC-04 were read in full by the research pass, the earlier LOC revisions only for dates, references and IDs, and transport drafts 18 and 19 only in the sections on FETCH, properties, caching and subscriptions.
- The drafts move: LOC changed its property numbers in three revisions in a row, and Joining FETCH is gone from transport draft 20. What is written here is the state of 2026-10-10.
- `spikes/msf-loc/` holds the research passes' files as they wrote them. They are outside lint and review.

## Open questions raised

For the project owner, and not answered here:

- Does following MSF include §6, a new stream for every Object, which the closest other implementation and LOC's own example do not do?
- Which LOC revision: LOC-04's numbers on draft 18, or what MSF-01 cites?
- Is a relay cache part of Phase 1, given that draft 18 does not require one and a publisher that answers a FETCH for its catalog meets MSF-01?
- Is moqlivemock to become an interop peer, and with it Go a tool of the repository?
- Does a moqtail relay pass a LOC-04 property it does not know unchanged, as draft 18 §2.5 asks? Not read, and it decides whether LOC Objects can go through the peer relay there is.
