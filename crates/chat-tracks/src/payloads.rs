//! What a line says, as a participant sends it on `said` and as the gatherer
//! sends it on `lines`. The shared rows
//! that the TypeScript side reads too are in `fixtures/chat-tracks/`.
//!
//! `said`, one Object a Group: `{"text": string, "sentAt": number}`.
//! `lines`: `{"name": string, "text": string, "sentAt": number, "at": number}`,
//! in that order. `sentAt` is the sender's clock and `at` the gatherer's, in
//! milliseconds since the Unix epoch.

use serde_json::Value;

/// The largest `sentAt` taken: 2^53 - 1, the largest integer a JavaScript
/// number holds exactly. WHY that bound: the pages read the lines with
/// `JSON.parse`, and a larger value would reach them as another number.
pub const MAX_SENT_AT: u64 = (1 << 53) - 1;

/// A line as its sender said it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentLine {
    /// `text`.
    pub text: String,
    /// `sentAt`, which only its form is checked of.
    pub sent_at: u64,
}

/// Why a payload of `said` or `lines` is no line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// Not UTF-8, not JSON, or no JSON object.
    NotAnObject,
    /// `text` is missing or no string.
    Text,
    /// `sentAt` is missing, no integer, below 0 or above [`MAX_SENT_AT`].
    SentAt,
    /// Of `lines` only: `name` is missing or no string.
    Name,
    /// Of `lines` only: `at` is missing, no integer, below 0 or above
    /// [`MAX_SENT_AT`].
    At,
}

impl Malformed {
    /// One word for the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotAnObject => "not-an-object",
            Self::Text => "text",
            Self::SentAt => "sent-at",
            Self::Name => "name",
            Self::At => "at",
        }
    }
}

/// Reads a payload of `said`. Members other than the two are passed over,
/// so that a sender may add some without its lines being lost.
///
/// WHY `sentAt` is checked for its form and nothing else: it is the sender's
/// clock, which nobody else can check, and the order of the lines is the
/// order they reach the gatherer (decided on 2026-10-24). An integer within
/// the bound is passed on as the sender gave it.
///
/// # Errors
///
/// What is wrong with it.
pub fn read_said(payload: &[u8]) -> Result<SentLine, Malformed> {
    let members = object(payload)?;
    Ok(SentLine {
        text: string(&members, "text", Malformed::Text)?,
        sent_at: time(&members, "sentAt", Malformed::SentAt)?,
    })
}

impl SentLine {
    /// The payload of the line on `said`, `{"text":...,"sentAt":...}`.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        format!(
            r#"{{"text":{},"sentAt":{}}}"#,
            json_string(&self.text),
            self.sent_at
        )
        .into_bytes()
    }
}

/// Reads a payload of `lines`, the Object 0 of Group `number`. Members other
/// than the four are passed over, as in [`read_said`]. The times are held to
/// the same form, which the gatherer writes them in.
///
/// # Errors
///
/// What is wrong with it.
pub fn read_line(number: u64, payload: &[u8]) -> Result<Line, Malformed> {
    let members = object(payload)?;
    Ok(Line {
        number,
        name: string(&members, "name", Malformed::Name)?,
        text: string(&members, "text", Malformed::Text)?,
        sent_at: time(&members, "sentAt", Malformed::SentAt)?,
        at: time(&members, "at", Malformed::At)?,
    })
}

/// The members of a payload that is a JSON object; `None` for any other.
pub(crate) fn members(payload: &[u8]) -> Option<serde_json::Map<String, Value>> {
    match serde_json::from_slice::<Value>(payload) {
        Ok(Value::Object(members)) => Some(members),
        _ => None,
    }
}

fn object(payload: &[u8]) -> Result<serde_json::Map<String, Value>, Malformed> {
    members(payload).ok_or(Malformed::NotAnObject)
}

fn string(
    members: &serde_json::Map<String, Value>,
    key: &str,
    missing: Malformed,
) -> Result<String, Malformed> {
    match members.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(missing),
    }
}

/// `as_u64` is `None` for a number that was written with a fraction or an
/// exponent, and for one below 0: neither is a time in milliseconds.
fn time(
    members: &serde_json::Map<String, Value>,
    key: &str,
    wrong: Malformed,
) -> Result<u64, Malformed> {
    members.get(key).and_then(whole).ok_or(wrong)
}

/// A whole number from 0 to [`MAX_SENT_AT`]; `None` for any other value.
pub(crate) fn whole(value: &Value) -> Option<u64> {
    value.as_u64().filter(|number| *number <= MAX_SENT_AT)
}

/// A string as JSON writes it, with its quotes.
pub(crate) fn json_string(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

/// A line of a room, as it is kept and published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Its number in the room: the Group ID on `lines`.
    pub number: u64,
    /// Who said it: the last field of the sender's namespace.
    pub name: String,
    /// What was said.
    pub text: String,
    /// The sender's clock.
    pub sent_at: u64,
    /// The gatherer's clock when it took the line.
    pub at: u64,
}

impl Line {
    /// The payload of the line on `lines`.
    ///
    /// WHY written by hand and not with a map of `serde_json`: the members
    /// are in the order of the decision, whatever the features of
    /// `serde_json` sort a map by, and a reader that shows the beginning of
    /// a payload alone (the Rust client's `FETCH_OBJECT`) shows who spoke.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        format!(
            r#"{{"name":{},"text":{},"sentAt":{},"at":{}}}"#,
            json_string(&self.name),
            json_string(&self.text),
            self.sent_at,
            self.at
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column of the tables: `-` is none.
    fn column(text: &str) -> Option<&str> {
        (text != "-").then_some(text)
    }

    /// A text column: a JSON string literal.
    fn text_of(literal: &str) -> String {
        serde_json::from_str(literal).unwrap()
    }

    /// `fixtures/chat-tracks/said.tsv`, which the TypeScript reader reads too.
    #[test]
    fn a_payload_of_said_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/chat-tracks/said.tsv");
        for row in table.lines() {
            let [case, payload, text, sent_at, malformed, written] =
                row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("{row}");
            };
            let read = read_said(payload.as_bytes());
            let expected = match column(malformed) {
                Some(word) => Err(word),
                None => Ok(SentLine {
                    text: text_of(text),
                    sent_at: sent_at.parse().unwrap(),
                }),
            };
            assert_eq!(read.clone().map_err(Malformed::word), expected, "{case}");
            if let Ok(said) = read {
                assert_eq!(
                    said.payload() == payload.as_bytes(),
                    written == "yes",
                    "{case}"
                );
            }
        }
    }

    /// `fixtures/chat-tracks/lines.tsv`, which the TypeScript reader reads too.
    #[test]
    fn a_payload_of_lines_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/chat-tracks/lines.tsv");
        for row in table.lines() {
            let [
                case,
                group,
                payload,
                name,
                text,
                sent_at,
                at,
                malformed,
                written,
            ] = row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("{row}");
            };
            let number = group.parse().unwrap();
            let read = read_line(number, payload.as_bytes());
            let expected = match column(malformed) {
                Some(word) => Err(word),
                None => Ok(Line {
                    number,
                    name: text_of(name),
                    text: text_of(text),
                    sent_at: sent_at.parse().unwrap(),
                    at: at.parse().unwrap(),
                }),
            };
            assert_eq!(read.clone().map_err(Malformed::word), expected, "{case}");
            if let Ok(line) = read {
                assert_eq!(
                    line.payload() == payload.as_bytes(),
                    written == "yes",
                    "{case}"
                );
            }
        }
    }

    /// Bytes that are not UTF-8, a byte order mark, and a control character
    /// as it is in a string, which the tables, being text, cannot hold. The
    /// TypeScript reader has the same cases (`packages/chat/test/payloads.test.ts`).
    #[test]
    fn a_payload_the_tables_cannot_hold_is_no_line() {
        let cases: [(&str, &[u8], Malformed); 4] = [
            (
                "said",
                b"\xef\xbb\xbf{\"text\":\"x\",\"sentAt\":5}",
                Malformed::NotAnObject,
            ),
            (
                "said",
                b"{\"text\":\"a\tb\",\"sentAt\":5}",
                Malformed::NotAnObject,
            ),
            (
                "said",
                b"{\"text\":\"\xff\",\"sentAt\":5}",
                Malformed::NotAnObject,
            ),
            (
                "lines",
                b"{\"name\":\"\xff\",\"text\":\"t\",\"sentAt\":1,\"at\":2}",
                Malformed::NotAnObject,
            ),
        ];
        for (case, payload, expected) in cases {
            let read = if case == "said" {
                read_said(payload).err()
            } else {
                read_line(0, payload).err()
            };
            assert_eq!(read, Some(expected), "{case}");
        }
    }
}
