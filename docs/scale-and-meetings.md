# Scaling a broadcast over relays, and meetings — design draft (proposed)

Everything here is **proposed**: nothing of it is built, and every choice is undecided until the project owner decides it ([open decisions](open-questions.md)). What the own relay does today was read in the code of `main` on 2026-10-25 (`bdad71ce`), and the file is named where it matters.

Decided on 2026-10-25, the inputs of this draft ([requirements](requirements.md)):

- One relay serves 500 viewers of a broadcast, and relays are added to build a broadcast of 10,000.
- Meetings, many to many, of 49 participants at most, after an agent can follow a broadcast.
- An agent takes part through a CLI; it gets speech as text, transcribed with whisper.cpp on its machine, and speaks as chat lines.

## Part 1: a broadcast of 10,000

### What one relay does today

- With a video track and a track of the shape of the sound (an Object a stream, 50 a second), one relay on Linux in a VM of 5 cores gave everything to 500 viewers and fell seconds behind at 1,000; with video alone it gave everything to 1,000 ([record](experiments/2026-10-24-one-video-track-to-many-viewers-on-linux.md)). The sound costs more than its bytes: at 500 viewers it is 25,000 streams opened and closed a second.
- The relay takes 256 sessions at once unless `--max-sessions` says otherwise (`HubConfig::max_sessions` in `crates/relay/src/hub.rs`), and closes one more at once with `INTERNAL_ERROR` and the reason `too many sessions`. A page of the demo is two sessions, one for the media and one for the chat (`@xmoq/chat` connects on its own), so 500 viewers are 1,000 sessions.
- A relay sends GOAWAY with a New Session URI (`--redirect-uri`) only when it is told to stop. The page (`keepPlaying`, `keepPublishing`) and the terminal viewer follow it to the URI.

### Shapes

| Shape | What it is | For 10,000 | Against |
| --- | --- | --- | --- |
| A. Relays side by side, the publisher sends to each | The publisher connects to every relay | 20 relays | A browser publisher sends its video 20 times; its uplink is the limit |
| B. A tree of two levels | The publisher sends to an origin relay; edge relays subscribe to the origin; viewers connect to edges | 20 edges of 500 and one origin with 20 sessions | A hop more for every Object; the origin is one machine every Object passes |
| C. B, with more levels | Edges subscribe to mid relays, which subscribe to the origin | Not needed below 500 × 500 = 250,000 | A hop more again |

**Recommended: B.** The edges are the relays side by side, and the origin feeds them; the publisher's uplink stays one copy. The origin serves no viewers, so its load is 20 sessions, whatever the audience. A relay that is to be an edge needs nothing a relay does not do already for viewers: it keeps the latest Group of each track and answers a Joining FETCH from it (row on the relay's cache in the [implementation notes](moqt-implementation-notes.md)), so the late joiners of an edge do not reach the origin. What a second hop adds to the delay was not measured, and the goal of 500 ms at the 99th percentile counts from the publisher to the viewer.

### How an edge learns where the tracks are

| Choice | How | For | Against |
| --- | --- | --- | --- |
| 1. A configured upstream | The edge is started with the origin's URL and a namespace prefix (`xmoq`), and treats the origin as a publisher that announced it | The draft allows it: "Given sufficient out of band information, it is valid for a subscriber to send a SUBSCRIBE or FETCH message to a publisher (including a relay) without any previous MOQT messages besides SETUP" (§6). A viewer that comes before the broadcast is held at the origin (RENDEZVOUS_TIMEOUT), as now | Every SUBSCRIBE of a track nobody publishes goes to the origin |
| 2. SUBSCRIBE_NAMESPACE to the origin | The edge asks the origin for the namespaces under a prefix, and takes each NAMESPACE as an announcement and each NAMESPACE_DONE as its withdrawal | The edge knows what exists; the origin already serves SUBSCRIBE_NAMESPACE | The relay does not read NAMESPACE as a subscriber; more state to keep right |
| 3. SUBSCRIBE_TRACKS to the origin | The origin sends a PUBLISH for every track to every edge (§6.1) | Nothing to ask | Every track goes to every edge, watched or not; neither relay implements SUBSCRIBE_TRACKS |

**Recommended: 1**, as the smallest step; 2 when there is more than one origin.

### The other direction: what viewers send

A viewer that chats announces `xmoq/chat/<room>/<name>` and says its lines on `said`, and the gatherer finds them with SUBSCRIBE_NAMESPACE at its relay ([architecture](architecture.md)). With edges the gatherer sits at the origin and the participants announce at the edges, where it does not see them. §9.5 has "Relays MUST forward PUBLISH_NAMESPACE or PUBLISH messages to all matching subscribers". **Recommended: the edge sends the PUBLISH_NAMESPACE of its sessions on to its upstream**, so that the gatherer is told of it and its SUBSCRIBE comes back down through the edge. The same carries an agent's lines and, later, a meeting's participants on different edges. Whether 10,000 viewers may all write in the chat is a separate question: one session of a gatherer at the own relay holds about 250 participants, and the gatherers are split by room for that (Q48, [architecture](architecture.md)).

### Who sends a viewer to which relay

| Choice | How | For | Against |
| --- | --- | --- | --- |
| a. The room | The room already tells a viewer where the broadcast is; it gives each viewer an edge with room, counting what it handed out | Nothing new between relay and room | The room's count is what it gave out, not what the edge holds |
| b. The relays report to the room | Each edge tells the room how many sessions it has | The count is real | The relay and the room do not talk today; that link waits for authorization and hosting ([open decisions](open-questions.md)) |
| c. A load balancer in front of the edges | DNS or anycast | Nothing in the room | The pages pin a certificate hash (`serverCertificateHashes`); each edge would need the same certificate or a CA-signed one, which waits for hosting |

**Recommended: a, with an edge that turns away the ones beyond its share** (next section), and b once relay and room talk.

### When a relay is full

Today a relay takes every session up to 256 (or `--max-sessions`) and closes the next. Before it gets there with too many viewers, it falls behind for all of them: at 1,000 with sound, every viewer was seconds late.

**Recommended**: an edge takes as many sessions as it was measured to serve (1,000 sessions, 500 viewers, on that VM), and answers one more with GOAWAY naming another edge rather than closing it, so that the clients that follow GOAWAY move by themselves; the room is asked again where the edge knows no other. Who starts another edge when all are full is a matter of hosting, which is undecided.

### What the own relay has and lacks to be an edge

Built: choice 1 above, with the session opened when a request needs it and moved at a GOAWAY of the upstream, and the namespaces announced below passed up (`--upstream`, `--upstream-cert-sha256`, `--upstream-namespace`; the row on a relay above this one in the [implementation notes](moqt-implementation-notes.md)). The relay is the client of that session, and asks the upstream for a SUBSCRIBE, FETCH or TRACK_STATUS of a track that no session of its own announced. Lacking, read in the code of `main` on 2026-10-25:

3. **Choice 2.** It sends no SUBSCRIBE_NAMESPACE upstream and does not read NAMESPACE; `discovery.rs` only sends them.
4. **PUBLISH upstream.** The PUBLISH_NAMESPACE of its sessions is passed upstream (the other direction above, built: the chat's gatherer above hears participants below); PUBLISH is not, since neither relay serves SUBSCRIBE_TRACKS.
5. **The origin limits an edge as one viewer.** What a slow subscriber is counted by is per session, over all its tracks (4 MiB, 4,096 Objects, 256 streams), and an edge that falls behind on one track has that subscription ended (`TOO_FAR_BEHIND`), for all its viewers at once. A session has 16 FETCHes at once. An edge needs other limits at the origin, given by its token or by configuration.
6. **Its own token.** The relay checks a token in every request with a key it shares with the room. A token for the namespace `*` allows every namespace (`crates/room-token/src/lib.rs`), so an edge given the key can make its own, as the gatherer does; the viewers' tokens are checked at the edge and not passed on (§10.2.1).
7. **Losing the origin.** When the origin's session goes, the edge ends the tracks of its viewers as for any source that goes (PUBLISH_DONE), and the clients ask again, and the edge connects again for the next request. A GOAWAY of the origin has the edge connect to the URI it names and take what it carries from there as well, so that its viewers go on (built).

Not lacking: the cache, FETCH, priorities, delivery timeouts and RENDEZVOUS_TIMEOUT work at an edge as at any relay (a held SUBSCRIBE is held at the origin as well), and the origin serves an edge as it serves any subscriber.

### To measure

Measured once, on one machine over loopback ([record](experiments/2026-10-25-one-video-track-through-two-edges.md)): an origin with two edges of up to 250 viewers each, where the origin sent each Group once to each edge and the frames came sooner than with the same viewers at the origin alone. Still to measure: the delay a second hop adds on its own machine; an origin with 20 edges; a page with both tracks on one session, which the record of 500 viewers did not have (each viewer there was two sessions of the Rust client); and a run of 10,000, which takes about 21 machines or a scaled-down run with fewer viewers an edge.

## Part 2: meetings of 49

### The sizes

At 2.5 Mbit/s a participant, a participant that receives everybody else gets 48 × 2.5 = 120 Mbit/s, and the relay sends 49 × 120 ≈ 5.9 Gbit/s. Each participant has to receive less than everybody.

Sound is Opus, a piece every 20 ms, each an Object on a stream of its own (decided on 2026-10-13). If all 49 send all the time, each receives 48 × 50 = 2,400 streams a second and the relay opens 117,600 a second, nearly five times the 25,000 with which the relay's processor went from 24% to 59% at the mean in the record above.

### Finding each other

| Choice | How | For | Against |
| --- | --- | --- | --- |
| A. SUBSCRIBE_NAMESPACE | Each participant announces `xmoq/meet/<room>/<participant>` and publishes a catalog there; each subscribes to the namespaces under `xmoq/meet/<room>` and is told NAMESPACE when one comes and NAMESPACE_DONE when one goes | The relay serves it (8 a session, 4,096 namespaces under a prefix, row on SUBSCRIBE_NAMESPACE in the [implementation notes](moqt-implementation-notes.md)); the relay itself says who left; as the chat does | `@xmoq/moqt` decodes NAMESPACE and NAMESPACE_DONE and sends no SUBSCRIBE_NAMESPACE |
| B. The room lists them | The room's WebSocket says who is there | Nothing new in the browser library | The room believes the pages; a participant whose connection to the relay died stays listed; against the direction of carrying everything over QUIC (the chat's decision of 2026-10-24) |

**Recommended: A.** The namespace prefix is the meeting's own, apart from `xmoq/chat`, and the meeting's chat is the room's chat as it is.

### Which video each one receives

| Choice | How | For | Against |
| --- | --- | --- | --- |
| A. Everybody, full size | — | — | 120 Mbit/s a participant |
| B. Versions chosen by the subscriber | Each participant publishes two or three versions (as the trial `?ladder=1`: 640x360 and 320x180, the small one about 0.5 to 0.6 Mbit/s as encoded, [record](experiments/2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md)); a participant subscribes to the large version of who speaks and the small one of the tiles it shows, and to nothing of the others | The relay stays as it is; what one receives is what it shows | Changing who speaks needs a key frame of the new version |
| C. The relay chooses | The relay forwards versions by who speaks | — | The relay reads no media, and would have to |
| D. One picture made by a server | A server decodes, composes and encodes | One stream a participant | A decoder and encoder for 49, and the delay of both |

**Recommended: B.** With one large version and 8 small tiles a participant receives about 2.5 + 8 × 0.6 ≈ 7.3 Mbit/s; showing all 48 at once needs a smaller version still, whose bitrate was not measured. Details:

- The relay subscribes to a version only while somebody wants it, where the participant announces its namespace instead of sending PUBLISH, and cancels its SUBSCRIBE when the last subscriber leaves (row on the last subscriber of a track in the [implementation notes](moqt-implementation-notes.md)). So a version nobody shows is neither sent nor, if the sender stops encoding while it has no subscription, encoded.
- On a change of speaker the participant subscribes to the new large version and keeps the small one until the large one has a key frame. It can ask for one with NEW_GROUP_REQUEST, which the relay passes on and the video sender of `@xmoq/media` answers with `dynamicGroups` (a new Group once the current one is 500 ms long).
- FORWARD cannot be switched on a subscription the relay holds: it refuses a REQUEST_UPDATE with anything but delivery timeouts and NEW_GROUP_REQUEST. Changing a version is a SUBSCRIBE and a cancel.

### Who mixes the sound

| Choice | How | For | Against |
| --- | --- | --- | --- |
| A. Nobody: each participant plays the tracks it receives | As the demo plays one | Nothing new beside the relay; no delay added; **an agent gets each speaker on a track of their own, so whisper.cpp transcribes each person apart, with no need to tell voices apart** | The receiver gets as many tracks as speak at once |
| B. A mixer beside the relay | Like the gatherer of the chat: it subscribes to all and publishes a mix | One track for each listener | Each listener needs a mix without its own voice, so 49 mixes; a decode and an encode of delay; an agent gets one mix and has to tell the voices apart |
| C. A then B | A, and a mixer for the listeners that cannot take many tracks | — | Two ways to keep |

**Recommended: A, with only speech sent.** A participant sends no sound while it does not speak (a voice activity detector at the sender, or muted), so a participant receives the few that speak; at two at once that is 2 × 50 streams a second, and the relay's 117,600 become about 4,900. Who speaks is found by each participant from the sound it decodes; an audio level in an Object property, as RTP has, is the next step if decoding all of them costs too much. How Chromium's echo cancellation works with several tracks played at once was not looked at.

### What an agent subscribes to in a meeting

**Recommended**: the namespaces of the meeting (who is there), the sound of each participant (transcribed by person, named by their namespace), the chat's `lines`, and the catalog of each. No video unless asked, and then the small version's key frames alone, written as pictures. The agent announces its own namespace under the meeting, so it is listed like a participant, and says its lines on `said` as any participant. How many speakers whisper.cpp keeps up with at once on one machine was not measured. How the people present learn that an agent is there and transcribes is [open](open-questions.md): listing it is the least; whether they have to agree is the project owner's to decide.

### Who may enter

The room of the spike gives a token to anyone, and who may get one is undecided for the broadcast as well ([open decisions](open-questions.md)).

| Choice | How | For | Against |
| --- | --- | --- | --- |
| a. Whoever has the meeting's address | An address nobody can guess; the room gives tokens to whoever asks | Nothing new | Whoever gets the address gets in |
| b. A list of invited people | The room checks who asks | — | Needs authentication, which is undecided |
| c. A host admits | The room holds who asks until the host lets them in | — | A host's page and a waiting state in the room |

**Recommended: a to begin with**, with the tokens it already makes: one to publish on the participant's own namespace, `xmoq/meet/<room>/<participant>`, and one to subscribe to `xmoq/meet/<room>/*` (a `*` at the end allows the namespaces under it, decided on 2026-10-24), and the chat's two. b or c when authentication is decided. An agent is brought in by a participant, who gets a token for it from the room, so that somebody present answers for it.

### The delay

The goal of 500 ms at the 99th percentile is a broadcast's. A conversation needs less: ITU-T G.114 is often cited as 150 ms one way being good and 400 ms the most for planning (not read for this draft). The playout buffer's default of 150 ms alone is most of the first. **Recommended**: measure mouth to ear on one machine with a buffer of 0 and 50 ms before setting a goal for meetings.

### Limits of the relay a meeting reaches

- Sessions: 49 participants are about 98, within the default of 256.
- What a slow subscriber is counted by: per session over all its tracks. A participant's session holds about 50 tracks (sound, catalogs, a few videos), and one video that falls behind can end the subscription of a sound track. This is the [open decision](open-questions.md) on counting per subscription, and a meeting is where it matters.
- Requests: about 48 catalogs, up to 48 sound tracks and up to 9 videos, about 105 at once, with more while a speaker changes. The relay reads 264 request streams of a session at once; what `@xmoq/moqt` allows itself to open at once was not checked for this draft.
- A meeting on one relay is recommended to begin with: across edges it needs the forwarding of PUBLISH_NAMESPACE of Part 1.
