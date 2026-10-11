//! The Token structure of the AUTHORIZATION TOKEN parameter and Setup
//! Option (§10.2.2, §10.3.1.4), decoded.
//!
//! What a token is good for is not this crate's: "The contents and
//! serialization of this payload are defined by the Token Type".

use crate::varint;

/// A Token structure, by its Alias Type (§10.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// DELETE (0x0): "There is an Alias but no Type or Value."
    Delete {
        /// Token Alias.
        alias: u64,
    },
    /// REGISTER (0x1): "There is an Alias, a Type and a Value."
    Register {
        /// Token Alias.
        alias: u64,
        /// Token Type.
        token_type: u64,
        /// Token Value.
        value: Vec<u8>,
    },
    /// `USE_ALIAS` (0x2): "There is an Alias but no Type or Value."
    UseAlias {
        /// Token Alias.
        alias: u64,
    },
    /// `USE_VALUE` (0x3): "There is no Alias and there is a Type and Value."
    UseValue {
        /// Token Type.
        token_type: u64,
        /// Token Value.
        value: Vec<u8>,
    },
}

impl Token {
    /// Decodes the value of an AUTHORIZATION TOKEN. `None` where it "cannot
    /// be decoded", for which "the receiver MUST close the Session with
    /// `KEY_VALUE_FORMATTING_ERROR`" (§10.2.2): an Alias Type the draft
    /// does not have, a number that is cut short, and bytes after an Alias
    /// that is all there is.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut rest = bytes;
        let mut number = || {
            let (value, read) = varint::decode(rest)?;
            rest = rest.get(read..)?;
            Some(value)
        };
        let alias_type = number()?;
        let token = match alias_type {
            0x0 => Self::Delete { alias: number()? },
            0x2 => Self::UseAlias { alias: number()? },
            0x1 => {
                let (alias, token_type) = (number()?, number()?);
                return Some(Self::Register {
                    alias,
                    token_type,
                    value: rest.to_vec(),
                });
            }
            0x3 => {
                let token_type = number()?;
                return Some(Self::UseValue {
                    token_type,
                    value: rest.to_vec(),
                });
            }
            _ => return None,
        };
        rest.is_empty().then_some(token)
    }

    /// Encodes it as the value of the parameter or the option.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Delete { alias } => {
                varint::encode(0x0, &mut out);
                varint::encode(*alias, &mut out);
            }
            Self::UseAlias { alias } => {
                varint::encode(0x2, &mut out);
                varint::encode(*alias, &mut out);
            }
            Self::Register {
                alias,
                token_type,
                value,
            } => {
                varint::encode(0x1, &mut out);
                varint::encode(*alias, &mut out);
                varint::encode(*token_type, &mut out);
                out.extend_from_slice(value);
            }
            Self::UseValue { token_type, value } => {
                varint::encode(0x3, &mut out);
                varint::encode(*token_type, &mut out);
                out.extend_from_slice(value);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by hand from Figure 5 of §10.2.2: no other implementation's
    /// bytes were at hand.
    #[test]
    fn a_token_structure_is_decoded_by_its_alias_type() {
        let value = || b"ab".to_vec();
        // (the bytes; what they are)
        let cases: [(&[u8], Option<Token>); 11] = [
            (&[0x00, 0x05], Some(Token::Delete { alias: 5 })),
            (
                &[0x01, 0x05, 0x00, 0x61, 0x62],
                Some(Token::Register {
                    alias: 5,
                    token_type: 0,
                    value: value(),
                }),
            ),
            (&[0x02, 0x05], Some(Token::UseAlias { alias: 5 })),
            (
                &[0x03, 0x00, 0x61, 0x62],
                Some(Token::UseValue {
                    token_type: 0,
                    value: value(),
                }),
            ),
            // A value of no bytes.
            (
                &[0x03, 0x07],
                Some(Token::UseValue {
                    token_type: 7,
                    value: vec![],
                }),
            ),
            // Nothing, an Alias Type there is none of, and what is cut short.
            (&[], None),
            (&[0x04, 0x00], None),
            (&[0x00], None),
            (&[0x01, 0x05], None),
            (&[0x03], None),
            // Bytes after an Alias that has no Type and no Value.
            (&[0x02, 0x05, 0x00], None),
        ];
        for (bytes, token) in cases {
            assert_eq!(Token::decode(bytes), token, "{bytes:?}");
            if let Some(token) = token {
                assert_eq!(token.encode(), bytes, "{token:?}");
            }
        }
    }
}
