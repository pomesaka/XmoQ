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
//! `PUBLISH_NAMESPACE`; `sub`: SUBSCRIBE and a Standalone FETCH) in exactly
//! that namespace, until its expiry.
//!
//! With `--auth-setup on` the relay wants such a token in the SETUP of a
//! client as well (the Setup Option of the same number, §10.3.1.4), and
//! closes a session without one: there the namespace and the operation are
//! not looked at, only whether the room made the token and it has not
//! expired.
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

use ring::hmac;
use xmoq_moqt::TerminationCode;
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::request_error_code as code;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::Parameter;
use xmoq_moqt::token::Token;
use xmoq_relay_core::{Authorizer, Denied, Operation};

/// The Setup Option AUTHORIZATION TOKEN (§10.3.1.4).
const SETUP_AUTHORIZATION_TOKEN: u64 = 0x03;

/// The relay's check of the tokens a room made with `key`.
pub struct RoomTokens {
    key: hmac::Key,
    /// `--auth-setup`: a client's SETUP has to carry a token as well.
    setup: bool,
    /// The clock, in seconds since 1970; a test sets its own.
    now: fn() -> u64,
}

impl RoomTokens {
    pub fn new(key: &[u8], setup: bool) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, key),
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
        let malformed = code::MALFORMED_AUTH_TOKEN;
        // §10.2.2: "a well-formed Token structure but otherwise invalid
        // AUTHORIZATION TOKEN parameter" is turned down as malformed.
        if token_type != 0 {
            return Err(malformed);
        }
        let value = std::str::from_utf8(value).map_err(|_| malformed)?;
        let (signed, mac) = value.rsplit_once('|').ok_or(malformed)?;
        let mut fields = signed.split('|');
        let (Some("v1"), Some(namespace), Some(role), Some(expiry), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(malformed);
        };
        let expiry: u64 = expiry.parse().map_err(|_| malformed)?;
        let mac = unhex(mac).ok_or(malformed)?;
        // Before anything the token says is believed.
        hmac::verify(&self.key, signed.as_bytes(), &mac).map_err(|_| code::UNAUTHORIZED)?;
        if expiry <= (self.now)() {
            return Err(code::EXPIRED_AUTH_TOKEN);
        }
        Ok((namespace, role))
    }
}

impl Authorizer for RoomTokens {
    /// One of the tokens among `parameters` has to allow the operation in
    /// the namespace. A Joining FETCH (no namespace) is allowed without a
    /// token, since its SUBSCRIBE was asked about; a token it has is
    /// checked all the same, for its key and its expiry.
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
                let namespace_fits = namespace.is_none_or(|asked| {
                    let asked: Vec<&[u8]> = asked.fields().iter().map(Vec::as_slice).collect();
                    let named: Vec<&[u8]> = named.split('/').map(str::as_bytes).collect();
                    asked == named
                });
                if role_fits && namespace_fits {
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
        match (verdict, namespace) {
            (Some(Ok(())), _) | (None, None) => Ok(()),
            (Some(Err(error)), _) => Err(Denied::Request(error)),
            (None, Some(_)) => Err(Denied::Request(code::UNAUTHORIZED)),
        }
    }

    /// With `--auth-setup`: one of the tokens of the SETUP has to be the
    /// room's and not expired, or the session is closed, with
    /// `UNAUTHORIZED` ("The client is not authorized to establish a
    /// session", §3.5), `MALFORMED_AUTH_TOKEN` or `EXPIRED_AUTH_TOKEN`.
    ///
    /// The Alias Types in SETUP: "If a server receives Alias Type DELETE
    /// (0x0) or `USE_ALIAS` (0x2) in a SETUP message, it MUST close the
    /// session with a `PROTOCOL_VIOLATION`" (§10.2.2); a `REGISTER` that does
    /// not fit the cache, which is every one here, "MUST NOT fail the
    /// session ... it MUST treat the option as Alias Type `USE_VALUE`"
    /// (§10.3.1.4).
    fn setup(&self, setup_options: &[KeyValuePair]) -> Result<(), TerminationCode> {
        if !self.setup {
            return Ok(());
        }
        let mut verdict = Err(TerminationCode::Unauthorized);
        let mut seen = false;
        for option in setup_options {
            let Value::Bytes(bytes) = option.value() else {
                continue;
            };
            if option.ty() != SETUP_AUTHORIZATION_TOKEN {
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
            if this.is_ok() || !seen {
                verdict = this;
            }
            seen = true;
            if verdict.is_ok() {
                break;
            }
        }
        verdict
    }
}

/// The bytes a text of hexadecimal digits stands for.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len() / 2)
        .map(|at| u8::from_str_radix(text.get(at * 2..at * 2 + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
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
            // A Joining FETCH: allowed without a token and with the one of
            // its SUBSCRIBE; not with one that has expired.
            (vec![], Subscribe, false, Ok(())),
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
            // Without the flag the SETUP is not looked at.
            assert_eq!(room(false).setup(&options), Ok(()), "{tokens:?}");
        }
    }
}
