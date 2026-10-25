//! What was said in one piece of a voice, as the transcriber publishes it on
//! `speech` (decided on 2026-10-25): one Group a piece, numbered from 0
//! without a gap within the transcript, whose Object 0 is
//! `{"from": [string, ...], "text": string, "startUs": number, "endUs": number, "at": number}`,
//! in that order. The shared rows that every reader reads are in
//! `fixtures/transcript-tracks/`.
//!
//! `from` is the namespace of the broadcast whose voice it was, every field.
//! `startUs` and `endUs` are where the piece begins and ends on the clock of
//! the Timestamps of LOC of that broadcast's audio track, in microseconds;
//! `at` is the transcriber's clock when it had read the piece, in
//! milliseconds since the Unix epoch.
//!
//! WHY JSON: the chat's tracks are JSON, a page reads it (`@xmoq/json`),
//! and an agent reads it as it is. WHY the whole namespace in `from`, and
//! not the fields after the prefix: a transcript of one broadcast has a
//! prefix that is its whole namespace, which would leave nothing.

use serde_json::Value;

/// The largest number taken: 2^53 - 1, the largest integer a JavaScript
/// number holds exactly. The Timestamps of LOC are microseconds since the
/// Unix epoch in the demo, 2^50 now.
pub const MAX_NUMBER: u64 = (1 << 53) - 1;

/// A piece of speech, as it is published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// Its number in the transcript: the Group ID on `speech`.
    pub number: u64,
    /// `from`: the namespace of the broadcast, each field as UTF-8.
    pub from: Vec<String>,
    /// `text`: the words.
    pub text: String,
    /// `startUs`.
    pub start_us: u64,
    /// `endUs`, no less than `startUs`.
    pub end_us: u64,
    /// `at`.
    pub at: u64,
}

/// Why a payload of `speech` is no piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// Not UTF-8, not JSON, or no JSON object.
    NotAnObject,
    /// `from` is missing, no array, empty, or has a member that is no
    /// string or is empty.
    From,
    /// `text` is missing or no string.
    Text,
    /// `startUs` is missing, no integer, below 0 or above [`MAX_NUMBER`].
    StartUs,
    /// As `startUs`, or below `startUs`.
    EndUs,
    /// As `startUs`.
    At,
}

impl Malformed {
    /// One word for the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotAnObject => "not-an-object",
            Self::From => "from",
            Self::Text => "text",
            Self::StartUs => "start-us",
            Self::EndUs => "end-us",
            Self::At => "at",
        }
    }
}

/// Reads a payload of `speech`, the Object 0 of Group `number`. Members
/// other than the five are passed over, so that a transcriber may add some
/// without its pieces being lost.
///
/// # Errors
///
/// What is wrong with it.
pub fn read_speech(number: u64, payload: &[u8]) -> Result<Speech, Malformed> {
    let Ok(Value::Object(members)) = serde_json::from_slice::<Value>(payload) else {
        return Err(Malformed::NotAnObject);
    };
    let from = match members.get("from") {
        Some(Value::Array(fields)) if !fields.is_empty() => fields
            .iter()
            .map(|field| match field {
                Value::String(field) if !field.is_empty() => Ok(field.clone()),
                _ => Err(Malformed::From),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(Malformed::From),
    };
    let text = match members.get("text") {
        Some(Value::String(text)) => text.clone(),
        _ => return Err(Malformed::Text),
    };
    let start_us = number_of(&members, "startUs", Malformed::StartUs)?;
    let end_us = number_of(&members, "endUs", Malformed::EndUs)?;
    if end_us < start_us {
        return Err(Malformed::EndUs);
    }
    Ok(Speech {
        number,
        from,
        text,
        start_us,
        end_us,
        at: number_of(&members, "at", Malformed::At)?,
    })
}

/// `as_u64` is `None` for a number that was written with a fraction or an
/// exponent, and for one below 0.
fn number_of(
    members: &serde_json::Map<String, Value>,
    key: &str,
    wrong: Malformed,
) -> Result<u64, Malformed> {
    members
        .get(key)
        .and_then(Value::as_u64)
        .filter(|number| *number <= MAX_NUMBER)
        .ok_or(wrong)
}

/// A string as JSON writes it, with its quotes.
fn json_string(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

impl Speech {
    /// The payload of the piece on `speech`.
    ///
    /// WHY written by hand and not with a map of `serde_json`: the members
    /// are in the order of the decision whatever the features of
    /// `serde_json` sort a map by, and a reader that shows the beginning of
    /// a payload alone (the Rust client's `FETCH_OBJECT`) shows who spoke.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        let from: Vec<String> = self.from.iter().map(|field| json_string(field)).collect();
        format!(
            r#"{{"from":[{}],"text":{},"startUs":{},"endUs":{},"at":{}}}"#,
            from.join(","),
            json_string(&self.text),
            self.start_us,
            self.end_us,
            self.at
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fixtures/transcript-tracks/speech.tsv`, which every reader reads.
    #[test]
    fn a_payload_of_speech_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/transcript-tracks/speech.tsv");
        for row in table.lines() {
            let [
                case,
                group,
                payload,
                from,
                text,
                start_us,
                end_us,
                at,
                malformed,
                written,
            ] = row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("{row}");
            };
            let number = group.parse().unwrap();
            let read = read_speech(number, payload.as_bytes());
            let expected = if malformed == "-" {
                Ok(Speech {
                    number,
                    from: serde_json::from_str(from).unwrap(),
                    text: serde_json::from_str(text).unwrap(),
                    start_us: start_us.parse().unwrap(),
                    end_us: end_us.parse().unwrap(),
                    at: at.parse().unwrap(),
                })
            } else {
                Err(malformed)
            };
            assert_eq!(read.clone().map_err(Malformed::word), expected, "{case}");
            if let Ok(speech) = read {
                assert_eq!(
                    speech.payload() == payload.as_bytes(),
                    written == "yes",
                    "{case}"
                );
            }
        }
    }

    /// Bytes that are not UTF-8, which the table, being text, cannot hold.
    #[test]
    fn a_payload_that_is_not_utf_8_is_no_piece() {
        let payload = b"{\"from\":[\"a\"],\"text\":\"\xff\",\"startUs\":1,\"endUs\":2,\"at\":3}";
        assert_eq!(read_speech(0, payload), Err(Malformed::NotAnObject));
    }
}
