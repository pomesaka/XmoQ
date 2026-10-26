# Where a parameter may appear

Each Message Parameter of draft 18 and the messages it may appear in (§10.2.1: "If it appears in some other type of message, the receiving endpoint MUST close the connection with a PROTOCOL_VIOLATION"), as both implementations check them. `xmoq-moqt` reads `scopes.tsv` in a test of `crates/moqt/src/params.rs` (`allowed_in`); `@xmoq/moqt` does not read it yet. Written by hand from §10.2.2 to §10.2.14.

## `scopes.tsv`

One row a parameter, three columns separated by tabs, no header. Every row ends with one line feed.

| Column | What it is |
|---|---|
| 1 | The parameter, by its name in the draft |
| 2 | Its Parameter Type, in hex |
| 3 | Every message it may appear in, separated by commas; any message of the list below that is not named is one it may not appear in |

The messages, by the names the implementations give them:

| Name | The message |
|---|---|
| `SUBSCRIBE`, `SUBSCRIBE_OK`, `PUBLISH`, `PUBLISH_NAMESPACE`, `FETCH`, `FETCH_OK`, `SUBSCRIBE_NAMESPACE`, `TRACK_STATUS` | As in the draft |
| `PUBLISH_OK`, `PUBLISH_NAMESPACE_OK`, `TRACK_STATUS_OK`, `SUBSCRIBE_NAMESPACE_OK`, `REQUEST_UPDATE_OK` | The `REQUEST_OK` answering PUBLISH, `PUBLISH_NAMESPACE`, `TRACK_STATUS`, `SUBSCRIBE_NAMESPACE`, `REQUEST_UPDATE` |
| `REQUEST_UPDATE` | A `REQUEST_UPDATE` of a subscription |
| `REQUEST_UPDATE_OF_FETCH`, `REQUEST_UPDATE_OF_SUBSCRIBE_NAMESPACE` | A `REQUEST_UPDATE` of a FETCH, of a `SUBSCRIBE_NAMESPACE` |

How the draft's words are read, where they are not plain:

- A `REQUEST_UPDATE` the draft names without its request (§10.2.2, §10.2.3, §10.2.4) is one of a subscription or of a FETCH; AUTHORIZATION TOKEN, which is about the request whatever it is, may be in the update of a `SUBSCRIBE_NAMESPACE` too. "REQUEST_UPDATE (for a subscription)" (§10.2.9, §10.2.12, §10.2.13) is of a subscription alone; "REQUEST_UPDATE (for a subscription or FETCH)" (§10.2.7) of either.
- `SUBSCRIBE_TRACKS`, which §10.2.2 and §10.2.12 name, is no message of this list: neither implementation implements it, and it is refused whatever it carries.
