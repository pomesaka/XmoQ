# How long a request waits for its answer

draft-ietf-moq-transport-18 does not say how long an endpoint waits for the answer to a request it sent. §13.6 has "Each implementation is expected to set its own timeouts", and §3.5 has a code for closing the session over it (`CONTROL_MESSAGE_TIMEOUT`, "the peer took too long to respond to a control message"). The two implementations of this repository wait the same time and do the same when it is over; the row "A request the peer does not answer" of [the implementation notes](../../docs/moqt-implementation-notes.md) has what they do. These rows pin the time.

The rule: **`answer-wait-ms` of [`fixtures/limits/`](../limits/README.md)** (provisional, with its reason at the constant), **and as long again as the request itself tells a relay to wait**: the RENDEZVOUS_TIMEOUT of a SUBSCRIBE (§10.2.6, "The relay does not send SUBSCRIBE_OK until a publisher becomes available") or the FILL_TIMEOUT of a FETCH (§10.2.5, "the maximum total duration in milliseconds a relay SHOULD spend waiting for upstream sources"). No more than 2,147,483,647 ms, the longest delay a timer of a browser takes. No other parameter changes the time.

The time runs from when the request is written to its answer: SUBSCRIBE_OK, PUBLISH_OK, FETCH_OK, REQUEST_OK or REQUEST_ERROR.

## Format

`waits.tsv`, one row per request, five columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The request, by the name of its message. For the reader: the rule does not go by it |
| 3 | The one Message Parameter the request carries, by its name in §10.2; `-` for none |
| 4 | The value of that parameter, in milliseconds; `-` for none |
| 5 | How long the answer is waited for, in milliseconds, where the time without a parameter is that of `answer-wait-ms` |

## `publish-done-of-an-unanswered-publish.hex`

What ends a PUBLISH nobody answered in that time: a PUBLISH_DONE with the Status Code `INTERNAL_ERROR` (`0x0`), Stream Count 0 and the Error Reason `no answer in time`, written by hand from §10.11 and sent by both implementations, which then finish their direction of the request stream. Every other request is reset. Bytes in hexadecimal, separated by blanks.

## What is not in the table

- What else an endpoint does when the time is over, and that the session stays open: the row named above.
