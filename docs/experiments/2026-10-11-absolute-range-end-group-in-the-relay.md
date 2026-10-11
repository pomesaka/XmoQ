# The relay keeps to the End Group of an AbsoluteRange filter

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: none of its own. Phase 1, Rust relay fan-out to browser viewers: which Objects a subscriber gets (§5.1.2, a MUST NOT for every publisher)

## Objective

The relay kept to the Start Location of a Subscription Filter and sent a subscriber with an `AbsoluteRange` filter what came after its End Group as well (§5.1.2: "A publisher MUST NOT send objects from outside the requested range"). It now sends nothing of a Group after the End Group. Run such a subscription through the own relay against Groups before, in and after its range, and compare the relay core before and after the change with the tool that feeds both the same events.

The End Group is the Group of the Start Location plus the End Group Delta, and passes whole (§5.1.2: "strictly less than or equal to the End Group"; "If the specified End Group Delta is zero, the remainder of that Group passes the filter").

## Environment

The own relay and the Rust client built from the working copy this record is part of, on both ends. Loopback, port 48600, macOS. The runs are those of [the record of the Start Location](2026-10-11-subscription-filter-in-the-relay.md), with another filter.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"

# P1, at 1 s: Groups 5 to 8 from 3 s on, and Groups 9 and 10 at about 11 s.
$C --mode publish --payload-hex 6162 --first-group 5 --answer-fetch
# The subscriber, at 5.5 s.
$C --mode subscribe --objects 9 --timeout-secs 11 --subscription-filter <filter>
# P2, at 7 s: Groups 0 to 2 from 8.5 s on.
$C --mode publish --payload-hex 6162

T="python3 spikes/relay-core-diff/compare.py"
$T @- disk --world filters --kinds --touch filters     # also split-filters, shared-filters, full, fetch
```

`--subscription-filter absolute-range:<group>,<object>,<end group>` is new. It takes the End Group, and the client sends the delta to the Group of the Start Location.

## Observed results

**The subscriber got the Groups of its range and none after the End Group.** Group 0 reaches no subscriber in these runs, for a reason that is not the filter (the record of the Start Location).

| Filter | Range | Groups on the subscription | Subgroup streams the relay opened for it |
|---|---|---|---|
| `absolute-range:1,0,9` | {1, 0} to the end of Group 9 | 1, 2, 9 | 3 |
| `absolute-range:9,0,9` | Group 9 (delta 0) | 9 | 1 |
| `absolute-range:1,0,1` | Group 1 (delta 0) | 1 | 1 |
| `absolute-start:1,0` | from {1, 0} | 1, 2, 9, 10 | 4 |

In the third run the End Group was over when Groups 9 and 10 came; the subscription stayed open and got nothing more. One run each; the first was made twice, with the same Groups.

**The cores before and after differ only in sending less, and only in the worlds with filters.** 20,000 random sequences of 80 events a world; "before" is the change this one is built on.

| World | SUBSCRIBEs with an `AbsoluteRange` filter | Sequences that differ, of 20,000 | Bystanders treated differently |
|---|---|---|---|
| `filters` | 12,159 | 110 | - |
| `split-filters` | 6,029 | 62 | 0 of 331,775 actions |
| `shared-filters` | 6,029 | 61 | 0 of 295,447 actions |
| `full` | none | 0 | - |
| `fetch` | none | 0 | - |

In every sequence that differs, the first difference is that the new core opens fewer streams to subscribers or sends fewer Objects, and does everything else of that step as the old one did: at `DataStreamOpened` (199 sequences over the three worlds), at `Object` (29) and at a PUBLISH that brings streams the relay held for it (5). In 1 of them the old core also reset, in the same step, a stream the new one did not open. In no sequence do the versions differ before a SUBSCRIBE with a filter. The filters the tool draws have an End Group Delta of 0 or 1.

## Limitations and blockers

- **The tool does not say that the stream or the Object the new core left out was one for a subscriber with an `AbsoluteRange` filter.** It says which kind of action is missing in the first step that differs; that the other three filters and an unfiltered subscription are untouched is what the tests of the relay core show, whose rows for them did not change.
- "The first difference" is all the comparison says of a sequence; see [the record of that tool](2026-10-11-relay-core-diff-with-fetch.md).
- A SUBSCRIBE whose End Group is over already is accepted, and a subscription is not ended when its End Group has passed (implementation notes, known gaps). Neither was changed.
- The runs have one Object a Group: a subscription that ends inside what a stream carries does not exist, since an End Group ends with a Group. A Group of the range that comes after a later one is a row of the table test of the relay core, and Groups 1 and 2 of the first run, which came after Group 8.
- Another process ran an interop on other ports of the machine during the four runs.
- No other implementation was asked what it does with an End Group. `@xmoq/moqt` and the Rust client as publishers look at no filter (known gaps).
