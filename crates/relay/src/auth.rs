//! An experiment in authorization: tokens a room hands out, checked with a
//! key the relay shares with the room. Off unless the relay is given a key
//! (`--auth-key-hex`), and no decision of the project: how the relay
//! authorizes is an open question.
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
//! FIXME: a token that is registered under an alias (§10.2.2, `REGISTER`,
//! `USE_ALIAS`, `DELETE`) is not kept: such a parameter is turned down. A
//! token in SETUP (§10.3.1.4) is not looked at. A subscription that was
//! allowed goes on after its token has expired.

use std::time::{SystemTime, UNIX_EPOCH};

use ring::hmac;
use xmoq_moqt::message::request_error_code as code;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::Parameter;
use xmoq_moqt::varint;
use xmoq_relay_core::{Authorize, Operation};

/// §10.2.2: "`USE_VALUE` (0x3): There is no Alias and there is a Type and
/// Value."
const USE_VALUE: u64 = 0x3;

/// The check the relay core asks before it takes a request, with `key`.
pub fn authorizer(key: &[u8]) -> Authorize {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    Authorize::new(move |operation, namespace, parameters| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        allowed(&key, now, operation, namespace, parameters)
    })
}

/// Whether one of the tokens among `parameters` allows the operation; else
/// the code of what was wrong with the first, or `UNAUTHORIZED` for a
/// request without a token.
fn allowed(
    key: &hmac::Key,
    now: u64,
    operation: Operation,
    namespace: &TrackNamespace,
    parameters: &[Parameter],
) -> Result<(), u64> {
    let mut first_error = None;
    for parameter in parameters {
        let Parameter::AuthorizationToken(token) = parameter else {
            continue;
        };
        match token_allows(key, now, operation, namespace, token) {
            Ok(()) => return Ok(()),
            Err(error) => first_error = first_error.or(Some(error)),
        }
    }
    Err(first_error.unwrap_or(code::UNAUTHORIZED))
}

fn token_allows(
    key: &hmac::Key,
    now: u64,
    operation: Operation,
    namespace: &TrackNamespace,
    token: &[u8],
) -> Result<(), u64> {
    let malformed = code::MALFORMED_AUTH_TOKEN;
    let (alias_type, read) = varint::decode(token).ok_or(malformed)?;
    let rest = token.get(read..).ok_or(malformed)?;
    let (token_type, read) = varint::decode(rest).ok_or(malformed)?;
    if alias_type != USE_VALUE || token_type != 0 {
        return Err(malformed);
    }
    let value = std::str::from_utf8(rest.get(read..).ok_or(malformed)?).map_err(|_| malformed)?;
    let (signed, mac) = value.rsplit_once('|').ok_or(malformed)?;
    let mut fields = signed.split('|');
    let (Some("v1"), Some(of_namespace), Some(role), Some(expiry), None) = (
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
    hmac::verify(key, signed.as_bytes(), &mac).map_err(|_| code::UNAUTHORIZED)?;
    if expiry <= now {
        return Err(code::EXPIRED_AUTH_TOKEN);
    }
    let asked: Vec<&[u8]> = namespace.fields().iter().map(Vec::as_slice).collect();
    let named: Vec<&[u8]> = of_namespace.split('/').map(str::as_bytes).collect();
    let role_fits = matches!(
        (role, operation),
        ("pub", Operation::Publish) | ("sub", Operation::Subscribe)
    );
    if asked == named && role_fits {
        Ok(())
    } else {
        Err(code::UNAUTHORIZED)
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
    const NOW: u64 = 1_800_000_000;

    /// The Token structure a room would hand out.
    fn token(key: &[u8], signed: &str) -> Parameter {
        let mac = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), signed.as_bytes());
        let digits: Vec<String> = mac
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let hex = digits.concat();
        let mut bytes = vec![0x03, 0x00];
        bytes.extend_from_slice(format!("{signed}|{hex}").as_bytes());
        Parameter::AuthorizationToken(bytes)
    }

    #[test]
    fn a_request_is_allowed_by_a_token_for_its_namespace_its_operation_and_its_time() {
        let namespace = TrackNamespace::new(vec![b"rooms".to_vec(), b"blue".to_vec()]).unwrap();
        let good = "v1|rooms/blue|sub|1800000060";
        let key = hmac::Key::new(hmac::HMAC_SHA256, KEY);
        // UNAUTHORIZED 0x1, MALFORMED_AUTH_TOKEN 0x4, EXPIRED_AUTH_TOKEN 0x5.
        // (the parameters of the request; what it asks; the verdict)
        let cases = [
            (vec![token(KEY, good)], Operation::Subscribe, Ok(())),
            (vec![], Operation::Subscribe, Err(0x1)),
            // Another operation, another namespace, a longer one.
            (vec![token(KEY, good)], Operation::Publish, Err(0x1)),
            (
                vec![token(KEY, "v1|rooms/red|sub|1800000060")],
                Operation::Subscribe,
                Err(0x1),
            ),
            (
                vec![token(KEY, "v1|rooms|sub|1800000060")],
                Operation::Subscribe,
                Err(0x1),
            ),
            // Expired, at the second of its expiry.
            (
                vec![token(KEY, "v1|rooms/blue|sub|1800000000")],
                Operation::Subscribe,
                Err(0x5),
            ),
            // Made with another key: whatever it says, also that it has
            // expired.
            (
                vec![token(b"another key", good)],
                Operation::Subscribe,
                Err(0x1),
            ),
            (
                vec![token(b"another key", "v1|rooms/blue|sub|1")],
                Operation::Subscribe,
                Err(0x1),
            ),
            // No Token structure of this kind.
            (
                vec![Parameter::AuthorizationToken(vec![])],
                Operation::Subscribe,
                Err(0x4),
            ),
            (
                vec![Parameter::AuthorizationToken(vec![0x01, 0x05, 0x00])],
                Operation::Subscribe,
                Err(0x4),
            ),
            (
                vec![token(KEY, "v2|rooms/blue|sub|1800000060")],
                Operation::Subscribe,
                Err(0x4),
            ),
            // One of two tokens allows it.
            (
                vec![token(KEY, "v1|rooms/red|sub|1800000060"), token(KEY, good)],
                Operation::Subscribe,
                Ok(()),
            ),
        ];
        for (parameters, operation, verdict) in cases {
            assert_eq!(
                allowed(&key, NOW, operation, &namespace, &parameters),
                verdict,
                "{parameters:?} {operation:?}"
            );
        }
    }
}
