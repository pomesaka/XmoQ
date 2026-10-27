# Whether the SUBSCRIBE of a Joining FETCH is yet to come

A Joining FETCH names the Request ID of a SUBSCRIBE (§10.12.2). The two travel on streams of their own, so the endpoint that receives them can read the FETCH first, and then knows no request with that ID. Both implementations tell an ID whose request is yet to come from one that names nothing, by the same rule, and these rows pin the rule: the Request IDs the peer has used, the ID of the FETCH, the ID it names, and whether the FETCH is held or refused.

The rule: the request is yet to come when the ID is one of the peer's (even from a client, odd from a server, §10.1), smaller than the ID of the FETCH itself, and not used. §10.1 has "Each endpoint increments its Request ID by 2 for each new request", so a request that names another was made after it.

## Format

`held.tsv`, one row per Joining FETCH, six columns separated by tabs, no header. Every row ends with one line feed, the last one too; there are no empty rows.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | `client` or `server`: who sent the requests. The receiver is the other |
| 3 | The Request IDs the peer used before the FETCH, in the order they arrived, separated by commas; `-` for none. Requests arrive in any order (they travel on streams of their own), so the list need not grow. An ID with `:gone` after it is one whose request is over before the FETCH arrives |
| 4 | The Request ID of the Joining FETCH |
| 5 | Its Joining Request ID |
| 6 | `held`: the request with that ID is yet to come, and the FETCH waits for it. `refused`: REQUEST_ERROR `INVALID_JOINING_REQUEST_ID` (`0x32`) at once, and the session stays open |

## What a test has to set up

- For every ID of column 3, the endpoint received a request with that ID that is no subscription. A Joining Request ID among them is therefore refused. A Joining Request ID that names a SUBSCRIBE which is Pending or Established is neither held in this sense nor refused, and is not in the table.
- For an ID with `:gone`, the peer then reset the stream of that request, before the FETCH. WHY this way of being over and no other: in both implementations the request is gone the moment the reset is known, whatever the application had made of it. A request that was answered for good and whose stream the peer has not ended may still be held by an implementation, and is not what `:gone` means. What the rows with `:gone` pin is that an ID stays used when nothing is held for it any more: an implementation that asks only whether a request with that ID is open holds such a FETCH.
- What kind of request it is, and whether the endpoint keeps it open or refuses it at once by itself, is for each test to choose: the Rust session is handed a PUBLISH_NAMESPACE and keeps it open; `@xmoq/moqt` refuses one itself.
- The FETCH is a Relative Joining FETCH with Joining Start 0; neither matters to the rule.

## What is not in the table

- What becomes of a FETCH that is held: the code that holds it (`crates/moqt/src/session/peer.rs`, `packages/moqt/src/core/peer-requests.ts`), and at the relay `a_joining_fetch_whose_subscribe_never_comes_is_refused_after_the_time_for_first_bytes` of `crates/relay/src/tests.rs`.
- An implementation remembers only so many IDs the peer used out of order (`request-ids-ahead` of [`fixtures/limits/`](../limits/README.md)). One that it forgot reads as yet to come, and the FETCH that names it is refused when the time runs out instead of at once.

## Source

**The table is a decision, not an observation.** The draft says nothing of a Joining FETCH that is read before its SUBSCRIBE; no other implementation was asked. The rows were written by hand for the rule above.

`@xmoq/moqt` is a client, so it reads the rows in which the peer is the `server`; those rows repeat the cases of the `client` rows that the rule has, with IDs up to 63. It serves no Joining FETCH: where a row has `held`, it refuses the FETCH at once with `NOT_SUPPORTED` (`0x3`), as the Rust client does with a FETCH its session hands it, and where a row has `refused`, with `0x32`.
