# MoQ Transport implementation notes

Behaviour that the Rust (`crates/moqt`) and TypeScript (`packages/moqt`) implementations must share, where `draft-ietf-moq-transport-18` is silent, contradicts itself, or leaves a choice. Both implementations follow this table; a change here is made in both in the same pull request.

Items marked **provisional** depend on an [open question](open-questions.md).

| Case | Behaviour | Basis |
|---|---|---|
| Request that the draft defines and XmoQ does not implement, opened by the peer (FETCH, TRACK_STATUS, SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS) | Answer with REQUEST_ERROR `NOT_SUPPORTED` (`0x3`); the session stays open | §10.6.2 provides the code. Closing the session would drop every other track for one unsupported request |
| SUBSCRIBE, PUBLISH or PUBLISH_NAMESPACE opened by the peer, received by an implementation that has no responder side (the clients) | Same answer: REQUEST_ERROR `NOT_SUPPORTED` | A client does not serve requests; the relay will |
| REQUEST_UPDATE for a subscription that the receiver publishes to (the stream of its own established PUBLISH) | Answer the first one with REQUEST_ERROR `NOT_SUPPORTED`, then end the subscription with PUBLISH_DONE `UPDATE_FAILED` (`0x8`). Further REQUEST_UPDATEs that arrive before the subscription has ended get no reply. The session stays open. The parameters of every REQUEST_UPDATE are still checked against §10.2.1 first, and a parameter that is not allowed there is a `PROTOCOL_VIOLATION` | §10.9 requires REQUEST_OK or REQUEST_ERROR in reply, and when the update is unsuccessful the publisher MUST also terminate the subscription with `UPDATE_FAILED`. §10.9.1: when coalesced updates fail, "only a single REQUEST_ERROR will be sent". Updates are not implemented |
| REQUEST_UPDATE on any other request stream of a client | `PROTOCOL_VIOLATION` | §10.9 lets only the sender of a request, or the subscriber of a subscription created by PUBLISH, send it. A client's own SUBSCRIBE or PUBLISH_NAMESPACE stream cannot carry one from the peer |
| Other message or stream type that the draft defines and XmoQ does not implement (`FETCH_HEADER`, GOAWAY and other messages that have no reply) | Close the session with `INTERNAL_ERROR` | There is no per-request answer to give. The peer follows the draft, so it is not a violation |
| An Object after End of Group or End of Track on the same subgroup stream | Malformed Track: the subscription is cancelled and the session stays open. Decided as soon as the first byte of the further Object arrives, without waiting for the whole Object | §2.4.2 lists it as a malformed track. §11.4.3 calls it "a protocol violation" in passing; the list in §2.4.2 is taken as the rule. Waiting would buffer up to the Object size limit for an Object that is discarded anyway |
| One Object with more than one fault | The fault in the earlier field decides, because an Object is judged field by field as it arrives. Object Properties come before Payload Length and Object Status, so an Object whose properties make the track malformed is a Malformed Track even if its status is not Normal (which would be a session-level violation on its own). A fault before the properties (Object ID overflow, a broken key-value encoding, a receive limit) is reported first. An Object after End of Group or End of Track is not decoded at all (row above) | Follows from deciding as early as possible (rows above and below); any other order would need the whole Object buffered |
| Any message that arrives on the request stream of a PUBLISH after the publisher sent PUBLISH_DONE for it, whether the PUBLISH was pending or established | Ignored, until the stream closes | §5.1 allows PUBLISH_DONE in both states, and §10.9 lets the subscriber send REQUEST_UPDATE, so a PUBLISH_OK, a REQUEST_ERROR and several updates can all cross the PUBLISH_DONE |
| SUBSCRIBE_OK that reuses a Track Alias whose previous subscription received PUBLISH_DONE and is still draining | Accepted. The alias now names the new subscription and the old one ends | §11.1 requires `DUPLICATE_TRACK_ALIAS` only when two established subscriptions overlap |
| A subscription ends while subgroup streams for it are open | Send STOP_SENDING on each of them | §10.11 (SHOULD). Otherwise each stream can buffer up to the Object size limit |
| Message type `0x1E` | Treated as defined and not implemented (row above) | Table 5 lists `0x1E` as PUBLISH_OK; §10.5 says PUBLISH is answered with REQUEST_OK (`0x7`). XmoQ follows §10.5, as `moqtail` 0.15.0 does |
| Message or stream type that appears in no table, and the message IDs that Table 5 marks RESERVED (`0x01`, `0x20`, `0x21`, `0x40`, `0x41`, the setup messages of older drafts) | Close the session with `PROTOCOL_VIOLATION` | The draft says to close the session and names no code. A reserved ID is not a message of draft 18 |
| Unknown Setup Option, unknown Property, unknown error code in REQUEST_ERROR, PUBLISH_DONE or a stream reset, unknown auth token type | Never a reason to close the session. An unknown error code is treated as `INTERNAL_ERROR` for that context | §14 (MUST): these registries can grow without negotiation |
| `AUTHORIZATION_TOKEN` | Accepted and carried as opaque bytes; not interpreted | Authentication is undecided. Rejecting the parameter would break peers that send it |
| `FILL_TIMEOUT` or `TRACK_NAMESPACE_PREFIX` in a message that does not allow it | `PROTOCOL_VIOLATION` | §10.2.1 |
| Padding stream (`0x132B3E28`) | Read and discard | §11.5.1 (MUST) |
| After PUBLISH_DONE | The Track Alias stays usable until the number of streams in Stream Count have ended, or the caller reports a timeout. A finished alias is remembered, up to a bound, as "discard" so that a late Object is not resolved to a new track. The timer lives outside the sans-I/O core | §10.11, §11.1 |
| Receive limit exceeded | Control message: close the session with `INTERNAL_ERROR`. Object on a data stream: stop that stream only. The values of the limits are **provisional** | Decided for XmoQ: one oversized Object does not cost the other tracks of the session |
| More than 64 key-value pairs in one list (Setup Options, Message Parameters including repeated `AUTHORIZATION_TOKEN`s, Track Properties, Object Properties) | A receive limit (row above). The number is the same in both implementations, so that neither accepts input the other rejects. Where the list carries a count, the count is checked against the limit as soon as it is read, before any pair is decoded, so a count that is over the limit and also larger than the remaining bytes is reported as the limit. **Provisional** | The draft sets no limit. A pair arrives in 2 bytes and takes far more in memory. Closing for the count is not closing "solely because of an unknown value" (§14) |
| Stream Reset Error Code used with STOP_SENDING or a reset | Malformed Track: `MALFORMED_TRACK` (`0x12`). Receive limit exceeded: `INTERNAL_ERROR` (`0x0`). The subscription ended or was cancelled: `CANCELLED` (`0x1`) | §3.3.3 (SHOULD use a relevant code). `0x12` is described for a relay publisher; it is still the code that tells the sender why |
| Object Properties that make the track malformed (a Mandatory Track Property on an Object, a broken Prior Group ID Gap or Prior Object ID Gap) | Decided when the properties have been read, before the payload arrives | Same reason as the End of Group row: the payload of a rejected Object is not buffered. Otherwise a stream that ends inside the payload gives a different result in each implementation |
| SUBSCRIBE_OK that reuses the Track Alias of a draining subscription and is itself refused (it carries a Mandatory Track Property the receiver does not support) | The draining subscription is left as it is. The refused one never takes the alias | A subscription that was never established cannot end another one |
| Remembered Track Aliases of ended subscriptions; stream closes counted before SUBSCRIBE_OK | At most 64 of each; the oldest is dropped. **Provisional** | Same number in both implementations |
| A Mandatory Track Property error and a duplicate Track Alias in the same message | The session-closing error (`DUPLICATE_TRACK_ALIAS`) wins | §11.1 |
| The peer sends STOP_SENDING on a request stream that this endpoint opened | On the stream of an established PUBLISH: the subscription ends and its open subgroup streams are reset. On the stream of a request that is still pending: nothing changes yet; the request ends when the peer's own direction of the stream is reset or carries a reply. On the stream of a SUBSCRIBE that already received PUBLISH_DONE: nothing changes, the subscription keeps draining so that late Objects still arrive. On a control stream: `PROTOCOL_VIOLATION` | §5.1 and §5.1.1 make STOP_SENDING the subscriber's way to end an established subscription created by PUBLISH. §3.3.2: a receiver that cancels a request terminates every direction that is still open, so the reset of its own direction follows, and acting on STOP_SENDING alone could drop a REQUEST_ERROR that is already in flight. §10.11 lets Objects arrive after PUBLISH_DONE. §3.3 forbids closing a control stream |
| The peer resets its direction of a request stream that this endpoint opened | A pending request ends as cancelled by the peer. An established subscription ends. A SUBSCRIBE that already received PUBLISH_DONE keeps draining | §3.3.2, §10.11 |
| Request ID parity; Redirect in REQUEST_ERROR | Both are checked | §10.1, §10.6.1 |
| Request IDs from the peer that arrive out of order or leave gaps | Never a reason to close the session. Seen IDs are remembered to detect a duplicate; when more than 1024 IDs are remembered above the lowest one not seen yet, each new ID pushes out the lowest remembered one, so a duplicate of that ID is no longer detected. One exception, in Rust only: an ID whose request is still open is a duplicate even after it was pushed out (TypeScript keeps no requests of the peer, so the case does not arise there). A REQUEST_UPDATE consumes a Request ID even when the message itself is ignored | §10.1 asks the receiver to check parity and duplicates only. Forgetting gaps instead ("everything below is seen") would turn a late, valid request into a duplicate and close the session |
| More subgroup streams arrive for a subscription than the Stream Count of its PUBLISH_DONE | Not checked. The streams are handled like the others, and the session stays open | §10.11 gives the count so that the subscriber knows when everything arrived. Closing for a wrong count is not asked for, and a publisher that cannot count sends 2^62 - 1 |
| A stream error code received from WebTransport that is outside the range of `draft-ietf-webtrans-http3-16` §4.4 or on one of its reserved codepoints | Reported as a reset or stop without a code, which is then handled like `INTERNAL_ERROR` (unknown-code row above) | §4.4: such a code SHOULD be delivered "as a stream reset with no application error code" |
| Track Namespace and Track Name in message types | The names an API user sees are `track_namespace` and `track_name` (camelCase in TypeScript), as in the draft's message definitions. How a message stores them is each implementation's choice: Rust keeps one validated full-track-name value, TypeScript keeps two fields | §2.4.1 and each message's definition. What must match between the implementations is the vocabulary, not the representation |

## Known gaps against the draft

These are requirements of draft 18 that the implementations do not meet yet. Each has a `FIXME:` at the place in the code where it belongs.

- **Authorization token structure (§10.2.2).** The token is carried as opaque bytes, so the alias types (REGISTER, USE_ALIAS, DELETE) and the rules for them in SETUP are not enforced. Left until authentication is decided.
- **Datagrams (§11).** Neither implementation reads datagrams, so an OBJECT_DATAGRAM is dropped without notice and the rule "an endpoint that receives an unknown datagram type MUST close the session" is not met. Datagram delivery is outside the Phase 0b scope.
- **Request IDs of requests the peer opens, in the Rust client.** The client answers every such request with `NOT_SUPPORTED` and does not check its Request ID (§10.1, `INVALID_REQUEST_ID`). The relay checks them.
- **Session Termination Error Code towards a browser.** `wtransport` 0.7.2 cannot send the `WT_CLOSE_SESSION` capsule, so a browser sees a lost connection without the code or the reason. Recorded in [the relay record](experiments/2026-10-10-own-relay-vs-moqtail-peers.md).

The relay (`crates/relay-core`, `crates/relay`) adds these:

- **Routing by namespace (§9.5).** A PUBLISH_NAMESPACE is accepted and its namespace recorded, and nothing uses it: a SUBSCRIBE for a track that no PUBLISH offered is rejected, where the relay should send a SUBSCRIBE to every publisher of a matching namespace.
- **Several publishers of one track (§9.3).** A second PUBLISH is rejected.
- **Subscription Filter (§5.1.2).** Every subscriber gets what arrives after it subscribed, whatever its filter says.
- **NEW_GROUP_REQUEST (§10.2.13).** A relay MUST pass it on to the publisher; it is ignored.
- **Delivery timeouts (§8, §10.2.3, §10.2.4).** `OBJECT_DELIVERY_TIMEOUT` and `SUBGROUP_DELIVERY_TIMEOUT` are ignored, as parameters and as Track Properties.
- **RENDEZVOUS_TIMEOUT (§10.2.6).** A SUBSCRIBE that carries it is rejected at once with `DOES_NOT_EXIST` when the track has no publisher, where the relay SHOULD wait.
- **Retry Interval (§10.6.2).** Always 0, which reads as "do not retry", also where a retry could succeed later.
- **REQUEST_UPDATE** is refused, never applied, so the Forward State of a subscription cannot change.
- **Authorization (§9.4, §9.5).** Not done; there is no authentication yet.

## Relay

Choices of the relay where the draft leaves one. The limits are **provisional**.

| Case | Behaviour | Basis |
|---|---|---|
| PUBLISH for a track that already has a publisher | REQUEST_ERROR `UNINTERESTED` (`0x20`) | §9.3 lets a relay refuse a publisher and names no code. §10.6.2 gives this one to a subscriber that declines a PUBLISH, which the relay is here |
| Forward State in the answer to PUBLISH | 1 from the start. Objects that arrive while nobody subscribes are dropped | §9.5 allows 0 until a subscriber comes, which needs a REQUEST_UPDATE towards the publisher later |
| SUBSCRIBE with FORWARD 0 | Accepted. No subgroup stream is opened and no Object sent; the subscriber still gets PUBLISH_DONE | §5.1: "The publisher does not send Objects if the Forward State is 0" |
| LARGEST_OBJECT in SUBSCRIBE_OK | The one the publisher gave in PUBLISH, or a larger Location that has passed through the relay since | §10.2.11 |
| What ends the upstream subscription | The subscribers get PUBLISH_DONE `TRACK_ENDED`, whatever the publisher's status was. Exceptions: `MALFORMED_TRACK` for a malformed track, `UPDATE_FAILED` after a refused REQUEST_UPDATE, `TOO_FAR_BEHIND` below | The draft has no rule for the status a relay sends when its upstream ends; this is the relay's choice, and an open question of [the relay record](experiments/2026-10-10-own-relay-vs-moqtail-peers.md) |
| REQUEST_UPDATE for a request the relay has not answered yet | Its REQUEST_ERROR is the only response the stream gets, and the request ends | Responses carry no Request ID, so two on one stream could not be told apart |
| A request that would double a subscription in the same role; a PUBLISH with a Mandatory Track Property the relay does not support; a request beyond the limit of open ones | `DUPLICATE_SUBSCRIPTION`; `UNSUPPORTED_EXTENSION`; `EXCESSIVE_LOAD` | §5.1, §2.5.1, §10.6.2 |
| A subscriber that does not take what is sent to it | Counted per subscriber session, over all its subgroup streams: payload bytes and Objects not written out, and streams not written out, finished or not. Over a limit, the subscription that hit it ends with PUBLISH_DONE `TOO_FAR_BEHIND` and all its streams are reset with `TOO_FAR_BEHIND`. Other subscribers and the publisher are not held back | §8 names `TOO_FAR_BEHIND`. A limit per stream does not see many short Subgroups; a count of Objects alone takes a burst of small ones for slowness |
| A subgroup stream whose Track Alias is not known yet | Held, with its Objects, until the PUBLISH arrives, within limits per session (payload and properties are counted) and for a limited time. When the PUBLISH is refused, the streams with its alias are stopped | §11.4.2: a receiver "MAY abandon the stream, or choose to buffer it for a brief period" |
| STOP_SENDING on a request stream the peer opened | The request is cancelled: a subscription ends and its open streams are reset; the relay resets its direction of the stream. On the relay's control stream: `PROTOCOL_VIOLATION` | §3.3.2, §5.1.1, §3.3 |
| Stream Count in the PUBLISH_DONE the relay sends | The number of subgroup streams it opened for the subscription. 2^62 - 1 when one of them was reset before the transport reported it open, or was never opened. A stream the relay finished with a FIN is always counted exactly | §10.11 (MUST): the exact number, or 2^62 - 1. The clients send the count their caller gives them and have no such case |
| A subscriber that does not close its direction of the request stream after the relay's PUBLISH_DONE | After a wait, the relay sends STOP_SENDING (`CANCELLED`) and forgets the request | §10.11 and §5.1.1 do not make the subscriber close the stream, and until it does the request counts against the limit of open ones |
| A publisher whose connection goes | What had arrived on the connection before is read first, for a limited time, and only then is the session treated as closed. Subgroup streams the publisher had finished are delivered whole to the subscribers; only the ones cut short are reset | A publisher that sends its last Object and PUBLISH_DONE and closes must not lose them. Which bytes a transport still hands over after a close is pinned by the tests in `crates/moqt-io/src/contract.rs` |

| Limit | Value |
|---|---|
| Sessions; WebTransport handshakes in progress | 256; 64 |
| Open requests of one peer; Request IDs remembered above a gap | 256; 1024 |
| Streams of one peer read at once, which is also the QUIC limit on the streams it may have open | 64 unidirectional, 264 request streams. The 256 open requests act first (`EXCESSIVE_LOAD`); a peer that goes on is held by QUIC at 264 |
| QUIC windows of the relay, per connection | 256 KiB received and unread a stream, 4 MiB over all streams; 4 MiB written and unacknowledged |
| Undecoded bytes of subgroup streams | 1 MiB per stream, 4 MiB per session |
| Held for an unknown Track Alias, per session | 8 streams, 32 Objects, 1 MiB, for 2 seconds |
| Waiting to be written to one subscriber session | 4 MiB, 4096 Objects, 256 streams |
| Messages waiting for one session's control stream, and for each request stream | 256 |
| Events waiting for the relay | 1024 |
| Wait for late Objects after PUBLISH_DONE | 2 seconds |
| A stream that does not say what it is; the WebTransport handshake | 10 seconds each |
| A request stream the peer leaves open after the relay's PUBLISH_DONE | 2 seconds, then STOP_SENDING |
| Reading what had arrived on a connection that is gone | 1 second |
| Idle timeout; keep-alive | 9 seconds; 3 seconds |

## Encodings the draft does not give

- `RENDEZVOUS_TIMEOUT` (`0x04`) and `FILL_TIMEOUT` (`0x0A`): the value is a variable-length integer in milliseconds. The draft (§10.2.5, §10.2.6) gives no encoding. This follows `moqtail` 0.15.0, read in `libs/moqtail-rs/src/model/parameter/message_parameter.rs` at tag `relay@0.15.0` on 2026-10-10 (lines 510–515 build the pair, line 733 writes it; only FORWARD, SUBSCRIBER_PRIORITY and GROUP_ORDER are written as one fixed byte). The basis is source reading only: no captured bytes carry these parameters.

## Other observations about the draft

- The variable-length integer of draft 18 is not the QUIC (RFC 9000) varint: the number of leading one bits gives the length, up to 9 bytes.
- The second example in §11.6 does not add up (a 14-byte value under `Length = 21`). It is not used as a test fixture.
- `moqtail` 0.15.0 clients omit `MOQT_IMPLEMENTATION` in SETUP (§10.3.1.5 is a SHOULD); its relay sends it. The relay's control-message log prints the raw bytes received and sent, checked in `libs/moqtail-rs/src/transport/control_stream_handler.rs` at tag `relay@0.15.0` on 2026-10-10.
