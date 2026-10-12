# How long a request waits for its answer

draft-ietf-moq-transport-18 does not say how long an endpoint waits for the answer to a request it sent. §13.6 has "Each implementation is expected to set its own timeouts", and §3.5 has a code for closing the session over it (`CONTROL_MESSAGE_TIMEOUT`, "the peer took too long to respond to a control message"). The two implementations of this repository wait the same time and do the same when it is over; the row "A request the peer does not answer" of [the implementation notes](../../docs/moqt-implementation-notes.md) has what they do. These rows pin the time.

The rule: **10 seconds** (provisional; the time the peer's SETUP gets), **and as long again as the request itself tells a relay to wait**: the RENDEZVOUS_TIMEOUT of a SUBSCRIBE (§10.2.6, "The relay does not send SUBSCRIBE_OK until a publisher becomes available") or the FILL_TIMEOUT of a FETCH (§10.2.5, "the maximum total duration in milliseconds a relay SHOULD spend waiting for upstream sources"). No more than 2,147,483,647 ms, the longest delay a timer of a browser takes. No other parameter changes the time.

The time runs from when the request is sent to its answer: SUBSCRIBE_OK, PUBLISH_OK, FETCH_OK, REQUEST_OK or REQUEST_ERROR.

## Format

`waits.tsv`, one row per request, five columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The request, by the name of its message. For the reader: the rule does not go by it |
| 3 | The one Message Parameter the request carries, by its name in §10.2; `-` for none |
| 4 | The value of that parameter, in milliseconds; `-` for none |
| 5 | How long the answer is waited for, in milliseconds, where the time without a parameter is 10,000 |

## What is not in the table

- What an endpoint does when the time is over, and that the session stays open: the implementation notes, and the interop run (`mise run interop`), where a publisher that answers late is given up by the page and by the Rust client.
- A FETCH without a FILL_TIMEOUT is waited 10 seconds for, though "the relay waits for an implementation specific duration" (§10.2.5) the requester cannot know. The relay of this repository takes 5 seconds.
