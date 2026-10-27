# What a receiver of draft 18 does with bytes it takes or refuses

The counterpart of [`fixtures/moqt-18-by-hand/`](../moqt-18-by-hand/README.md), which holds bytes that are taken: bytes a peer may send, and what the Rust (`crates/moqt`) and the TypeScript (`packages/moqt`) implementation do with them. Both test suites read every file here. The bytes were written by hand from the text of `draft-ietf-moq-transport-18`; the verdicts are the decisions of both implementations, and why each is decided so is at the test that reads the file and at the code.

Every file has one row per case, columns separated by tabs, no header, and every row ends with one line feed. Column 1 is the name of the row, unique in its file. Bytes are hexadecimal digits, lower case, with no spaces. Variable-length integers are those of §1.4.1.

A verdict is one of:

| Verdict | What it means |
|---|---|
| `ok` | Taken; the session and the stream go on |
| `closed:<code>` | The session is closed with that Session Termination Error Code (§3.5) |
| `malformed` | The track is malformed (§2.4.2): its subscription is cancelled, the session stays |
| `stopped` | The stream alone is ended, for a limit of the receiver; the session stays |
| `dropped` | The datagram is dropped; the session stays |
| `taken` | The GOAWAY is taken; the session stays |
| `control`, `subgroup`, `fetch`, `padding` | What a unidirectional stream is taken for (§3.4) |

## `control.tsv`

Three columns: the name, one control message, its verdict (`ok` or `closed:<code>`). The message is decoded alone, as the first and only message of what it is read from.

## `goaway.tsv`

Five columns: the name; the role of the receiver, `client` or `server`; where the GOAWAYs come, `control` (the control stream, after SETUP) or `own-request` (the stream of a SUBSCRIBE of the receiver, Pending); the GOAWAYs, fed in turn; and what they lead to, `taken` or `closed:<code>`. The TypeScript session is a client, and reads the rows of a client alone.

## `datagrams.tsv`

Three columns: the name, one datagram (empty for an empty one), its verdict (`dropped` or `closed:<code>`).

## `streams.tsv`

Three columns: the name, the first bytes of a unidirectional stream (its Stream Type, and what may follow), and what the stream is taken for, or `closed:<code>`.

## `fetch-stream.tsv`

Four columns: the name; the Group Order of the FETCH the stream answers, `ascending` or `descending`; the whole stream, ended with a FIN after its last byte; its verdict. A reader takes Objects of up to 1,024 bytes of payload and 1,024 bytes of properties.

## `subgroup-stream.tsv`

Three columns: the name, the whole stream, ended with a FIN after its last byte, and its verdict. The two implementations bound an Object differently ([`fixtures/limits/`](../limits/README.md)); the one row that is `stopped` is over the limit of both.

## `reset-codes.tsv`

Two columns: why a stream is reset or stopped (`malformed-track`, `local-limit`, `cancelled`), and the Stream Reset Error Code (§3.3.3) it is ended with.
