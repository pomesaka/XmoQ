# Tokens a room hands out, checked by the relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, design and test authorization

## Objective

Material for deciding how the relay authorizes, which is an open question ([open questions](../open-questions.md)): the smallest form in which only someone with a token of a room can publish or subscribe in the room's namespace, with what draft 18 has for it. **Nothing here decides it.** The check is off unless the relay is given a key.

## What draft 18 has (§10.2.2, §10.3.1.3, §10.3.1.4)

- **Where a token travels**: in the AUTHORIZATION TOKEN parameter (0x03) of a request, "It MAY appear in a PUBLISH, SUBSCRIBE, REQUEST_UPDATE, SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS, PUBLISH_NAMESPACE, TRACK_STATUS or FETCH message", and in the Setup Option of the same number, "The endpoint can specify one or more tokens in SETUP that the peer can use to authorize MOQT session establishment". Both, then: one for the session, one for each request.
- **What it is**: a Token structure, with an Alias Type, a Token Alias, a Token Type and a Token Value. Token Type 0 "is reserved to indicate that the type is not defined in the table and is negotiated out-of-band"; the draft defines no format of a value.
- **Aliases**: a sender can register a token under a number (`REGISTER`), refer to it later (`USE_ALIAS`) and retire it (`DELETE`), "for efficiency only"; the receiver keeps what was registered, up to the MAX_AUTH_TOKEN_CACHE_SIZE it said in SETUP (0 when it said none). `USE_VALUE` carries the token itself and needs none of that.
- **The refusals**: REQUEST_ERROR `UNAUTHORIZED` (0x1), `MALFORMED_AUTH_TOKEN` (0x4), `EXPIRED_AUTH_TOKEN` (0x5), and for a session the codes of the same names (§3.5).
- **A relay**: "Relays MUST ensure subscribers are authorized to access the content associated with the Track ... The specifics of how a relay authorizes a user are outside the scope of this specification" (§9.4).

Others: the crate `moqtail` 0.15.0 has the Token structure with its alias types (`src/model/parameter/authorization_token.rs`), and its tests build a parameter with `new_use_value(0, "test-token")`; whether its relay checks anything was not read. `moqlivemock` was not read.

## The experiment

- **The relay** (`crates/relay/src/auth.rs`, `--auth-key-hex <key>`): before it takes a PUBLISH, a PUBLISH_NAMESPACE, a SUBSCRIBE or a Standalone FETCH, it wants a token among the request's parameters: Alias Type `USE_VALUE`, Token Type 0, and as the value the text `v1|<namespace>|<pub or sub>|<expiry in seconds since 1970>|<HMAC-SHA-256 of what is before, hex>`, made with the key it shares with whoever hands tokens out. The relay core got a place to hand such a check in (`Relay::set_authorize`); the key and the clock stay outside it.
- **The Rust client** (`--auth-token-hex`) puts a Token structure it is given into those four requests. It makes none.
- **The room** is `spikes/auth-token/mint.py` here: it makes a token from a key, a namespace, a role and a number of seconds.

## Commands

```sh
xmoq-relay --port 48895 --cert-file ... --key-file ... --auth-key-hex 00112233445566778899aabbccddeeff
xmoq-moqt-client --mode publish --namespace rooms/blue --track t ... \
    --auth-token-hex "$(spikes/auth-token/mint.py <key> rooms/blue pub 60)"
xmoq-moqt-client --mode subscribe --namespace rooms/blue --track t --objects 1 ... [--auth-token-hex <token>]
```

## Observed results

One run, the relay with the key.

| Who | Token | What the relay answered |
|---|---|---|
| Publisher | none | PUBLISH_NAMESPACE refused, 0x1 |
| Publisher | for `rooms/red` | PUBLISH_NAMESPACE refused, 0x1 |
| Publisher | `pub` for `rooms/blue` | accepted; it published |
| Subscriber | none | SUBSCRIBE refused, 0x1 |
| Subscriber | `sub` for `rooms/blue` | SUBSCRIBE_OK, and an Object |
| Subscriber | expired 10 seconds before | refused, 0x5 |
| Subscriber | `sub` for `rooms/red` | refused, 0x1 |
| Subscriber | `pub` for `rooms/blue` | refused, 0x1 |
| Subscriber | made with another key | refused, 0x1 |
| Subscriber | bytes that are no such token | refused, 0x4 |
| A Standalone FETCH | none | refused, 0x1 |

Without `--auth-key-hex` the relay is as it was: the matrix of `interop/` ran against it unchanged.

## Limitations and blockers

- **No browser.** `@xmoq/moqt` reads and writes the parameter (`core/parameters.ts`) and has no way to give a token to a request. What it would take: an option for the token on `subscribe`, `publish`, `publishNamespace` and `fetch`; `@xmoq/msf` and `@xmoq/media` passing it on to the requests they send; the room making tokens (HMAC with WebCrypto in its worker) and handing one to each page with the broadcast; the pages of the demo carrying it. None of it was written.
- **A token is checked when a request comes, and never again.** A subscription that was allowed goes on after its token has expired.
- **No aliases, and nothing in SETUP**: a parameter with `REGISTER`, `USE_ALIAS` or `DELETE` is refused as malformed, which is not what §10.2.2 has for them, and a token in SETUP is not looked at.
- A Joining FETCH is not asked about: it names no namespace and belongs to a SUBSCRIBE of its session, which was.
- The relay's own SUBSCRIBE and FETCH to a publisher carry no token.
- One shared key: whoever has it can make any token. The namespace has to match whole; there are no prefixes.

## Decision or next action

None. The choices, for the project's owner:

| | Where the token is checked | For | Against |
|---|---|---|---|
| A | Each request (this experiment) | A room's namespace is the unit that is allowed; no state in the relay; a publisher and a viewer differ by their token | Every request of every library has to carry it; a session without any token still takes a place among the relay's sessions |
| B | SETUP alone | One place in each client; a session without a token is closed at once | The token has to say everything the session may do; the relay keeps it for the session |
| C | Both | What the draft has room for | The most to build |
| | **How it is made** | | |
| 1 | HMAC with a shared key (this experiment) | A few lines on either side | The relay can make tokens, and so can anyone who reads its key |
| 2 | A signature (Ed25519), the relay with the public key | The relay cannot make tokens; several relays need no secret | More to build in the room; larger tokens |
| 3 | A format others have (the CAT of the IETF, a JWT) | Tools and a registered Token Type | A library, and a format to read into |
