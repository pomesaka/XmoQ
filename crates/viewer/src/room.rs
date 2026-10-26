//! What a room says, and what is said to it: the messages of the room's
//! WebSocket (`packages/room`), as text.

use serde_json::{Value, json};

/// The most characters of a line of the chat that are shown.
pub const LINE_LIMIT: usize = 120;

/// Where a broadcast is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broadcast {
    /// The address of the relay.
    pub relay_url: String,
    /// The SHA-256 of the relay's certificate, as hex.
    pub cert_sha256_hex: String,
    /// The fields of the Track Namespace.
    pub namespace: Vec<String>,
}

/// Where the relay of the rooms is, as a room says it in its hello with a
/// broadcast and without one (decided on 2026-10-25; why, at the top of
/// `packages/room/src/worker.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    /// The address of the relay.
    pub relay_url: String,
    /// The SHA-256 of the relay's certificate, as hex.
    pub cert_sha256_hex: String,
}

/// What a room said.
///
/// The chat is not among it: it is on `MoQ` tracks (decided on
/// 2026-10-24; [`crate::chat_track`]). A room that still sends its chat, or
/// a history of it in its hello, has it passed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// The first message: the broadcast the room has.
    Hello {
        /// The broadcast, where a publisher is in the room.
        broadcast: Option<Broadcast>,
        /// The relay of the rooms, where the room says it.
        relay: Option<Relay>,
        /// The prefix a transcriber listens under for the room's broadcast,
        /// its fields, where the room says it: its transcript is `speech` of
        /// `xmoq/transcript/<prefix>` (decided on 2026-10-25; why, at the top
        /// of `packages/room/src/worker.ts`).
        transcript: Option<Vec<String>>,
        /// The same for the room's meeting (`meetingTranscript`).
        meeting_transcript: Option<Vec<String>>,
    },
    /// A publisher came, or the one that was there left.
    Broadcast(Option<Broadcast>),
    /// The answer to [`ask_for_a_token`]: the token as text, or none where
    /// the room has no key.
    Token(Option<String>),
    /// The answer to [`ask_for_a_seat`] where the meeting had room.
    Seat(Seat),
    /// The answer to [`ask_for_a_seat`] where the meeting was full: how many
    /// it takes.
    MeetingFull(u64),
    /// The answer to [`ask_for_a_seat`] where the room takes no such seat:
    /// why, in the room's words (`SeatRefusal` of
    /// `packages/room/src/core/rules.ts`).
    MeetingRefused(String),
    /// Something the viewer has no use for.
    Other,
}

/// A participant's seat in a room's meeting (decided on 2026-10-25):
/// what of the room's `meeting` a reader that announces nothing there has
/// a use for. Its `publish`, the participant's own namespace, is passed
/// over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    /// The participant's id, as it was asked for.
    pub participant: String,
    /// The namespace `subscribe` allows, its fields: `xmoq/meet/<room>/*`.
    pub subscribe: Vec<String>,
    /// `sub` in it, or none where the room has no key.
    pub subscribe_token: Option<String>,
}

/// What is no message of a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// Whether a character changes how the text around it is laid out without
/// being seen: the marks, embeddings, overrides and isolates of the
/// bidirectional algorithm, the separators of lines and paragraphs, and the
/// byte order mark.
fn lays_out(char: char) -> bool {
    matches!(
        char,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// A text with nothing a terminal would act on, and no longer than `most`
/// characters.
///
/// WHY: a line of the chat is anybody's text, and so is a track's name and
/// what a relay says of itself; each is written to a terminal, where an
/// escape code moves the cursor, changes the title or worse, and a
/// right-to-left override shows a text as another than it is.
#[must_use]
pub fn clean(text: &str, most: usize) -> String {
    text.chars()
        .filter(|char| !char.is_control() && !lays_out(*char))
        .take(most)
        .collect()
}

/// The fields of a namespace: strings, none empty, and at least one.
fn fields_of(value: &Value) -> Option<Vec<String>> {
    let fields: Vec<String> = value
        .as_array()?
        .iter()
        .map(|field| field.as_str().map(str::to_string))
        .collect::<Option<_>>()?;
    (!fields.is_empty() && fields.iter().all(|field| !field.is_empty())).then_some(fields)
}

fn relay_of(value: &Value) -> Option<Relay> {
    let relay_url = value.get("relayUrl")?.as_str()?;
    // WHY https alone: it is what a relay is reached by (WebTransport), and
    // an address of any other kind from a room is not one to hand a token to.
    if !relay_url.starts_with("https://") {
        return None;
    }
    Some(Relay {
        relay_url: relay_url.to_string(),
        cert_sha256_hex: value.get("certSha256Hex")?.as_str()?.to_string(),
    })
}

fn seat_of(message: &Value) -> Option<Seat> {
    let subscribe = message.get("subscribe")?;
    Some(Seat {
        participant: message.get("participant")?.as_str()?.to_string(),
        subscribe: fields_of(subscribe.get("namespace")?)?,
        subscribe_token: subscribe
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn broadcast_of(value: &Value) -> Option<Broadcast> {
    let Relay {
        relay_url,
        cert_sha256_hex,
    } = relay_of(value)?;
    Some(Broadcast {
        relay_url,
        cert_sha256_hex,
        namespace: value
            .get("namespace")?
            .as_array()?
            .iter()
            .map(|field| field.as_str().map(str::to_string))
            .collect::<Option<_>>()?,
    })
}

/// Reads one message of a room.
///
/// A broadcast or a relay that lacks a field, has one of another type, or
/// names a relay by anything but `https://`, is read as none: there is
/// nowhere to connect to.
///
/// # Errors
///
/// [`Invalid`] for text that is no JSON.
pub fn read(text: &str) -> Result<Said, Invalid> {
    let message: Value =
        serde_json::from_str(text).map_err(|error| Invalid(format!("no JSON: {error}")))?;
    Ok(match message.get("type").and_then(Value::as_str) {
        Some("hello") => Said::Hello {
            broadcast: message.get("broadcast").and_then(broadcast_of),
            relay: message.get("relay").and_then(relay_of),
            transcript: message.get("transcript").and_then(fields_of),
            meeting_transcript: message.get("meetingTranscript").and_then(fields_of),
        },
        Some("broadcast") => Said::Broadcast(message.get("broadcast").and_then(broadcast_of)),
        Some("token") => Said::Token(
            message
                .get("token")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        // A seat without its participant or its namespace is none: there is
        // nothing to subscribe to.
        Some("meeting") => seat_of(&message).map_or(Said::Other, Said::Seat),
        Some("meeting-full") => message
            .get("most")
            .and_then(Value::as_u64)
            .map_or(Said::Other, Said::MeetingFull),
        Some("meeting-refused") => message
            .get("why")
            .and_then(Value::as_str)
            .map_or(Said::Other, |why| Said::MeetingRefused(why.to_string())),
        _ => Said::Other,
    })
}

/// What a token of a room allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRole {
    /// Watching, and the room's chat lines.
    Sub,
    /// The viewer's own chat namespace.
    Pub,
}

/// The message that asks a room for a token for `role` in a namespace.
#[must_use]
pub fn ask_for_a_token(role: TokenRole, namespace: &[String]) -> String {
    let role = match role {
        TokenRole::Sub => "sub",
        TokenRole::Pub => "pub",
    };
    json!({"type": "token-for", "role": role, "namespace": namespace}).to_string()
}

/// The message that asks a room for a seat in its meeting as `participant`,
/// a random id the asker made (decided on 2026-10-25).
#[must_use]
pub fn ask_for_a_seat(participant: &str) -> String {
    json!({"type": "join-meeting", "participant": participant}).to_string()
}

/// The address of a room's WebSocket, from the address of the room
/// (`http://host:port/rooms/<name>`) and the name to be known by there.
///
/// WHY only letters and digits of the name are kept: it goes into the
/// address as it is.
///
/// # Errors
///
/// [`Invalid`] for an address that does not begin with `http`.
pub fn websocket_address(room: &str, name: &str) -> Result<String, Invalid> {
    let rest = room
        .strip_prefix("http")
        .ok_or_else(|| Invalid("a room is http://host:port/rooms/<name>".to_string()))?;
    let name: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
    Ok(format!("ws{}/ws?name={name}", rest.trim_end_matches('/')))
}

/// The AUTHORIZATION TOKEN (§10.2.2 of `draft-ietf-moq-transport-18`) that
/// carries a token of a room: Alias Type `USE_VALUE` (0x3), Token Type 0,
/// and the text, as the pages send it.
#[must_use]
pub fn authorization_token(token: &str) -> Vec<u8> {
    [&[0x03, 0x00], token.as_bytes()].concat()
}

/// The SHA-256 of a certificate from its hex.
///
/// # Errors
///
/// [`Invalid`] for anything but 64 hex digits.
pub fn certificate_hash(hex: &str) -> Result<[u8; 32], Invalid> {
    let invalid = || Invalid("the hash of a certificate is 64 hex digits".to_string());
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let mut hash = [0; 32];
    for (byte, digits) in hash.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let digits = std::str::from_utf8(digits).map_err(|_| invalid())?;
        *byte = u8::from_str_radix(digits, 16).map_err(|_| invalid())?;
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broadcast() -> Broadcast {
        Broadcast {
            relay_url: "https://127.0.0.1:4443/".to_string(),
            cert_sha256_hex: "ab".repeat(32),
            namespace: vec!["xmoq".to_string(), "demo".to_string()],
        }
    }

    #[test]
    fn the_messages_of_a_room_are_read() {
        let hash = "ab".repeat(32);
        let there = format!(
            r#"{{"relayUrl":"https://127.0.0.1:4443/","certSha256Hex":"{hash}","namespace":["xmoq","demo"],"since":1}}"#
        );
        // (the message; what it is read as)
        let cases = [
            // A history of the chat, from a room that still keeps one, is
            // passed over.
            (
                format!(
                    r#"{{"type":"hello","history":[{{"type":"chat","seq":1,"from":"a","text":"hi"}}],"connected":2,"broadcast":{there}}}"#
                ),
                Said::Hello {
                    broadcast: Some(broadcast()),
                    relay: None,
                    transcript: None,
                    meeting_transcript: None,
                },
            ),
            (
                r#"{"type":"hello","connected":1,"broadcast":null}"#.to_string(),
                Said::Hello {
                    broadcast: None,
                    relay: None,
                    transcript: None,
                    meeting_transcript: None,
                },
            ),
            // So is a line of its chat: the chat is on tracks.
            (
                r#"{"type":"chat","seq":7,"from":"publisher","text":"hello","at":1}"#.to_string(),
                Said::Other,
            ),
            (
                format!(r#"{{"type":"broadcast","broadcast":{there}}}"#),
                Said::Broadcast(Some(broadcast())),
            ),
            (
                r#"{"type":"broadcast","broadcast":null}"#.to_string(),
                Said::Broadcast(None),
            ),
            // A relay that is not reached by https.
            (
                r#"{"type":"broadcast","broadcast":{"relayUrl":"http://h/","certSha256Hex":"00","namespace":["a"]}}"#
                    .to_string(),
                Said::Broadcast(None),
            ),
            (
                r#"{"type":"broadcast","broadcast":{"relayUrl":"file:///etc/passwd","certSha256Hex":"00","namespace":["a"]}}"#
                    .to_string(),
                Said::Broadcast(None),
            ),
            // A broadcast without its certificate, and one whose namespace is no strings.
            (
                r#"{"type":"broadcast","broadcast":{"relayUrl":"https://h/","namespace":["a"]}}"#
                    .to_string(),
                Said::Broadcast(None),
            ),
            (
                r#"{"type":"broadcast","broadcast":{"relayUrl":"https://h/","certSha256Hex":"00","namespace":["a",1]}}"#
                    .to_string(),
                Said::Broadcast(None),
            ),
            (
                r#"{"type":"token","role":"sub","namespace":["a"],"token":"v1|a|sub|1|ff"}"#
                    .to_string(),
                Said::Token(Some("v1|a|sub|1|ff".to_string())),
            ),
            (
                r#"{"type":"token","role":"sub","namespace":["a"],"token":null}"#.to_string(),
                Said::Token(None),
            ),
            (r#"{"type":"connected","connected":3}"#.to_string(), Said::Other),
            (r#"{"type":7}"#.to_string(), Said::Other),
            ("[]".to_string(), Said::Other),
        ];
        for (message, said) in cases {
            assert_eq!(read(&message), Ok(said), "{message}");
        }
        assert!(read("{").is_err());
    }

    #[test]
    fn the_relay_of_the_rooms_is_read_from_the_hello() {
        let hash = "ab".repeat(32);
        let relay = format!(r#"{{"relayUrl":"https://127.0.0.1:4443/","certSha256Hex":"{hash}"}}"#);
        let there = format!(
            r#"{{"relayUrl":"https://127.0.0.1:4443/","certSha256Hex":"{hash}","namespace":["xmoq","demo"]}}"#
        );
        let read_relay = || {
            Some(Relay {
                relay_url: "https://127.0.0.1:4443/".to_string(),
                cert_sha256_hex: "ab".repeat(32),
            })
        };
        // (the hello; the broadcast and the relay it is read as)
        let cases = [
            (
                format!(r#"{{"type":"hello","connected":1,"broadcast":null,"relay":{relay}}}"#),
                None,
                read_relay(),
            ),
            (
                format!(r#"{{"type":"hello","connected":1,"broadcast":{there},"relay":{relay}}}"#),
                Some(broadcast()),
                read_relay(),
            ),
            // A room that names no relay.
            (
                r#"{"type":"hello","connected":1,"broadcast":null,"relay":null}"#.to_string(),
                None,
                None,
            ),
            // A relay that is not reached by https, or without its certificate.
            (
                r#"{"type":"hello","broadcast":null,"relay":{"relayUrl":"http://h/","certSha256Hex":"00"}}"#
                    .to_string(),
                None,
                None,
            ),
            (
                r#"{"type":"hello","broadcast":null,"relay":{"relayUrl":"https://h/"}}"#.to_string(),
                None,
                None,
            ),
        ];
        for (message, broadcast, relay) in cases {
            assert_eq!(
                read(&message),
                Ok(Said::Hello {
                    broadcast,
                    relay,
                    transcript: None,
                    meeting_transcript: None,
                }),
                "{message}"
            );
        }
    }

    #[test]
    fn the_prefixes_of_the_transcripts_are_read_from_the_hello() {
        // Each of its own key.
        assert_eq!(
            read(
                r#"{"type":"hello","broadcast":null,"transcript":["xmoq","demo"],"meetingTranscript":["xmoq","meet","r"]}"#
            ),
            Ok(Said::Hello {
                broadcast: None,
                relay: None,
                transcript: Some(vec!["xmoq".to_string(), "demo".to_string()]),
                meeting_transcript: Some(vec![
                    "xmoq".to_string(),
                    "meet".to_string(),
                    "r".to_string(),
                ]),
            })
        );
        // (the value of both keys of a hello without a broadcast; what each
        // is read as)
        let cases = [
            (
                r#"["xmoq","meet","r"]"#,
                Some(vec![
                    "xmoq".to_string(),
                    "meet".to_string(),
                    "r".to_string(),
                ]),
            ),
            (r#"["xmoq"]"#, Some(vec!["xmoq".to_string()])),
            ("null", None),
            // No fields, an empty field, a field that is no string, and no array.
            ("[]", None),
            (r#"["xmoq",""]"#, None),
            (r#"["xmoq",1]"#, None),
            (r#""xmoq/meet/r""#, None),
        ];
        for (transcript, read_as) in cases {
            let message = format!(
                r#"{{"type":"hello","broadcast":null,"transcript":{transcript},"meetingTranscript":{transcript}}}"#
            );
            assert_eq!(
                read(&message),
                Ok(Said::Hello {
                    broadcast: None,
                    relay: None,
                    transcript: read_as.clone(),
                    meeting_transcript: read_as,
                }),
                "{message}"
            );
        }
    }

    #[test]
    fn the_answers_to_a_seat_are_read() {
        let seat = |token: Option<&str>| {
            Said::Seat(Seat {
                participant: "a1".to_string(),
                subscribe: ["xmoq", "meet", "r", "*"].map(str::to_string).to_vec(),
                subscribe_token: token.map(str::to_string),
            })
        };
        // (the message, as `packages/room/src/worker.ts` sends it;
        // what it is read as)
        let cases = [
            (
                r#"{"type":"meeting","participant":"a1","publish":{"namespace":["xmoq","meet","r","a1"],"token":"v1|xmoq/meet/r/a1|pub|1|ff"},"subscribe":{"namespace":["xmoq","meet","r","*"],"token":"v1|xmoq/meet/r/*|sub|1|ff"}}"#,
                seat(Some("v1|xmoq/meet/r/*|sub|1|ff")),
            ),
            // A room without a key.
            (
                r#"{"type":"meeting","participant":"a1","publish":{"namespace":["xmoq","meet","r","a1"],"token":null},"subscribe":{"namespace":["xmoq","meet","r","*"],"token":null}}"#,
                seat(None),
            ),
            (
                r#"{"type":"meeting-full","most":49}"#,
                Said::MeetingFull(49),
            ),
            // Without the participant, without the namespace, and a full
            // meeting that does not say how many it takes.
            (
                r#"{"type":"meeting","subscribe":{"namespace":["xmoq","meet","r","*"],"token":null}}"#,
                Said::Other,
            ),
            (
                r#"{"type":"meeting","participant":"a1","subscribe":{"token":null}}"#,
                Said::Other,
            ),
            (r#"{"type":"meeting-full"}"#, Said::Other),
            (
                r#"{"type":"meeting-refused","why":"taken"}"#,
                Said::MeetingRefused("taken".to_string()),
            ),
            // A refusal that does not say why.
            (r#"{"type":"meeting-refused"}"#, Said::Other),
        ];
        for (message, said) in cases {
            assert_eq!(read(message), Ok(said), "{message}");
        }
    }

    #[test]
    fn a_line_is_made_safe_for_a_terminal() {
        // (the text; the most characters; what is written)
        let cases = [
            // An escape code is taken out, and what is left of it is text.
            ("a\u{1b}[2Jb\nc", 120, "a[2Jbc".to_string()),
            // So are a right-to-left override, a line separator, an isolate
            // and a byte order mark.
            (
                "a\u{202e}b\u{2028}c\u{2066}d\u{feff}",
                120,
                "abcd".to_string(),
            ),
            // A long line is cut at a character.
            (
                "\u{3042}\u{3042}\u{3042}",
                2,
                "\u{3042}\u{3042}".to_string(),
            ),
        ];
        for (text, most, written) in cases {
            assert_eq!(clean(text, most), written, "{text:?}");
        }
    }

    #[test]
    fn what_is_said_to_a_room() {
        assert_eq!(
            ask_for_a_token(TokenRole::Sub, &["xmoq".to_string(), "demo".to_string()]),
            r#"{"namespace":["xmoq","demo"],"role":"sub","type":"token-for"}"#
        );
        assert_eq!(
            ask_for_a_token(TokenRole::Pub, &["xmoq".to_string(), "chat".to_string()]),
            r#"{"namespace":["xmoq","chat"],"role":"pub","type":"token-for"}"#
        );
        assert_eq!(
            ask_for_a_seat("a1"),
            r#"{"participant":"a1","type":"join-meeting"}"#
        );
    }

    #[test]
    fn the_address_of_a_rooms_websocket() {
        // (the room; the name; the address)
        let cases = [
            (
                "http://127.0.0.1:8787/rooms/demo",
                "terminal",
                Ok("ws://127.0.0.1:8787/rooms/demo/ws?name=terminal"),
            ),
            (
                "https://rooms.example/rooms/a/",
                "t e&r=m/1",
                Ok("wss://rooms.example/rooms/a/ws?name=term1"),
            ),
            ("http://h/rooms/a", "", Ok("ws://h/rooms/a/ws?name=")),
            ("ws://h/rooms/a", "t", Err(())),
            ("", "t", Err(())),
        ];
        for (room, name, address) in cases {
            assert_eq!(
                websocket_address(room, name).map_err(|_| ()),
                address.map(str::to_string),
                "{room}"
            );
        }
    }

    #[test]
    fn a_token_and_a_hash_as_bytes() {
        assert_eq!(authorization_token("v1"), [0x03, 0x00, b'v', b'1']);
        let mut expected = [0u8; 32];
        expected[0] = 0x01;
        expected[31] = 0xfe;
        let hex = format!("01{}FE", "00".repeat(30));
        assert_eq!(certificate_hash(&hex), Ok(expected));
        // (what is no hash of 64 hex digits)
        let cases = [
            String::new(),
            "00".repeat(31),
            "00".repeat(33),
            format!("{}zz", "00".repeat(31)),
            format!("+1{}", "00".repeat(31)),
            "\u{3042}".repeat(64),
        ];
        for hex in cases {
            assert!(certificate_hash(&hex).is_err(), "{hex}");
        }
    }
}
