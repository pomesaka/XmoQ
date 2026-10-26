//! Authorization: tokens a room hands out, checked with a key the relay
//! shares with the room. Off unless the relay is given a key
//! (`--auth-key-hex`). The form is the project's choice for a single relay
//! (a token on every request, made with a shared key); the format of the
//! value below is the experiment's.
//!
//! The token travels as §10.2.2 of `draft-ietf-moq-transport-18` has it, in
//! the AUTHORIZATION TOKEN parameter of a PUBLISH, a `PUBLISH_NAMESPACE`, a
//! SUBSCRIBE or a FETCH: a Token structure with Alias Type `USE_VALUE`
//! (0x3), Token Type 0 ("negotiated out-of-band"), and as its value the
//! text
//!
//! ```text
//! v1|<namespace, its fields joined with />|<pub or sub>|<expiry, seconds since 1970>|<mac, hex>
//! ```
//!
//! where the mac is HMAC-SHA-256 with the shared key over the text before
//! the last `|`. A token allows one operation (`pub`: PUBLISH and
//! `PUBLISH_NAMESPACE`; `sub`: SUBSCRIBE and a Standalone FETCH) in that
//! namespace, until its expiry: in exactly that namespace, or, where its
//! last field is `*`, in those under the fields before it
//! ([`namespace_fits`], decided on 2026-10-24, so that a room's chat can be
//! allowed).
//!
//! WHY a token on every request (decided on 2026-10-11): the room's
//! namespace is the unit, and the relay keeps no state of it. WHY NOT on
//! SETUP alone: the token would have to speak for the whole session. WHY
//! NOT a signature the relay checks with a public key, yet: more on the
//! room's side, for a need that comes with more than one relay, when it is
//! to take the shared key's place. WHY NOT CAT or JWT: a library, for
//! nothing this needs yet.
//!
//! The relay wants such a token in the SETUP of a client as well (the Setup
//! Option of the same number, §10.3.1.4), and closes a session without
//! one: there the namespace and the operation are not looked at, only
//! whether the room made the token and it has not expired. `--auth-setup
//! off` leaves the SETUP be.
//!
//! **The relay keeps no aliases**: it does not send
//! `MAX_AUTH_TOKEN_CACHE_SIZE`, whose "default value is 0 which prohibits
//! the use of token Aliases" (§10.3.1.3). What §10.2.2 has a receiver do
//! with the four Alias Types then is in [`RoomTokens::request`] and
//! [`RoomTokens::setup`].
//!
//! FIXME: a subscription that was allowed goes on after its token has
//! expired.

use std::time::{SystemTime, UNIX_EPOCH};

use xmoq_moqt::TerminationCode;
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::request_error_code as code;
use xmoq_moqt::message::setup_option;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::Parameter;
use xmoq_moqt::token::Token;
use xmoq_relay_core::{Authorizer, Denied, Operation};
use xmoq_room_token::{Fault, Grant, Role, RoomKey, namespace_fits};

/// The relay's check of the tokens a room made with `key`.
pub struct RoomTokens {
    key: RoomKey,
    /// A client's SETUP has to carry a token as well; not with
    /// `--auth-setup off`.
    setup: bool,
    /// The clock, in seconds since 1970; a test sets its own.
    now: fn() -> u64,
}

impl RoomTokens {
    pub fn new(key: &[u8], setup: bool) -> Self {
        Self {
            key: RoomKey::new(key),
            setup,
            now: || {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |since| since.as_secs())
            },
        }
    }

    /// What a token value says, once the room's key and the clock have
    /// borne it out: the namespace and the role. Else the code of the
    /// `REQUEST_ERROR` it is turned down with.
    fn read<'a>(&self, token_type: u64, value: &'a [u8]) -> Result<(&'a str, &'a str), u64> {
        // §10.2.2: "a well-formed Token structure but otherwise invalid
        // AUTHORIZATION TOKEN parameter" is turned down as malformed.
        if token_type != 0 {
            return Err(code::MALFORMED_AUTH_TOKEN);
        }
        let Grant {
            namespace,
            role,
            expiry,
        } = self.key.read(value).map_err(|fault| match fault {
            Fault::Malformed => code::MALFORMED_AUTH_TOKEN,
            Fault::NotOfTheKey => code::UNAUTHORIZED,
        })?;
        if expiry <= (self.now)() {
            return Err(code::EXPIRED_AUTH_TOKEN);
        }
        Ok((namespace, role))
    }
}

impl Authorizer for RoomTokens {
    /// One of the tokens among `parameters` has to allow the operation in
    /// the namespace. A Joining FETCH names no namespace: its token is
    /// checked for the key, the operation and the expiry, and it has to
    /// have one, as every request of a connection that has a token does.
    ///
    /// The Alias Types, for a receiver that keeps none (§10.2.2):
    ///
    /// * A structure that "cannot be decoded": "the receiver MUST close
    ///   the Session with `KEY_VALUE_FORMATTING_ERROR`".
    /// * `REGISTER`: "If a registration is attempted which would cause this
    ///   limit to be exceeded, the receiver MUST terminate the Session with
    ///   a `AUTH_TOKEN_CACHE_OVERFLOW` error", and the limit is 0.
    /// * `USE_ALIAS` and `DELETE`: "The receiver of a message referencing
    ///   an Alias that is not currently registered MUST reject the message
    ///   with `UNKNOWN_AUTH_TOKEN_ALIAS`". **The session is closed with
    ///   that code** (0x17): it is a code of §3.5 alone, and `REQUEST_ERROR`
    ///   (§10.6) has none of that name to reject a message with.
    /// * `USE_VALUE`: the token is read.
    fn request(
        &self,
        operation: Operation,
        namespace: Option<&TrackNamespace>,
        parameters: &[Parameter],
    ) -> Result<(), Denied> {
        let mut verdict = None;
        for parameter in parameters {
            let Parameter::AuthorizationToken(bytes) = parameter else {
                continue;
            };
            let (token_type, value) = match Token::decode(bytes) {
                None => return Err(Denied::Session(TerminationCode::KeyValueFormattingError)),
                Some(Token::Register { .. }) => {
                    return Err(Denied::Session(TerminationCode::AuthTokenCacheOverflow));
                }
                Some(Token::UseAlias { .. } | Token::Delete { .. }) => {
                    return Err(Denied::Session(TerminationCode::UnknownAuthTokenAlias));
                }
                Some(Token::UseValue { token_type, value }) => (token_type, value),
            };
            let allows = self.read(token_type, &value).and_then(|(named, role)| {
                let role_fits = matches!(
                    (role, operation),
                    ("pub", Operation::Publish) | ("sub", Operation::Subscribe)
                );
                let fits = namespace.is_none_or(|asked| namespace_fits(asked, named));
                if role_fits && fits {
                    Ok(())
                } else {
                    Err(code::UNAUTHORIZED)
                }
            });
            // One that allows it is enough; else the first that did not.
            verdict = match (verdict, allows) {
                (Some(Ok(())), _) | (_, Ok(())) => Some(Ok(())),
                (Some(Err(first)), Err(_)) => Some(Err(first)),
                (None, Err(error)) => Some(Err(error)),
            };
        }
        match verdict {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(Denied::Request(error)),
            None => Err(Denied::Request(code::UNAUTHORIZED)),
        }
    }

    /// One of the tokens of the SETUP has to be the room's and not expired, or the session is closed, with
    /// `UNAUTHORIZED` ("The client is not authorized to establish a
    /// session", §3.5), `MALFORMED_AUTH_TOKEN` or `EXPIRED_AUTH_TOKEN`.
    ///
    /// With `--auth-setup off` no token is asked for; the Alias Types are
    /// looked at all the same.
    ///
    /// The Alias Types in SETUP: "If a server receives Alias Type DELETE
    /// (0x0) or `USE_ALIAS` (0x2) in a SETUP message, it MUST close the
    /// session with a `PROTOCOL_VIOLATION`" (§10.2.2); a `REGISTER` that does
    /// not fit the cache, which is every one here, "MUST NOT fail the
    /// session ... it MUST treat the option as Alias Type `USE_VALUE`"
    /// (§10.3.1.4).
    fn setup(&self, setup_options: &[KeyValuePair]) -> Result<(), TerminationCode> {
        let mut verdict = Err(TerminationCode::Unauthorized);
        let mut seen = false;
        for option in setup_options {
            let Value::Bytes(bytes) = option.value() else {
                continue;
            };
            if option.ty() != setup_option::AUTHORIZATION_TOKEN {
                continue;
            }
            let (token_type, value) = match Token::decode(bytes) {
                None => return Err(TerminationCode::KeyValueFormattingError),
                Some(Token::UseAlias { .. } | Token::Delete { .. }) => {
                    return Err(TerminationCode::ProtocolViolation);
                }
                Some(
                    Token::Register {
                        token_type, value, ..
                    }
                    | Token::UseValue { token_type, value },
                ) => (token_type, value),
            };
            let this = match self.read(token_type, &value) {
                Ok(_) => Ok(()),
                Err(code::MALFORMED_AUTH_TOKEN) => Err(TerminationCode::MalformedAuthToken),
                Err(code::EXPIRED_AUTH_TOKEN) => Err(TerminationCode::ExpiredAuthToken),
                Err(_) => Err(TerminationCode::Unauthorized),
            };
            // One that is good is enough; else the first that was not.
            if this.is_ok() || !seen {
                verdict = this;
            }
            seen = true;
        }
        if self.setup { verdict } else { Ok(()) }
    }

    /// A token for the namespace `*`, which allows every namespace
    /// ([`namespace_fits`]), for [`OWN_TOKEN_SECONDS`] from now: what the
    /// relay puts on its SETUP and requests to a parent relay that
    /// shares the key. Made anew for each, as the chat's gatherer makes its
    /// own (`crates/chat`): a relay runs for longer than one token.
    fn own_token(&self, operation: Operation) -> Option<Vec<u8>> {
        let role = match operation {
            Operation::Publish => Role::Pub,
            Operation::Subscribe => Role::Sub,
        };
        let expiry = (self.now)().saturating_add(OWN_TOKEN_SECONDS);
        let text = self.key.make("*", role, expiry);
        Some(
            Token::UseValue {
                token_type: 0,
                value: text.into_bytes(),
            }
            .encode(),
        )
    }
}

/// How long a token the relay makes for itself runs: as long as one the
/// room makes (`TOKEN_SECONDS` of `packages/room/src/token.ts`)
/// and one the gatherer makes. Provisional.
const OWN_TOKEN_SECONDS: u64 = 600;

#[cfg(test)]
mod tests {
    use ring::hmac;

    use super::*;

    const KEY: &[u8] = b"a key of the room";

    fn room(setup: bool) -> RoomTokens {
        RoomTokens {
            now: || 1_800_000_000,
            ..RoomTokens::new(KEY, setup)
        }
    }

    /// The value a room would make, and the Token structure around it.
    fn value(key: &[u8], signed: &str) -> Vec<u8> {
        let mac = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), signed.as_bytes());
        let digits: Vec<String> = mac
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("{signed}|{}", digits.concat()).into_bytes()
    }

    fn token(key: &[u8], signed: &str) -> Vec<u8> {
        Token::UseValue {
            token_type: 0,
            value: value(key, signed),
        }
        .encode()
    }

    const GOOD: &str = "v1|rooms/blue|sub|1800000060";

    type Case = (Vec<Vec<u8>>, Operation, bool, Result<(), Denied>);

    /// Each case: the tokens of the request; what it asks; whether it names
    /// the namespace rooms/blue, which a Joining FETCH does not; the verdict.
    fn check(cases: impl IntoIterator<Item = Case>) {
        let blue = TrackNamespace::new(vec![b"rooms".to_vec(), b"blue".to_vec()]).unwrap();
        for (tokens, operation, named, verdict) in cases {
            let parameters: Vec<Parameter> = tokens
                .iter()
                .cloned()
                .map(Parameter::AuthorizationToken)
                .collect();
            assert_eq!(
                room(false).request(operation, named.then_some(&blue), &parameters),
                verdict,
                "{tokens:?} {operation:?} {named}"
            );
        }
    }

    #[test]
    fn a_request_is_allowed_by_a_token_for_its_namespace_its_operation_and_its_time() {
        use Operation::{Publish, Subscribe};
        let request = |code| Err(Denied::Request(code));
        // REQUEST_ERROR: UNAUTHORIZED 0x1, MALFORMED_AUTH_TOKEN 0x4,
        // EXPIRED_AUTH_TOKEN 0x5.
        // (the tokens of the request; what it asks; whether it names the
        //  namespace rooms/blue, which a Joining FETCH does not; the
        //  verdict)
        let cases = [
            (vec![token(KEY, GOOD)], Subscribe, true, Ok(())),
            (vec![], Subscribe, true, request(0x1)),
            // Another operation, another namespace, a shorter one.
            (vec![token(KEY, GOOD)], Publish, true, request(0x1)),
            (
                vec![token(KEY, "v1|rooms/red|sub|1800000060")],
                Subscribe,
                true,
                request(0x1),
            ),
            (
                vec![token(KEY, "v1|rooms|sub|1800000060")],
                Subscribe,
                true,
                request(0x1),
            ),
            // Expired, at the second of its expiry.
            (
                vec![token(KEY, "v1|rooms/blue|sub|1800000000")],
                Subscribe,
                true,
                request(0x5),
            ),
            // Made with another key: whatever it says, also that it has
            // expired.
            (
                vec![token(b"another key", GOOD)],
                Subscribe,
                true,
                request(0x1),
            ),
            (
                vec![token(b"another key", "v1|rooms/blue|sub|1")],
                Subscribe,
                true,
                request(0x1),
            ),
            // A structure that is well-formed and no token of the room.
            (vec![vec![0x03, 0x00]], Subscribe, true, request(0x4)),
            (vec![vec![0x03, 0x05, 0x61]], Subscribe, true, request(0x4)),
            (
                vec![token(KEY, "v2|rooms/blue|sub|1800000060")],
                Subscribe,
                true,
                request(0x4),
            ),
            // A star allows what is under it and the namespace itself, and
            // nothing shorter.
            (
                vec![token(KEY, "v1|rooms/*|sub|1800000060")],
                Subscribe,
                true,
                Ok(()),
            ),
            (
                vec![token(KEY, "v1|rooms/blue/*|sub|1800000060")],
                Subscribe,
                true,
                Ok(()),
            ),
            (
                vec![token(KEY, "v1|rooms/blue/red/*|sub|1800000060")],
                Subscribe,
                true,
                request(0x1),
            ),
            // One of two tokens allows it.
            (
                vec![token(KEY, "v1|rooms/red|sub|1800000060"), token(KEY, GOOD)],
                Subscribe,
                true,
                Ok(()),
            ),
        ];
        check(cases);
    }

    #[test]
    fn the_alias_types_and_a_joining_fetch_are_treated_as_a_relay_without_a_cache_has_to() {
        use Operation::Subscribe;
        let request = |code| Err(Denied::Request(code));
        let session = |code| Err(Denied::Session(code));
        let registered = Token::Register {
            alias: 1,
            token_type: 0,
            value: value(KEY, GOOD),
        };
        // (as in the test above)
        let cases = [
            // A structure that cannot be decoded.
            (
                vec![vec![]],
                Subscribe,
                true,
                session(TerminationCode::KeyValueFormattingError),
            ),
            (
                vec![vec![0x04, 0x00]],
                Subscribe,
                true,
                session(TerminationCode::KeyValueFormattingError),
            ),
            // The Alias Types of a cache the relay does not keep, also
            // beside a token that allows the request.
            (
                vec![registered.encode()],
                Subscribe,
                true,
                session(TerminationCode::AuthTokenCacheOverflow),
            ),
            (
                vec![vec![0x02, 0x01]],
                Subscribe,
                true,
                session(TerminationCode::UnknownAuthTokenAlias),
            ),
            (
                vec![vec![0x00, 0x01]],
                Subscribe,
                true,
                session(TerminationCode::UnknownAuthTokenAlias),
            ),
            (
                vec![token(KEY, GOOD), vec![0x02, 0x01]],
                Subscribe,
                true,
                session(TerminationCode::UnknownAuthTokenAlias),
            ),
            // A Joining FETCH: with the token of its SUBSCRIBE, whatever
            // namespace that names; not without one, with one for
            // publishing, or with one that has expired.
            (vec![], Subscribe, false, request(0x1)),
            (
                vec![token(KEY, "v1|rooms/red|sub|1800000060")],
                Subscribe,
                false,
                Ok(()),
            ),
            (
                vec![token(KEY, "v1|rooms/blue|pub|1800000060")],
                Subscribe,
                false,
                request(0x1),
            ),
            (vec![token(KEY, GOOD)], Subscribe, false, Ok(())),
            (
                vec![token(KEY, "v1|rooms/blue|sub|1800000000")],
                Subscribe,
                false,
                request(0x5),
            ),
            (
                vec![vec![0x02, 0x01]],
                Subscribe,
                false,
                session(TerminationCode::UnknownAuthTokenAlias),
            ),
        ];
        check(cases);
    }

    #[test]
    fn a_session_is_allowed_by_a_token_of_the_room_in_its_setup() {
        let registered = Token::Register {
            alias: 1,
            token_type: 0,
            value: value(KEY, GOOD),
        };
        // Session codes: UNAUTHORIZED 0x2, PROTOCOL_VIOLATION 0x3,
        // KEY_VALUE_FORMATTING_ERROR 0x6, MALFORMED_AUTH_TOKEN 0x16,
        // EXPIRED_AUTH_TOKEN 0x18.
        // (the tokens of the SETUP; the code the session is closed with)
        let cases: [(Vec<Vec<u8>>, Option<u32>); 9] = [
            (vec![token(KEY, GOOD)], None),
            // Any namespace and either role: the session is not a request.
            (vec![token(KEY, "v1|rooms/red|pub|1800000060")], None),
            (vec![], Some(0x2)),
            (vec![token(b"another key", GOOD)], Some(0x2)),
            (vec![token(KEY, "v1|rooms/blue|sub|1800000000")], Some(0x18)),
            (vec![vec![0x03, 0x00]], Some(0x16)),
            (vec![vec![]], Some(0x6)),
            // REGISTER is read as USE_VALUE; the other two are not for SETUP.
            (vec![registered.encode()], None),
            (vec![vec![0x02, 0x01]], Some(0x3)),
        ];
        for (tokens, closed) in cases {
            let options: Vec<KeyValuePair> = tokens
                .iter()
                .map(|token| KeyValuePair::bytes(0x03, token.clone()).unwrap())
                .collect();
            let verdict = room(true).setup(&options).err().map(TerminationCode::code);
            assert_eq!(verdict, closed, "{tokens:?}");
            // With `--auth-setup off` no token is asked for, and a
            // structure that cannot be decoded or has an alias that is not
            // for SETUP closes the session all the same.
            let off = room(false).setup(&options).err().map(TerminationCode::code);
            assert_eq!(
                off,
                closed.filter(|code| matches!(code, 0x3 | 0x6)),
                "{tokens:?}"
            );
        }
    }

    /// The relay's own token passes a relay with the same key for its
    /// operation in any namespace, and is the one a room would make for
    /// `*` that runs for 600 seconds.
    #[test]
    fn the_relays_own_token_allows_its_operation_everywhere_at_a_relay_of_the_same_key() {
        let rows = [
            (Operation::Subscribe, Operation::Subscribe, Ok(())),
            (Operation::Publish, Operation::Publish, Ok(())),
            (
                Operation::Subscribe,
                Operation::Publish,
                Err(Denied::Request(code::UNAUTHORIZED)),
            ),
        ];
        let here = TrackNamespace::new(vec![b"xmoq".to_vec(), b"chat".to_vec()]).unwrap();
        for (made_for, asked, verdict) in rows {
            let made = room(false).own_token(made_for).unwrap();
            let parameters = [Parameter::AuthorizationToken(made)];
            assert_eq!(
                room(false).request(asked, Some(&here), &parameters),
                verdict,
                "{made_for:?} {asked:?}"
            );
        }
        assert_eq!(
            room(false).own_token(Operation::Subscribe),
            Some(token(KEY, "v1|*|sub|1800000600"))
        );
    }
}
