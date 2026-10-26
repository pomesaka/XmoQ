# A REQUEST_ERROR with a Redirect

What a client makes of a REQUEST_ERROR that answers one of its requests, with a Redirect or without (§10.6.1, §10.6.2): it takes the refusal, or closes the session. Both sessions read `answers.tsv` in their tests.

## `answers.tsv`

One row a line, four fields separated by tabs, no header. Every row ends with one line feed; there are no empty rows.

| Field | Meaning |
|---|---|
| name | What the row is about; unique |
| request | The request of the client the REQUEST_ERROR answers: `subscribe` (track `xmoq`/`a`), `publish-namespace` or `subscribe-namespace` (namespace `xmoq`) |
| bytes | The REQUEST_ERROR, whole, in hex |
| outcome | `rejected`: the client takes the refusal and the session stays open. `closed 0x3`: the session is closed with `PROTOCOL_VIOLATION` |

The REQUEST_ERRORs of the rows, written by hand from §10.6.2: Error Code `DOES_NOT_EXIST` (`0x10`) without a Redirect (`none`), or `REDIRECT` (`0x34`) with a Redirect of an empty Connect URI, Track Namespace and Track Name (`empty`), of the Track Name `b` alone (`track-name`), of the Track Namespace `a` alone (`namespace`), or of the Connect URI `h` alone (`connect-uri`). Each has a Retry Interval of 0 and no Error Reason.

What the rows hold a reader to: §10.6.1, "Track Name is not meaningful for namespace-scoped requests (SUBSCRIBE_NAMESPACE, PUBLISH_NAMESPACE) and MUST be empty; an endpoint that receives a non-empty Track Name in a Redirect for a namespace-scoped request MUST close the session with a PROTOCOL_VIOLATION". The Connect URI is the client's to follow or not.

**Not in the table**: a Redirect with a Connect URI to a server, which closes the session (§10.6.1: "If a server receives a Redirect with a non-zero Connect URI Length it MUST close the session with a PROTOCOL_VIOLATION"). `@xmoq/moqt` has no server role; the Rust session has a test of its own (`a_server_is_not_redirected_to_another_uri`).
