# The relay takes each Subgroup from one stream

- Date: 2026-10-10
- Commit SHA: `600a645687b1dc7db2c3af738c26cf0aa716e4f1` (the jj change `smuwyoqzxssx`). Compared with `main` at `4fc65f5bfe46`, whose relay core is the one of `63ce3bbfcbae`
- Roadmap item: Phase 0b, take one track from several publishers. This is the first of five steps: the relay still has one source per track

## Objective

A relay is a publisher towards its subscribers, and §2.2 has "Objects from the same Subgroup MUST NOT be sent on different streams, unless one of the streams was reset prematurely". With two sources of a track that sentence decides what the relay does with the second copy of a Subgroup, and it has to hold before a second source is taken. This step gives the relay the means: it knows which stream it took each Subgroup from, and what it does with another stream for the same Subgroup. With one source that changes three things, and is meant to change nothing else.

1. A stream whose header leaves the Subgroup ID to its first Object (§11.4.2, mode 0b01) is opened downstream when that Object comes, and not when the stream opens. One that ends without an Object leads to nothing downstream.
2. A second stream for a Subgroup from the same source: it passes when the first was cut short; when the first is still open, the one whose header the relay read later takes the place of the other; and it is stopped when the first ended with a FIN.
3. What still arrives on a stream the relay has stopped is dropped.

The reasons are in [implementation notes](../moqt-implementation-notes.md), section Relay.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Own relay and client | `xmoq-relay` and `xmoq-moqt-client` 0.0.0. The client has a new flag, `--subgroup-id zero|first-object|explicit`, for how its Subgroup Headers give the Subgroup ID |
| moqtail peers | moqtail-ts (`moqtail` npm 0.13.0) in Chromium 156.0.8078.4 through `playwright` 1.64.0; the moqtail Rust client (`moqtail` crate 0.15.0) |

Provisional settings: the relay remembers which stream a Subgroup came from for the 8 newest Groups of a track, and for 256 Subgroups a track.

## Commands

```sh
mise run check
mise run interop

# The new core next to the one on main, with random events of four worlds.
python3 spikes/relay-core-diff/compare.py main disk --world distinct --touching --spikes
python3 spikes/relay-core-diff/compare.py main disk --world v1 --touching
python3 spikes/relay-core-diff/compare.py main disk --world full --touching
python3 spikes/relay-core-diff/compare.py main disk --world split --touching
python3 spikes/relay-core-diff/compare.py main disk --world split --touching --raw-numbers  # has to fail
python3 spikes/relay-core-diff/compare.py --same disk --world full

# Streams whose header leaves the Subgroup ID to the first Object, with real peers.
R="node spikes/moqtail/browser/run.mjs --relay own --subgroup-id first-object"
$R --rust-publisher own --direction rust-to-rust --rust-subscriber own      # 1
$R --rust-publisher own --direction rust-to-rust --rust-subscriber moqtail  # 2
$R --rust-publisher own --direction rust-to-browser --subscribers 2         # 3
$R --direction browser-to-rust --rust-subscriber own                        # 4
$R --direction browser-to-rust --rust-subscriber moqtail                    # 5
$R --direction browser-to-browser --subscribers 2                           # 6
```

The worlds of the random events (`spikes/relay-core-diff/fuzz.rs`): in `distinct` no two streams of a session are of the same Subgroup and every header has Subgroup 0; `v1` has streams of the same Subgroup; `full` has those and the three ways a header gives the Subgroup ID. In `split`, sessions 1 and 2 have one track with the streams of `full`, and sessions 3 and 4 have another track in another namespace with the streams of `distinct`. Every world but `v1` also feeds three kinds of event the earlier comparisons did not (a control message, a session that failed on the I/O side, a SUBSCRIBE_OK with a Mandatory Track Property), starts each sequence with a track that is published and subscribed to, and has three in four of its data stream events be about that track, so that Objects reach a subscriber.

Two questions are asked of the sequences.

- **Do the versions differ before the change is touched?** (`--touching`) Once the two versions have parted in a sequence, they hold different things and what follows differs, so counting differences says little. What is counted is the sequences in which the versions differ before the first event the change is about: a stream that leaves the Subgroup ID to its first Object, or a stream for a Group and Subgroup that the same session opened a stream for before.
- **Does what follows reach anyone it should not?** (world `split`) Everything the change is about happens to sessions 1 and 2. Whatever the relay does to sessions 3 and 4 has to be the same in both versions for every event of every sequence.

For the second question the tool had to stop comparing streams to subscribers by their number. A version numbers them from one counter, so a stream more or less in one track shifts the number of every later stream in every track. They are compared under the session and how many streams were opened to it before, and an event that names one is passed to each version under the number that version gave it.

## Observed results

**Passed.**

### Nothing else changed

| World | Sequences | With an event the change is about | In which the versions differ | Differ before such an event | Streams opened to subscribers; Objects sent; sequences with an Object |
|---|---|---|---|---|---|
| `distinct` | 20,000 | 1,710 | 0 | 0 | 35,256; 5,170; 3,004 |
| `v1` | 20,000 | 13,475 | 1,764 | 0 | 9,612; 1,123; 781 |
| `full` | 20,000 | 19,401 | 7,968 | 0 | 24,676; 4,884; 2,898 |
| `split` | 20,000 | 15,948 | 5,127 | 0 | 33,268; 4,069; 2,807 |

The last column is what the new version did, to show how far the sequences get. In `distinct`, 1,710 sequences count as touching because a session that is opened again numbers its Groups from 0 again; the versions differ in none of them, so this row does not lean on what counts as touching. In the three worlds that have what the change is about, the versions differ in a part of the sequences, and in none before the change is touched.

**Sessions 3 and 4 of `split`:** 342,132 actions were for them, and the two versions differed in none of the 1,600,000 events in what they did to them, although they differed for sessions 1 and 2 in 5,127 of the sequences. With `--raw-numbers`, which compares the streams by number, the same run reports 2,900 events with a difference for sessions 3 and 4: the check can fail, and the names are what it takes to pass.

The 12 scenarios with real peers (routing by namespace, a PUBLISH that crosses the relay's SUBSCRIBE, a moqtail-ts publisher) ran against a relay that holds both versions: no difference in content, state, sequence or order, and the payload matched in all 12.

`--same`, world `full`: the digest of all actions was `c231e5c9a6f705f8` in each of three runs in processes of their own.

### The new behaviour

18 tests in `crates/relay-core/src/tests/subgroups.rs`, most as tables.

- **When a stream is opened downstream:** the three ways a header gives the Subgroup ID; a stream that ends without an Object, for each way and for a FIN and a reset; a stream that was held for its Track Alias; a subscriber that comes between the header and the first Object; a subscriber that joins inside a Subgroup, for each of the three ways.
- **A second stream:** the three cases; one that is recognised only when its first Object comes, while the first is open and after the first ended with a FIN; one that takes the place of another for a subscriber who was there from the start and one who joined later; one that took the place of another and ends without an Object.
- **What is remembered:** the 8 Groups (a second stream for a Group 7 Groups back is stopped, one 8 back passes); the 256 Subgroups, by behaviour and by the number of entries; a stream whose Subgroup was forgotten while it was open; a replacement in a full table.
- **What it costs:** a publisher that opens fifty streams for one Subgroup, and that nothing is left of them when it has gone; the publisher's Stream Count, which has the streams the relay stopped and the ones that named no Subgroup; a track that ends after it stopped a stream.
- **A known gap, pinned:** a subscriber whose stream was closed gets another for the same Subgroup.

### Streams whose Subgroup ID is the ID of the first Object, with real peers

| # | Publisher | Subscriber | Result |
|---|---|---|---|
| 1 | own Rust client | own Rust client | payload matched |
| 2 | own Rust client | moqtail Rust client | payload matched |
| 3 | own Rust client | two moqtail-ts pages | payload matched at both |
| 4 | moqtail-ts | own Rust client | payload matched |
| 5 | moqtail-ts | moqtail Rust client | payload matched |
| 6 | moqtail-ts | two moqtail-ts pages | payload matched at both |

In runs 1 and 4 the subscriber's trace shows the stream type `0x5a`, which has the mode 0b01, on the stream the relay sent. moqtail-ts sends that mode for an Object whose Subgroup ID is `null` (`getSubgroupHeaderType` in `dist/model.mjs` of the npm package, read on 2026-10-10); the page of these runs was given a switch for it.

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `45 of 45 pairings ended as expected`.

### A publisher that keeps opening streams for one Subgroup

Each new stream takes the place of the one before, which costs every subscriber of the track one reset and one new stream. Does that reach anyone else?

- **Streams open downstream:** one per subscriber at any time (the test with fifty streams). The streams of the publisher that were replaced stay in the relay's books until they are reported closed; how many a publisher has open at once is the QUIC limit the relay gives it (64).
- **Stream Count:** each subscriber's PUBLISH_DONE counts every stream it was sent (fifty in the test), as §10.11 asks.
- **`TOO_FAR_BEHIND`:** a subscriber that does not take the resets and new streams as fast as they come runs into the limit of streams that wait to be written (256 a subscriber session), and loses the subscription whose stream hit the limit. That limit is counted per subscriber session, over all its tracks, so a session that also subscribes to other tracks can lose one of those. **This is no more than a publisher can do today by opening a stream per Subgroup for many short Subgroups**: either way the cost for the subscribers is one stream per stream of the publisher, plus here one reset.
- **Subscribers of other tracks, on sessions that do not subscribe to this one:** nothing reaches them.

So no limit on the number of replacements was added. A publisher is in a position to end its track for everyone anyway.

### Why by Subgroup and not by Group

| | One stream per Subgroup (taken) | One source per Group |
|---|---|---|
| §2.2, no Subgroup on two streams downstream | met for what the relay remembers (8 Groups, 256 Subgroups; known gap beyond) | met for the Groups the relay remembers |
| When the relay can decide | at the header for two of the three ways of §11.4.2; at the first Object for the third | always at the header: the Group ID is in every header |
| When the stream downstream is opened | later than before for the third way only | as before |
| When a source vanishes mid-way | that Subgroup is cut; other Subgroups of the Group can come from another source | the rest of the Group is cut, unless another source is chosen for it |
| What is remembered | pairs of Group and Subgroup | Groups |

By Group is simpler. By Subgroup was taken because what is lost when a source vanishes is smaller, and because the one complication, the third way, is settled by the first Object. Starting by Group and refining later would have meant building the table and the later opening twice.

## Limitations and blockers

No blocker.

- **The three cases of a second stream were not run with a real peer.** No peer at hand opens a second stream for a Subgroup. They are covered by the tests of the core and by the random events.
- **The 8 Groups and 256 Subgroups are covered by tests of the core only.**
- **Objects past the end of a Subgroup, and a Subgroup on two streams that end with a FIN with different last Objects, are not detected** (§2.4.2, second and third example): a stream that is left or replaced is stopped unread. Known gap.
- **A second stream for a Subgroup the relay has forgotten passes, next to the first.** That is so 8 Groups back, and also inside the 8 Groups once a track has more than 256 Subgroups in them, where it can hit a Subgroup whose stream is still open. Known gap; a test pins it.
- **Of two open streams for a Subgroup, the relay keeps the one whose header it read later, which need not be the one that was opened later.** Known gap. Whether headers are read out of order with real peers was not looked at.
- **A subscriber whose stream for a Subgroup is closed gets a new one with the next Object.** This was so before this step. Where the stream closed because the subscriber stopped it, §11.4.3 has SHOULD NOT. Read in the code and pinned by a test of the core; not run with a peer that sends STOP_SENDING. Known gap.
- The random events of `v1`, `full` and `split` differ between the versions in a part of the sequences, by design. That those differences are the intended ones is shown for their first event in each sequence only. What follows was not sorted; what is shown about it is that it stays with the sessions of the track it began in, and only for sessions that share no track with them. **A session that subscribes to the touched track and to another one is not covered by that check.**
- In the comparison, a stream to a subscriber that only the old version has is never named by a later event.
- The comparison with `main` covers the core. The hub was not changed.
- One Object per run in the runs with real peers, loopback, one machine, Chromium only.

## Open questions raised

- Whether a Subgroup that was cut short is continued from another source: in [open questions](../open-questions.md), for Phase 2.
