# The Retry Interval of a REQUEST_ERROR

draft-ietf-moq-transport-18, §10.6.2: "Retry Interval: The minimum time (in milliseconds) before the request SHOULD be sent again, plus one. If the value is 0, the request SHOULD NOT be retried.", and "A Retry Interval value of 1 indicates the request can be retried immediately." The two implementations of this repository read the value in the same way and give the same one to a REQUEST_ERROR they make up themselves; the row "The Retry Interval of a REQUEST_ERROR" of [the implementation notes](../../docs/moqt-implementation-notes.md) has what they do with it. These rows pin the numbers.

No peer was a source for them: the relay and the libraries of moqtail 0.15.0 and moqtransport 0.14.0 send 0 in every REQUEST_ERROR they make.

## `reading.tsv`

What a Retry Interval that was received means. One row per value, five columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The Retry Interval, as it is on the wire |
| 3 | `yes` where the request may be sent again, `no` where it "SHOULD NOT be retried" |
| 4 | After how many milliseconds; `-` where it is not to be retried |
| 5 | The same as a wait a timer takes: no more than 2,147,483,647 ms, the longest delay a timer of a browser takes; `-` where it is not to be retried |

The value is a `vi64` of this draft, which carries every 64-bit number (§1.4.1), so the largest is 18,446,744,073,709,551,615.

## `made-up.tsv`

The Retry Interval of a REQUEST_ERROR that the session of either implementation makes up itself, by its Error Code. Three columns: the name of the code, the code, the Retry Interval.

The rule and its basis are in the row named above.

## What is not in the tables

- The relay's own values, which it knows more for: at its constant (row named above).
- A REQUEST_ERROR an application refuses a request with through `@xmoq/moqt`: the library gives it the value of this table for its code.
- What the caller does with the value. The libraries hand it on and do not wait by themselves.
