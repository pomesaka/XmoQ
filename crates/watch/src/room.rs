//! What a room says, and what is said to it: the messages of the room's
//! WebSocket (`spikes/durable-objects-room`), as text.

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

/// What a room said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// The first message: the lines of the chat so far, and the broadcast
    /// the room has.
    Hello {
        /// The chat so far, each line as [`Said::Chat`] has it.
        history: Vec<String>,
        /// The broadcast, where a publisher is in the room.
        broadcast: Option<Broadcast>,
    },
    /// A line of the chat, ready to be shown.
    Chat(String),
    /// A publisher came, or the one that was there left.
    Broadcast(Option<Broadcast>),
    /// The answer to [`ask_for_a_token`]: the token as text, or none where
    /// the room has no key.
    Token(Option<String>),
    /// Something the viewer has no use for.
    Other,
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

/// A text with nothing a terminal would act on, and no longer than `most`
/// characters.
///
/// WHY: a line of the chat is anybody's text, and it is written to a
/// terminal, where an escape code moves the cursor, changes the title or
/// worse.
#[must_use]
pub fn clean(text: &str, most: usize) -> String {
    text.chars()
        .filter(|char| !char.is_control())
        .take(most)
        .collect()
}

fn broadcast_of(value: &Value) -> Option<Broadcast> {
    Some(Broadcast {
        relay_url: value.get("relayUrl")?.as_str()?.to_string(),
        cert_sha256_hex: value.get("certSha256Hex")?.as_str()?.to_string(),
        namespace: value
            .get("namespace")?
            .as_array()?
            .iter()
            .map(|field| field.as_str().map(str::to_string))
            .collect::<Option<_>>()?,
    })
}

/// A line of the chat as the pages show it: its number, who said it, and
/// what.
fn chat_line(value: &Value) -> String {
    let text = |key| value.get(key).and_then(Value::as_str).unwrap_or("");
    let line = format!(
        "{} {}: {}",
        value.get("seq").and_then(Value::as_u64).unwrap_or(0),
        text("from"),
        text("text")
    );
    clean(&line, LINE_LIMIT)
}

/// Reads one message of a room.
///
/// A broadcast that lacks a field, or has one of another type, is read as
/// no broadcast: there is nowhere to connect to.
///
/// # Errors
///
/// [`Invalid`] for text that is no JSON.
pub fn read(text: &str) -> Result<Said, Invalid> {
    let message: Value =
        serde_json::from_str(text).map_err(|error| Invalid(format!("no JSON: {error}")))?;
    Ok(match message.get("type").and_then(Value::as_str) {
        Some("hello") => Said::Hello {
            history: message
                .get("history")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(chat_line)
                .collect(),
            broadcast: message.get("broadcast").and_then(broadcast_of),
        },
        Some("chat") => Said::Chat(chat_line(&message)),
        Some("broadcast") => Said::Broadcast(message.get("broadcast").and_then(broadcast_of)),
        Some("token") => Said::Token(
            message
                .get("token")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        _ => Said::Other,
    })
}

/// The message that asks a room for a token to watch in a namespace.
#[must_use]
pub fn ask_for_a_token(namespace: &[String]) -> String {
    json!({"type": "token-for", "role": "sub", "namespace": namespace}).to_string()
}

/// The message that says a line in the chat.
#[must_use]
pub fn say(text: &str) -> String {
    json!({"type": "chat", "text": text}).to_string()
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
        let chat = |line: &str| Said::Chat(line.to_string());
        // (the message; what it is read as)
        let cases = [
            (
                format!(
                    r#"{{"type":"hello","history":[{{"type":"chat","seq":1,"from":"a","text":"hi"}}],"connected":2,"broadcast":{there}}}"#
                ),
                Said::Hello {
                    history: vec!["1 a: hi".to_string()],
                    broadcast: Some(broadcast()),
                },
            ),
            (
                r#"{"type":"hello","history":[],"connected":1,"broadcast":null}"#.to_string(),
                Said::Hello {
                    history: vec![],
                    broadcast: None,
                },
            ),
            (
                r#"{"type":"chat","seq":7,"from":"publisher","text":"hello","at":1}"#.to_string(),
                chat("7 publisher: hello"),
            ),
            // An escape code in a line is taken out, and what is left of it is text.
            (
                r#"{"type":"chat","seq":8,"from":"x","text":"a\u001b[2Jb\nc"}"#.to_string(),
                chat("8 x: a[2Jbc"),
            ),
            // Fields that are missing or of another type.
            (r#"{"type":"chat"}"#.to_string(), chat("0 : ")),
            (
                r#"{"type":"chat","seq":"1","from":2,"text":null}"#.to_string(),
                chat("0 : "),
            ),
            (
                format!(r#"{{"type":"broadcast","broadcast":{there}}}"#),
                Said::Broadcast(Some(broadcast())),
            ),
            (
                r#"{"type":"broadcast","broadcast":null}"#.to_string(),
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
    fn a_long_line_is_cut_at_a_character() {
        let long = "\u{3042}".repeat(200);
        let message = format!(r#"{{"type":"chat","seq":1,"from":"a","text":"{long}"}}"#);
        let Ok(Said::Chat(line)) = read(&message) else {
            panic!("a chat");
        };
        assert_eq!(line.chars().count(), 120);
        assert_eq!(line, format!("1 a: {}", "\u{3042}".repeat(115)));
    }

    #[test]
    fn what_is_said_to_a_room() {
        assert_eq!(
            ask_for_a_token(&["xmoq".to_string(), "demo".to_string()]),
            r#"{"namespace":["xmoq","demo"],"role":"sub","type":"token-for"}"#
        );
        assert_eq!(say("a \"b\"\n"), r#"{"text":"a \"b\"\n","type":"chat"}"#);
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
