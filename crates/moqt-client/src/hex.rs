//! Hexadecimal text for command-line arguments and printed bytes.

use std::fmt::Write;

use anyhow::{Context, Result, bail};

pub fn encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub fn decode(input: &str) -> Result<Vec<u8>> {
    // WHY check the digits first: `u8::from_str_radix` also accepts a sign, so
    // "+1" would pass as 0x01.
    if !input.len().is_multiple_of(2) || !input.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("not a hex string");
    }
    (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).context("invalid hex digit"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_lowercase_pairs() {
        let cases: &[(&[u8], &str)] = &[(&[], ""), (&[0x00, 0x0a, 0xff], "000aff")];
        for (bytes, text) in cases {
            assert_eq!(encode(bytes), *text);
        }
    }

    #[test]
    fn decodes_what_it_encodes_and_rejects_the_rest() {
        let cases: &[(&str, Option<&[u8]>)] = &[
            ("", Some(&[])),
            ("000aFF", Some(&[0x00, 0x0a, 0xff])),
            ("0", None),
            ("+1", None),
            ("zz", None),
            ("あ", None),
        ];
        for (text, bytes) in cases {
            assert_eq!(decode(text).ok().as_deref(), *bytes, "{text:?}");
        }
    }
}
