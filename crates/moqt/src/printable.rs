//! Bytes a peer chose (a Track Namespace, a New Session URI), as text for a
//! line of a log or of stdout.
//!
//! WHY escaped: a line is read by what it starts with (the relay's log by
//! the interop scenes, `interop/disconnect.ts`; the Rust client's stdout by
//! the matrix), and a namespace may hold any byte (§2.4.1), a line break as
//! well, with which a peer could write a line of its own. A space would end
//! the value of `key=value`. WHY NOT `escape_ascii`: it leaves a space as it
//! is.

use std::fmt::Write;

use crate::name::TrackNamespace;

/// `bytes`, with what is not a visible ASCII character, and `\`, as `\xNN`.
#[must_use]
pub fn printable(bytes: &[u8]) -> String {
    escaped(bytes, b"")
}

/// The fields of `namespace`, each [`printable`] and a `/` in it as `\x2f`
/// as well, joined by `/`: `(a/b)` and `(a, b)` are told apart.
#[must_use]
pub fn printable_namespace(namespace: &TrackNamespace) -> String {
    namespace
        .fields()
        .iter()
        .map(|field| escaped(field, b"/"))
        .collect::<Vec<_>>()
        .join("/")
}

fn escaped(bytes: &[u8], also: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_graphic() && byte != b'\\' && !also.contains(&byte) {
            text.push(char::from(byte));
        } else {
            let _ = write!(text, "\\x{byte:02x}");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_could_end_a_line_or_a_value_is_escaped() {
        let cases: [(&[u8], &str); 6] = [
            (
                b"https://relay.example:4443/moq?sent-away=1",
                "https://relay.example:4443/moq?sent-away=1",
            ),
            (b"a\nCLOSE_SESSION", "a\\x0aCLOSE_SESSION"),
            (b"a b", "a\\x20b"),
            (b"a\\x0a", "a\\x5cx0a"),
            ("\u{e9}".as_bytes(), "\\xc3\\xa9"),
            (b"\r\t\x7f", "\\x0d\\x09\\x7f"),
        ];
        for (bytes, text) in cases {
            assert_eq!(printable(bytes), text, "{bytes:?}");
        }
    }

    #[test]
    fn the_fields_of_a_namespace_are_joined_by_a_slash_no_field_holds() {
        let cases: [(&[&[u8]], &str); 3] = [
            (&[b"xmoq", b"chat", b"r"], "xmoq/chat/r"),
            (&[b"a/b"], "a\\x2fb"),
            (
                &[b"a", b"x\nNAMESPACE_DONE suffix=b"],
                "a/x\\x0aNAMESPACE_DONE\\x20suffix=b",
            ),
        ];
        for (fields, text) in cases {
            let namespace =
                TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap();
            assert_eq!(printable_namespace(&namespace), text, "{fields:?}");
        }
    }
}
