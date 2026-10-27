//! The payloads of a room's reactions and of how many are in it. The shared
//! rows that the TypeScript side reads too are in
//! `fixtures/chat-tracks/`.
//!
//! `reacted`, a participant's, one Object a press:
//! `{"emoji": string, "sentAt": number}`.
//! `reactions`, the gatherer's, one Object a while that had some:
//! `{"counts": {emoji: number, ...}, "at": number}`.
//! `present`, the gatherer's, one Object a change: `{"count": number, "at":
//! number}`, without names.
//! Every number is a whole number up to [`MAX_SENT_AT`](crate::payloads::MAX_SENT_AT), the times in
//! milliseconds since the Unix epoch.

use std::fmt;

use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;

use crate::payloads::{json_string, members, whole};

/// The most UTF-16 units an emoji may have: one with its modifiers.
pub const MOST_EMOJI_UNITS: usize = 8;

/// Why a payload of `reacted`, `reactions` or `present` is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// Not UTF-8, not JSON, or no JSON object.
    NotAnObject,
    /// Of `reacted`: `emoji` is missing or no emoji ([`is_emoji`]).
    Emoji,
    /// Of `reacted`: `sentAt` is missing or no whole number up to
    /// [`MAX_SENT_AT`](crate::payloads::MAX_SENT_AT).
    SentAt,
    /// Of `reactions`: `counts` is missing, no object, empty, or has a
    /// member that is no emoji or whose count is no whole number from 1.
    Counts,
    /// Of `present`: `count` is missing or no whole number.
    Count,
    /// `at` is missing or no whole number.
    At,
}

impl Malformed {
    /// One word for the log, and of the shared tables.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotAnObject => "not-an-object",
            Self::Emoji => "emoji",
            Self::SentAt => "sent-at",
            Self::Counts => "counts",
            Self::Count => "count",
            Self::At => "at",
        }
    }
}

/// Whether `text` is taken as an emoji: 1 to [`MOST_EMOJI_UNITS`] UTF-16
/// units. WHY what it shows is not looked at: which sequences are emoji
/// changes with every version of Unicode, and a reader that knew fewer than
/// the sender would drop what the sender's page shows. WHY UTF-16 units:
/// they are what the pages count a string in.
#[must_use]
pub fn is_emoji(text: &str) -> bool {
    (1..=MOST_EMOJI_UNITS).contains(&text.encode_utf16().count())
}

/// A reaction as a participant said it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reacted {
    /// `emoji`.
    pub emoji: String,
    /// `sentAt`, the sender's clock, of which only the form is checked.
    pub sent_at: u64,
}

/// Reads a payload of `reacted`. Members other than the two are passed
/// over, as of `said`.
///
/// # Errors
///
/// What is wrong with it.
pub fn read_reacted(payload: &[u8]) -> Result<Reacted, Malformed> {
    let members = members(payload).ok_or(Malformed::NotAnObject)?;
    let emoji = match members.get("emoji") {
        Some(Value::String(emoji)) if is_emoji(emoji) => emoji.clone(),
        _ => return Err(Malformed::Emoji),
    };
    let sent_at = members
        .get("sentAt")
        .and_then(whole)
        .ok_or(Malformed::SentAt)?;
    Ok(Reacted { emoji, sent_at })
}

impl Reacted {
    /// The payload on `reacted`, `{"emoji":...,"sentAt":...}`.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        format!(
            r#"{{"emoji":{},"sentAt":{}}}"#,
            json_string(&self.emoji),
            self.sent_at
        )
        .into_bytes()
    }
}

/// The reactions of a room over one while, as the gatherer publishes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reactions {
    /// Each emoji and how often it came, in the order the payload has them.
    ///
    /// WHY a list and not a map: a map of `serde_json` sorts its keys unless
    /// the feature `preserve_order` is on, which would change every map of
    /// the workspace; the pages read the members in their order, and the
    /// first came first.
    pub counts: Vec<(String, u64)>,
    /// `at`, the gatherer's clock at the end of the while.
    pub at: u64,
}

/// Reads a payload of `reactions`. Members other than the two are passed
/// over. An emoji that is twice among `counts` is counted by the last, at
/// the place of the first, as `JSON.parse` reads it.
///
/// # Errors
///
/// What is wrong with it.
pub fn read_reactions(payload: &[u8]) -> Result<Reactions, Malformed> {
    let members = members(payload).ok_or(Malformed::NotAnObject)?;
    // The members are read again for `counts` alone, in their order: what
    // `members` holds of it is sorted.
    let counts = serde_json::from_slice::<CountsInOrder>(payload)
        .ok()
        .and_then(|read| read.0)
        .ok_or(Malformed::Counts)?;
    let counts = counts
        .into_iter()
        .map(|(emoji, count)| match whole(&count) {
            Some(count) if count >= 1 && is_emoji(&emoji) => Some((emoji, count)),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .filter(|counts| !counts.is_empty())
        .ok_or(Malformed::Counts)?;
    let at = members.get("at").and_then(whole).ok_or(Malformed::At)?;
    Ok(Reactions { counts, at })
}

impl Reactions {
    /// The payload on `reactions`, `{"counts":{...},"at":...}`, the counts
    /// in their order.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        let counts = self
            .counts
            .iter()
            .map(|(emoji, count)| format!("{}:{count}", json_string(emoji)))
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"counts":{{{counts}}},"at":{}}}"#, self.at).into_bytes()
    }
}

/// The members of `counts` of a payload, in their order; `None` where
/// `counts` is missing or no object. The last `counts` where there are two,
/// as `serde_json` and `JSON.parse` take it.
struct CountsInOrder(Option<Vec<(String, Value)>>);

impl<'de> Deserialize<'de> for CountsInOrder {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Payload;
        impl<'de> Visitor<'de> for Payload {
            type Value = CountsInOrder;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<CountsInOrder, A::Error> {
                let mut counts = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "counts" {
                        counts = map.next_value::<Members>()?.0;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(CountsInOrder(counts))
            }
        }
        deserializer.deserialize_map(Payload)
    }
}

/// The members of a JSON object in their order, a key that comes twice at
/// its first place with its last value; `None` for a value that is no
/// object.
struct Members(Option<Vec<(String, Value)>>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Object;
        impl<'de> Visitor<'de> for Object {
            type Value = Members;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any value")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members: Vec<(String, Value)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    match members.iter_mut().find(|(seen, _)| *seen == key) {
                        Some(member) => member.1 = value,
                        None => members.push((key, value)),
                    }
                }
                Ok(Members(Some(members)))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Members, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Members(None))
            }
            fn visit_str<E>(self, _: &str) -> Result<Members, E> {
                Ok(Members(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Members, E> {
                Ok(Members(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Members, E> {
                Ok(Members(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Members, E> {
                Ok(Members(None))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Members, E> {
                Ok(Members(None))
            }
            fn visit_unit<E>(self) -> Result<Members, E> {
                Ok(Members(None))
            }
        }
        deserializer.deserialize_any(Object)
    }
}

/// How many are in a room, as the gatherer publishes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Present {
    /// `count`: the participants the gatherer follows in the room, a name
    /// twice counted once.
    pub count: u64,
    /// `at`, the gatherer's clock when the count changed.
    pub at: u64,
}

/// Reads a payload of `present`. Members other than the two are passed
/// over: a `names` among them is not read.
///
/// # Errors
///
/// What is wrong with it.
pub fn read_present(payload: &[u8]) -> Result<Present, Malformed> {
    let members = members(payload).ok_or(Malformed::NotAnObject)?;
    let count = members
        .get("count")
        .and_then(whole)
        .ok_or(Malformed::Count)?;
    let at = members.get("at").and_then(whole).ok_or(Malformed::At)?;
    Ok(Present { count, at })
}

impl Present {
    /// The payload on `present`, `{"count":...,"at":...}`.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        format!(r#"{{"count":{},"at":{}}}"#, self.count, self.at).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column of the tables: `-` is none.
    fn column(text: &str) -> Option<&str> {
        (text != "-").then_some(text)
    }

    /// The rows of a table of six columns.
    fn rows(table: &str) -> Vec<[&str; 6]> {
        assert!(table.ends_with('\n') && !table.ends_with("\n\n"));
        table
            .lines()
            .map(|row| {
                row.split('\t')
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap_or_else(|_| panic!("a row of six columns: {row}"))
            })
            .collect()
    }

    /// Whether writing what was read gives the payload back, against the
    /// last column.
    fn written_back(case: &str, written: &[u8], payload: &str, expected: &str) {
        assert_eq!(written == payload.as_bytes(), expected == "yes", "{case}");
    }

    /// `fixtures/chat-tracks/reacted.tsv`, which the TypeScript reader reads too.
    #[test]
    fn a_payload_of_reacted_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/chat-tracks/reacted.tsv");
        for [case, payload, emoji, sent_at, malformed, written] in rows(table) {
            let read = read_reacted(payload.as_bytes());
            let expected = match column(malformed) {
                Some(word) => Err(word),
                None => Ok(Reacted {
                    emoji: serde_json::from_str(emoji).unwrap(),
                    sent_at: sent_at.parse().unwrap(),
                }),
            };
            assert_eq!(read.clone().map_err(Malformed::word), expected, "{case}");
            if let Ok(reacted) = read {
                written_back(case, &reacted.payload(), payload, written);
            }
        }
    }

    /// `fixtures/chat-tracks/reactions.tsv`, which the TypeScript reader
    /// reads too. The counts column is read with the same reader, as an
    /// object of its own.
    #[test]
    fn a_payload_of_reactions_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/chat-tracks/reactions.tsv");
        for [case, payload, counts, at, malformed, written] in rows(table) {
            let read = read_reactions(payload.as_bytes());
            let expected = if let Some(word) = column(malformed) {
                Err(word)
            } else {
                let counts = serde_json::from_str::<Members>(counts).unwrap().0.unwrap();
                Ok(Reactions {
                    counts: counts
                        .into_iter()
                        .map(|(emoji, count)| (emoji, count.as_u64().unwrap()))
                        .collect(),
                    at: at.parse().unwrap(),
                })
            };
            assert_eq!(read.clone().map_err(Malformed::word), expected, "{case}");
            if let Ok(reactions) = read {
                written_back(case, &reactions.payload(), payload, written);
            }
        }
    }

    /// `fixtures/chat-tracks/present.tsv`, which the TypeScript reader reads too.
    #[test]
    fn a_payload_of_present_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/chat-tracks/present.tsv");
        for [case, payload, count, at, malformed, written] in rows(table) {
            let read = read_present(payload.as_bytes());
            let expected = match column(malformed) {
                Some(word) => Err(word),
                None => Ok(Present {
                    count: count.parse().unwrap(),
                    at: at.parse().unwrap(),
                }),
            };
            assert_eq!(read.map_err(Malformed::word), expected, "{case}");
            if let Ok(present) = read {
                written_back(case, &present.payload(), payload, written);
            }
        }
    }

    #[test]
    fn counts_keep_their_order_and_an_emoji_twice_its_first_place() {
        let read = read_reactions(r#"{"counts":{"b":1,"a":2,"b":3},"at":5}"#.as_bytes());
        assert_eq!(
            read.map(|reactions| reactions.counts),
            Ok(vec![("b".to_string(), 3), ("a".to_string(), 2)])
        );
    }
}
