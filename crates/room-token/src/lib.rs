//! The token of a room (the experiment in authorization of the relay,
//! `crates/relay/src/auth.rs`): the text
//!
//! ```text
//! v1|<namespace, its fields joined with />|<pub or sub>|<expiry, seconds since 1970>|<mac, hex>
//! ```
//!
//! where the mac is HMAC-SHA-256, with the key the relay shares with the
//! room, over the text before the last `|`. The relay reads it; the
//! gatherer of the chat, the transcriber and the relay itself make their
//! own ([`RoomKey::authorization_token`]) with a key the relay shares with
//! them (provisional), as the room (`@xmoq/room`,
//! `packages/room/src/token.ts`) does.
//!
//! The key, and making and reading a token with it, are of the feature
//! `keys` (on by default). WHY a feature: they need HMAC-SHA-256 of `ring`,
//! which builds C, and `xmoq-viewer`, which does no input or output, takes
//! only [`Role`] and [`namespace_fits`] (`default-features = false`).
//!
//! WHY one crate for the Rust ones that make it and read it: one function
//! for each half leaves no second copy of the format to drift. The room's
//! is TypeScript: both read `fixtures/room-token/tokens.tsv`.

#[cfg(feature = "keys")]
use std::fmt::Write;

#[cfg(feature = "keys")]
use ring::hmac;
use xmoq_moqt::name::TrackNamespace;
#[cfg(feature = "keys")]
use xmoq_moqt::token::Token;

/// How long a token a Rust part makes for itself runs
/// ([`RoomKey::authorization_token`]), in seconds: as long as one the room
/// makes (`TOKEN_SECONDS` of `packages/room/src/token.ts`). Provisional.
#[cfg(feature = "keys")]
pub const TOKEN_SECONDS: u64 = 600;

/// What a token allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// PUBLISH and `PUBLISH_NAMESPACE`.
    Pub,
    /// SUBSCRIBE, FETCH and `SUBSCRIBE_NAMESPACE`.
    Sub,
}

impl Role {
    /// The role as a token says it, and as the room is asked for one.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Pub => "pub",
            Self::Sub => "sub",
        }
    }
}

/// What a token says, once its mac is the key's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant<'a> {
    /// The namespace, its fields joined with `/` ([`namespace_fits`]).
    pub namespace: &'a str,
    /// `pub` or `sub`, as it came.
    pub role: &'a str,
    /// Seconds since 1970.
    pub expiry: u64,
}

/// Why a text is no token of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Not of the form above.
    Malformed,
    /// Its mac is not the key's: whatever it says is not believed.
    NotOfTheKey,
}

/// The key the relay shares with the room.
#[cfg(feature = "keys")]
pub struct RoomKey(hmac::Key);

#[cfg(feature = "keys")]
impl RoomKey {
    /// The key of these bytes.
    #[must_use]
    pub fn new(key: &[u8]) -> Self {
        Self(hmac::Key::new(hmac::HMAC_SHA256, key))
    }

    /// The token that allows `role` in `namespace` (its fields joined with
    /// `/`) until `expiry`.
    #[must_use]
    pub fn make(&self, namespace: &str, role: Role, expiry: u64) -> String {
        let signed = format!("v1|{namespace}|{}|{expiry}", role.text());
        let mac = hmac::sign(&self.0, signed.as_bytes());
        let digits = mac.as_ref().iter().fold(String::new(), |mut digits, byte| {
            let _ = write!(digits, "{byte:02x}");
            digits
        });
        format!("{signed}|{digits}")
    }

    /// The token that allows `role` in `namespace` for [`TOKEN_SECONDS`]
    /// from `now_s` (seconds since 1970), as the value of an AUTHORIZATION
    /// TOKEN (§10.2.2: Alias Type `USE_VALUE`, Token Type 0, "negotiated
    /// out-of-band").
    #[must_use]
    pub fn authorization_token(&self, namespace: &str, role: Role, now_s: u64) -> Vec<u8> {
        let text = self.make(namespace, role, now_s.saturating_add(TOKEN_SECONDS));
        Token::UseValue {
            token_type: 0,
            value: text.into_bytes(),
        }
        .encode()
    }

    /// What `value` says, where the key made it. The expiry is not looked
    /// at: what it means is the reader's.
    ///
    /// # Errors
    ///
    /// [`Fault`].
    pub fn read<'a>(&self, value: &'a [u8]) -> Result<Grant<'a>, Fault> {
        let value = std::str::from_utf8(value).map_err(|_| Fault::Malformed)?;
        let (signed, mac) = value.rsplit_once('|').ok_or(Fault::Malformed)?;
        let mut fields = signed.split('|');
        let (Some("v1"), Some(namespace), Some(role), Some(expiry), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(Fault::Malformed);
        };
        let expiry: u64 = expiry.parse().map_err(|_| Fault::Malformed)?;
        let mac = unhex(mac).ok_or(Fault::Malformed)?;
        // Before anything the token says is believed.
        hmac::verify(&self.0, signed.as_bytes(), &mac).map_err(|_| Fault::NotOfTheKey)?;
        Ok(Grant {
            namespace,
            role,
            expiry,
        })
    }
}

/// The bytes a text of hexadecimal digits stands for.
#[must_use]
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len() / 2)
        .map(|at| u8::from_str_radix(text.get(at * 2..at * 2 + 2)?, 16).ok())
        .collect()
}

/// The bytes of the key given as `--auth-key-hex`: none for text that is not
/// hex, or is empty. WHY not an empty key: anybody makes the mac of a key of
/// no bytes, so an empty flag (a variable left unset) would have a relay
/// that lets everybody in look like one that asks for tokens. The room
/// (`keyOfSetting` of `packages/room/src/token.ts`) stops for an
/// `AUTH_KEY_HEX` that is set and is none as well.
#[must_use]
pub fn key_of_hex(text: &str) -> Option<Vec<u8>> {
    unhex(text).filter(|key| !key.is_empty())
}

/// Whether the namespace a token names, its fields joined with `/`, allows
/// the namespace `asked`: the same fields, or, where the last field of the
/// token is `*`, the fields before it, followed by any or none. Field by
/// field, not as text: `xmoq/chat/*` allows `xmoq/chat` and
/// `xmoq/chat/blue/a`, and not `xmoq/chatter`. Decided on 2026-10-24, for the chat
/// of a room.
///
/// WHY also the namespace before the `*` itself (decided on 2026-10-24): the
/// gatherer asks for the participants with a `SUBSCRIBE_NAMESPACE` whose
/// prefix is `xmoq/chat` itself (`crates/chat-tracks`), which is checked as
/// `sub` of that namespace, and the room's `lines` are a track of
/// `xmoq/chat/<room>`. WHY NOT one field or more, as `bucket/*` of AWS IAM
/// and `foo.>` of NATS have it: each of those would take a second token,
/// whole, for the same grant. WHY NOT a `*` in another field: nothing asks
/// for it; such a field is the text `*`, as before.
#[must_use]
pub fn namespace_fits(asked: &TrackNamespace, named: &str) -> bool {
    let asked: Vec<&[u8]> = asked.fields().iter().map(Vec::as_slice).collect();
    let named: Vec<&[u8]> = named.split('/').map(str::as_bytes).collect();
    match named.split_last() {
        Some((&b"*", before)) => asked.starts_with(before),
        _ => asked == named,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fixtures/room-token/tokens.tsv`, which `@xmoq/room` reads as well.
    #[cfg(feature = "keys")]
    #[test]
    fn a_token_is_its_text_and_the_mac_of_it() {
        let table = include_str!("../../../fixtures/room-token/tokens.tsv");
        let mut rows = 0;
        for line in table.lines() {
            let columns: Vec<&str> = line.split('\t').collect();
            let [name, key, namespace, role, expiry, token] = columns[..] else {
                panic!("a row of six columns: {line:?}");
            };
            let key = RoomKey::new(&unhex(key).unwrap());
            let role_of = match role {
                "pub" => Role::Pub,
                "sub" => Role::Sub,
                other => panic!("{name}: no role {other}"),
            };
            let expiry: u64 = expiry.parse().unwrap();
            assert_eq!(key.make(namespace, role_of, expiry), token, "{name}");
            assert_eq!(
                key.read(token.as_bytes()),
                Ok(Grant {
                    namespace,
                    role,
                    expiry
                }),
                "{name}"
            );
            assert_eq!(
                RoomKey::new(b"another key").read(token.as_bytes()),
                Err(Fault::NotOfTheKey),
                "{name}"
            );
            rows += 1;
        }
        assert_eq!(rows, 3);
    }

    /// Rows of `fixtures/room-token/tokens.tsv`, made [`TOKEN_SECONDS`]
    /// before their expiry, after the Alias Type `USE_VALUE` (0x03) and the
    /// Token Type 0.
    #[cfg(feature = "keys")]
    #[test]
    fn an_authorization_token_is_a_use_value_of_the_token_that_runs_for_token_seconds() {
        let key = RoomKey::new(b"a key of the room");
        // (the namespace; the role; now; the bytes)
        let cases: [(&str, Role, u64, &[u8]); 2] = [
            (
                "xmoq/chat/*",
                Role::Sub,
                1_799_999_460,
                b"\x03\x00v1|xmoq/chat/*|sub|1800000060|53749e09d846bd5dcf000b8c12fc683114d5d538966d74056e43e21102604116",
            ),
            (
                "xmoq/meet/blue/ann",
                Role::Pub,
                1_800_000_000,
                b"\x03\x00v1|xmoq/meet/blue/ann|pub|1800000600|ed2d81f6d427ae030fda6a5d8d277820115b90f419d28b77714381aa9aa7db91",
            ),
        ];
        for (namespace, role, now_s, bytes) in cases {
            assert_eq!(
                key.authorization_token(namespace, role, now_s),
                bytes,
                "{namespace}"
            );
        }
    }

    #[test]
    fn a_key_is_hex_of_one_byte_at_least() {
        let cases: [(&str, Option<&[u8]>); 5] = [
            ("00ff", Some(&[0x00, 0xff])),
            ("AB", Some(&[0xab])),
            ("", None),
            ("abc", None),
            ("zz", None),
        ];
        for (text, key) in cases {
            assert_eq!(key_of_hex(text).as_deref(), key, "{text:?}");
        }
    }

    #[test]
    fn a_token_whose_last_field_is_a_star_allows_the_namespaces_under_it() {
        // (the namespace of the token; the namespace asked; allowed)
        // A namespace has one field at least (§2.4.1), so `*` alone allows
        // every one.
        let cases = [
            ("xmoq/chat/blue", "xmoq/chat/blue", true),
            ("xmoq/chat/blue", "xmoq/chat/blue/a", false),
            ("xmoq/chat/blue/*", "xmoq/chat/blue/a", true),
            ("xmoq/chat/blue/*", "xmoq/chat/blue/a/b", true),
            ("xmoq/chat/blue/*", "xmoq/chat/blue", true),
            ("xmoq/chat/*", "xmoq/chat", true),
            ("xmoq/chat/*", "xmoq/chatter", false),
            ("xmoq/chat/blue/*", "xmoq/chat/blueberry/a", false),
            ("xmoq/chat/blue/*", "xmoq/chat/red/a", false),
            ("xmoq/chat/blue/*", "xmoq/chat", false),
            ("*", "xmoq", true),
            // Elsewhere a star is a field like any other.
            ("xmoq/*/blue", "xmoq/chat/blue", false),
            ("xmoq/*/blue", "xmoq/*/blue", true),
        ];
        for (named, asked, allowed) in cases {
            let fields = asked
                .split('/')
                .map(|field| field.as_bytes().to_vec())
                .collect();
            let asked_namespace = TrackNamespace::new(fields).unwrap();
            assert_eq!(
                namespace_fits(&asked_namespace, named),
                allowed,
                "{named} {asked}"
            );
        }
    }
}
