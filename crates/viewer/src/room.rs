//! What is asked of a room and what it answers: the requests and answers of
//! its HTTP API (`@xmoq/room`), whose forms are
//! `fixtures/room-api/README.md`.

use serde_json::{Map, Value, json};

/// The most characters of a line of the chat that are shown.
pub const LINE_LIMIT: usize = 120;

/// Where a room's relay is: its address, and the SHA-256 of its certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    /// The address of the relay.
    pub relay_url: String,
    /// The SHA-256 of the relay's certificate, as hex.
    pub cert_sha256_hex: String,
}

/// What is no answer of a room, and no address of one.
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

/// What a token of a room allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRole {
    /// Watching, and the room's chat lines.
    Sub,
    /// The viewer's own chat namespace.
    Pub,
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

/// Which of the two kinds of room an address names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomKind {
    /// `/broadcasts/<name>`: the room of one broadcast.
    Broadcast,
    /// `/meetings/<name>`: the room of a meeting, with its seats.
    Meeting,
}

/// A room of the HTTP API, as its address names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAddress {
    /// What comes before the kind, ending with `/`: `http://host:port/`.
    pub base: String,
    /// The kind of room.
    pub kind: RoomKind,
    /// The room's name, as it is in the address.
    pub name: String,
}

/// Reads which room an address names: `<base>/broadcasts/<name>` or
/// `<base>/meetings/<name>`, over `http` or `https`. A slash after the name,
/// a query and a fragment are left out.
///
/// # Errors
///
/// [`Invalid`] for any other address.
pub fn room_address(address: &str) -> Result<RoomAddress, Invalid> {
    let invalid = || {
        Invalid(format!(
            "a room is http(s)://host/broadcasts/<name> or http(s)://host/meetings/<name>, not {address}"
        ))
    };
    let (scheme, rest) = ["https://", "http://"]
        .into_iter()
        .find_map(|scheme| address.strip_prefix(scheme).map(|rest| (scheme, rest)))
        .ok_or_else(invalid)?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = rest.split_once('/').ok_or_else(invalid)?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let (before, name) = path.rsplit_once('/').ok_or_else(invalid)?;
    let (prefix, kind) = before.rsplit_once('/').unwrap_or(("", before));
    let kind = match kind {
        "broadcasts" => RoomKind::Broadcast,
        "meetings" => RoomKind::Meeting,
        _ => return Err(invalid()),
    };
    // WHY NOT `.` and `..`: the address of a request is the base joined with
    // the name, where they would name another path than the room's.
    if host.is_empty() || name.is_empty() || name == "." || name == ".." {
        return Err(invalid());
    }
    let prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    Ok(RoomAddress {
        base: format!("{scheme}{host}/{prefix}"),
        kind,
        name: name.to_string(),
    })
}

/// What a request asks a room for, which says how its answer is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// `GET /broadcasts/<name>` or `GET /meetings/<name>`.
    Room(RoomKind),
    /// `POST .../tokens`.
    Token,
    /// `POST /meetings/<name>/seats`.
    Seat,
    /// `POST /meetings/<name>/seats/<participant>/renew`.
    Renewal,
    /// `DELETE /meetings/<name>/seats/<participant>`.
    Leaving,
}

/// The method of a request to a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `DELETE`.
    Delete,
}

/// A request to a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// What it asks for.
    pub asked: Asked,
    /// Its method.
    pub method: Method,
    /// Its path, relative to [`RoomAddress::base`].
    pub path: String,
    /// Its body, JSON, where it has one.
    pub body: Option<String>,
}

/// A segment of a path, with every byte but the unreserved ones of RFC 3986
/// (§2.3) percent-encoded: a participant's id is the asker's, and goes into
/// the path.
fn path_segment(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

impl RoomAddress {
    fn path(&self) -> String {
        let kind = match self.kind {
            RoomKind::Broadcast => "broadcasts",
            RoomKind::Meeting => "meetings",
        };
        format!("{kind}/{}", self.name)
    }

    fn seat_path(&self, participant: &str) -> String {
        format!("meetings/{}/seats/{}", self.name, path_segment(participant))
    }

    /// `GET` of the room.
    #[must_use]
    pub fn ask_for_the_room(&self) -> Request {
        Request {
            asked: Asked::Room(self.kind),
            method: Method::Get,
            path: self.path(),
            body: None,
        }
    }

    /// A token for `role` in `namespace`, one of the room's.
    #[must_use]
    pub fn ask_for_a_token(&self, role: TokenRole, namespace: &[String]) -> Request {
        let role = match role {
            TokenRole::Sub => "sub",
            TokenRole::Pub => "pub",
        };
        Request {
            asked: Asked::Token,
            method: Method::Post,
            path: format!("{}/tokens", self.path()),
            body: Some(json!({"role": role, "namespace": namespace}).to_string()),
        }
    }

    /// A seat in the meeting of the room of this name for `participant`. A
    /// room of a broadcast has no seats: this asks the meeting's all the same.
    #[must_use]
    pub fn ask_for_a_seat(&self, participant: &str) -> Request {
        Request {
            asked: Asked::Seat,
            method: Method::Post,
            path: format!("meetings/{}/seats", self.name),
            body: Some(json!({"participant": participant}).to_string()),
        }
    }

    /// The seat of `participant` for longer, named by its lease.
    #[must_use]
    pub fn renew_a_seat(&self, participant: &str, lease: &str) -> Request {
        Request {
            asked: Asked::Renewal,
            method: Method::Post,
            path: format!("{}/renew", self.seat_path(participant)),
            body: Some(json!({"lease": lease}).to_string()),
        }
    }

    /// Leaving the seat of `participant`, named by its lease.
    #[must_use]
    pub fn leave_a_seat(&self, participant: &str, lease: &str) -> Request {
        Request {
            asked: Asked::Leaving,
            method: Method::Delete,
            path: self.seat_path(participant),
            body: Some(json!({"lease": lease}).to_string()),
        }
    }
}

/// The room of a broadcast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastRoom {
    /// Its relay, where the room was told it.
    pub relay: Option<Relay>,
    /// The namespace of its one broadcast, its fields.
    pub namespace: Vec<String>,
    /// The prefix a transcriber listens under for it, its fields.
    pub transcript: Option<Vec<String>>,
}

/// The room of a meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRoom {
    /// Its relay, where the room was told it.
    pub relay: Option<Relay>,
    /// The prefix of its participants' namespaces, its fields.
    pub namespace: Vec<String>,
    /// The prefix a transcriber listens under for it, its fields.
    pub transcript: Option<Vec<String>>,
    /// How many seats it has.
    pub most: u64,
}

/// A room, of either kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Room {
    /// The room of a broadcast.
    Broadcast(BroadcastRoom),
    /// The room of a meeting.
    Meeting(MeetingRoom),
}

/// A seat in a meeting, taken or renewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenSeat {
    /// The participant's id.
    pub participant: String,
    /// The participant's own namespace, its fields.
    pub publish: Vec<String>,
    /// `pub` in it, or none where the room has no key.
    pub publish_token: Option<String>,
    /// The namespaces of the meeting, its fields: `xmoq/meet/<name>/*`.
    pub subscribe: Vec<String>,
    /// `sub` in them, or none where the room has no key.
    pub subscribe_token: Option<String>,
    /// What the seat is renewed and left with.
    pub lease: String,
    /// When the seat runs out unless it is renewed, in milliseconds since
    /// the Unix epoch, by the room's clock.
    pub expires_at_ms: u64,
    /// How often the room asks for it to be renewed, in milliseconds.
    pub renew_ms: u64,
}

/// Why a room gives no seat, other than a full meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Another holds the participant's seat.
    Taken,
    /// The id is not one a participant may have.
    NotAParticipant,
    /// The name is not one a meeting's room may have.
    NotAMeetingRoom,
}

/// The answers to a seat, or to its renewal, that are no seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoSeat {
    /// The meeting has all the seats it takes.
    Full {
        /// How many it takes.
        most: u64,
    },
    /// The room will not give it.
    Refused(Refusal),
    /// The seat to be renewed ran out, or was never there.
    Gone,
}

/// What a room answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// To [`Asked::Room`].
    Room(Room),
    /// To [`Asked::Token`]: the token, or none where the room makes none.
    Token(Option<String>),
    /// To [`Asked::Seat`] and [`Asked::Renewal`].
    Seat(TakenSeat),
    /// To [`Asked::Seat`] and [`Asked::Renewal`].
    NoSeat(NoSeat),
    /// To [`Asked::Leaving`]: the seat is free.
    Left,
}

/// A relay of the HTTP API: `{url, certSha256Hex}`.
fn api_relay_of(value: &Value) -> Option<Relay> {
    let url = value.get("url")?.as_str()?;
    let hex = value.get("certSha256Hex")?.as_str()?;
    // WHY https alone: a relay is reached by WebTransport, and an address
    // of any other kind from a room is not one to hand a token to. WHY the hash read here: a
    // relay that cannot be pinned is no relay to connect to.
    if !url.starts_with("https://") || certificate_hash(hex).is_err() {
        return None;
    }
    Some(Relay {
        relay_url: url.to_string(),
        cert_sha256_hex: hex.to_string(),
    })
}

/// `null` as none, else what `read` reads; `Err` where it reads nothing.
fn nullable<T>(value: &Value, read: impl FnOnce(&Value) -> Option<T>) -> Result<Option<T>, ()> {
    if value.is_null() {
        Ok(None)
    } else {
        read(value).map(Some).ok_or(())
    }
}

fn text_of(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

fn broadcast_room_of(body: &Map<String, Value>) -> Option<BroadcastRoom> {
    Some(BroadcastRoom {
        relay: nullable(body.get("relay")?, api_relay_of).ok()?,
        namespace: fields_of(body.get("namespace")?)?,
        transcript: nullable(body.get("transcript")?, fields_of).ok()?,
    })
}

fn meeting_room_of(body: &Map<String, Value>) -> Option<MeetingRoom> {
    let BroadcastRoom {
        relay,
        namespace,
        transcript,
    } = broadcast_room_of(body)?;
    Some(MeetingRoom {
        relay,
        namespace,
        transcript,
        most: body.get("most")?.as_u64()?,
    })
}

/// A participant's id that can be a segment of a path: not empty, `.` or
/// `..`.
///
/// WHY refused and not percent-encoded: the id goes into the path of the
/// seat's renewal and of its leaving ([`path_segment`]), where `.` and `..`
/// are dot-segments that joining the path takes out (RFC 3986 §5.2.4), and
/// `%2E` is one as well to a URL parser of the URL Standard (its "single-dot
/// URL path segment"); `..` would have the DELETE of a seat go to the
/// meeting's own path. No page makes such an id.
fn participant_of(value: &Value) -> Option<String> {
    let id = value.as_str()?;
    (!matches!(id, "" | "." | "..")).then(|| id.to_string())
}

fn taken_seat_of(body: &Map<String, Value>) -> Option<TakenSeat> {
    let publish = body.get("publish")?;
    let subscribe = body.get("subscribe")?;
    Some(TakenSeat {
        participant: participant_of(body.get("participant")?)?,
        publish: fields_of(publish.get("namespace")?)?,
        publish_token: nullable(publish.get("token")?, text_of).ok()?,
        subscribe: fields_of(subscribe.get("namespace")?)?,
        subscribe_token: nullable(subscribe.get("token")?, text_of).ok()?,
        lease: body.get("lease")?.as_str()?.to_string(),
        expires_at_ms: body.get("expiresAt")?.as_u64()?,
        renew_ms: body.get("renewMs")?.as_u64()?,
    })
}

fn full_of(body: &Map<String, Value>) -> Option<NoSeat> {
    Some(NoSeat::Full {
        most: body.get("full")?.get("most")?.as_u64()?,
    })
}

fn refused_of(body: &Map<String, Value>) -> Option<NoSeat> {
    let why = match body.get("refused")?.get("why")?.as_str()? {
        "taken" => Refusal::Taken,
        "not-a-participant" => Refusal::NotAParticipant,
        "not-a-meeting-room" => Refusal::NotAMeetingRoom,
        _ => return None,
    };
    Some(NoSeat::Refused(why))
}

fn gone_of(body: &Map<String, Value>) -> Option<NoSeat> {
    (body.get("gone")? == &Value::Bool(true)).then_some(NoSeat::Gone)
}

/// Reads a room's answer to what was `asked`, by its status and its body
/// (empty for none). The statuses each request has are those of
/// `fixtures/room-api/README.md`.
///
/// # Errors
///
/// [`Invalid`] for a status the request does not have, and for a body that
/// is not the one of that status: a field missing or of another type, a
/// relay that is not reached by `https://` or whose hash is not 64 hex
/// digits, or a namespace with no fields or an empty one.
pub fn read_answer(asked: Asked, status: u16, body: &str) -> Result<Answer, Invalid> {
    let malformed = || {
        Invalid(format!(
            "the room answered {asked:?} with {status} and a body that is not the one of it: {}",
            clean(body, LINE_LIMIT)
        ))
    };
    if asked == Asked::Leaving {
        return if status == 204 {
            Ok(Answer::Left)
        } else {
            Err(malformed())
        };
    }
    let value: Value = serde_json::from_str(body).map_err(|_| malformed())?;
    let body = value.as_object().ok_or_else(malformed)?;
    let answer = match (asked, status) {
        (Asked::Room(RoomKind::Broadcast), 200) => {
            broadcast_room_of(body).map(|room| Answer::Room(Room::Broadcast(room)))
        }
        (Asked::Room(RoomKind::Meeting), 200) => {
            meeting_room_of(body).map(|room| Answer::Room(Room::Meeting(room)))
        }
        (Asked::Token, 200) => body
            .get("token")
            .and_then(|token| nullable(token, text_of).ok())
            .map(Answer::Token),
        (Asked::Seat | Asked::Renewal, 200) => taken_seat_of(body).map(Answer::Seat),
        (Asked::Seat, 409) => full_of(body)
            .or_else(|| refused_of(body))
            .map(Answer::NoSeat),
        (Asked::Seat, 400 | 404) => refused_of(body).map(Answer::NoSeat),
        (Asked::Renewal, 404) => gone_of(body).map(Answer::NoSeat),
        _ => None,
    };
    answer.ok_or_else(malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn which_room_an_address_names() {
        // (the address; the base, the kind and the name, or none)
        let cases = [
            (
                "http://127.0.0.1:8787/broadcasts/blue",
                Some(("http://127.0.0.1:8787/", RoomKind::Broadcast, "blue")),
            ),
            (
                "https://rooms.example/meetings/blue",
                Some(("https://rooms.example/", RoomKind::Meeting, "blue")),
            ),
            (
                "http://h/meetings/blue/",
                Some(("http://h/", RoomKind::Meeting, "blue")),
            ),
            (
                "http://h/broadcasts/blue?name=t#x",
                Some(("http://h/", RoomKind::Broadcast, "blue")),
            ),
            (
                "https://h/rooms/api/meetings/blue",
                Some(("https://h/rooms/api/", RoomKind::Meeting, "blue")),
            ),
            // A name that is empty, or that names another path.
            ("http://h/broadcasts/", None),
            ("http://h/broadcasts//", None),
            ("http://h/meetings/..", None),
            // Another path, another scheme, no host, no path.
            ("http://127.0.0.1:8787/rooms/demo", None),
            ("http://h/blue", None),
            ("http://h/broadcasts/blue/seats", None),
            ("ws://h/broadcasts/blue", None),
            ("http:///broadcasts/blue", None),
            ("http://h", None),
            ("", None),
        ];
        for (address, read_as) in cases {
            let expected = read_as.map(|(base, kind, name)| RoomAddress {
                base: base.to_string(),
                kind,
                name: name.to_string(),
            });
            assert_eq!(room_address(address).ok(), expected, "{address}");
        }
        let error = room_address("http://h/rooms/demo").expect_err("another path");
        assert!(
            error.0.contains("/broadcasts/<name>") && error.0.contains("/meetings/<name>"),
            "{error}"
        );
    }

    #[test]
    fn the_requests_to_a_room() {
        let broadcast = room_address("http://h/broadcasts/blue").expect("a broadcast's room");
        let meeting = room_address("http://h/meetings/blue").expect("a meeting's room");
        // (the request; its method, its path and its body)
        let cases = [
            (
                broadcast.ask_for_the_room(),
                Method::Get,
                "broadcasts/blue",
                None,
            ),
            (
                meeting.ask_for_the_room(),
                Method::Get,
                "meetings/blue",
                None,
            ),
            (
                broadcast.ask_for_a_token(TokenRole::Pub, &["xmoq".to_string()]),
                Method::Post,
                "broadcasts/blue/tokens",
                Some(r#"{"namespace":["xmoq"],"role":"pub"}"#),
            ),
            (
                meeting.ask_for_a_token(TokenRole::Sub, &["xmoq".to_string(), "chat".to_string()]),
                Method::Post,
                "meetings/blue/tokens",
                Some(r#"{"namespace":["xmoq","chat"],"role":"sub"}"#),
            ),
            (
                meeting.ask_for_a_seat("a/b"),
                Method::Post,
                "meetings/blue/seats",
                Some(r#"{"participant":"a/b"}"#),
            ),
            (
                meeting.renew_a_seat("a1", "0f"),
                Method::Post,
                "meetings/blue/seats/a1/renew",
                Some(r#"{"lease":"0f"}"#),
            ),
            // An id that is not one is sent as it is, a segment of the path.
            (
                meeting.leave_a_seat("a/b c", "0f"),
                Method::Delete,
                "meetings/blue/seats/a%2Fb%20c",
                Some(r#"{"lease":"0f"}"#),
            ),
        ];
        for (request, method, path, body) in cases {
            assert_eq!(
                (
                    request.method,
                    request.path.as_str(),
                    request.body.as_deref()
                ),
                (method, path, body),
                "{request:?}"
            );
        }
    }

    /// An answer in the form of the fifth column of `answers.tsv`.
    fn form_of(answer: &Result<Answer, Invalid>) -> String {
        let or_dash = |text: Option<&str>| text.unwrap_or("-").to_string();
        let relay_of = |relay: &Option<Relay>| {
            format!(
                "relay={} cert={}",
                or_dash(relay.as_ref().map(|relay| relay.relay_url.as_str())),
                or_dash(relay.as_ref().map(|relay| relay.cert_sha256_hex.as_str())),
            )
        };
        let prefix_of = |prefix: &Option<Vec<String>>| {
            prefix
                .as_ref()
                .map_or_else(|| "-".to_string(), |fields| fields.join("/"))
        };
        match answer {
            Err(_) => "malformed".to_string(),
            Ok(Answer::Room(Room::Broadcast(room))) => format!(
                "broadcast {} namespace={} transcript={}",
                relay_of(&room.relay),
                room.namespace.join("/"),
                prefix_of(&room.transcript)
            ),
            Ok(Answer::Room(Room::Meeting(room))) => format!(
                "meeting {} namespace={} transcript={} most={}",
                relay_of(&room.relay),
                room.namespace.join("/"),
                prefix_of(&room.transcript),
                room.most
            ),
            Ok(Answer::Seat(seat)) => format!(
                "seat participant={} pub={} sub={} lease={} expires={} renew={}",
                seat.participant,
                or_dash(seat.publish_token.as_deref()),
                or_dash(seat.subscribe_token.as_deref()),
                seat.lease,
                seat.expires_at_ms,
                seat.renew_ms
            ),
            Ok(Answer::NoSeat(NoSeat::Full { most })) => format!("full {most}"),
            Ok(Answer::NoSeat(NoSeat::Refused(why))) => format!(
                "refused {}",
                match why {
                    Refusal::Taken => "taken",
                    Refusal::NotAParticipant => "not-a-participant",
                    Refusal::NotAMeetingRoom => "not-a-meeting-room",
                }
            ),
            Ok(Answer::NoSeat(NoSeat::Gone)) => "gone".to_string(),
            Ok(Answer::Left) => "left".to_string(),
            Ok(Answer::Token(token)) => format!("token {}", or_dash(token.as_deref())),
        }
    }

    /// The request of the second column of `answers.tsv`, as this module
    /// makes it from what the column names.
    fn request_of(column: &str) -> Request {
        let mut parts = column.splitn(3, ' ');
        let method = parts.next().expect("a method");
        let path = parts.next().expect("a path");
        let body: Option<Value> = parts
            .next()
            .map(|body| serde_json::from_str(body).expect("a body of JSON"));
        let text = |key: &str| {
            body.as_ref()
                .and_then(|body| body.get(key))
                .and_then(Value::as_str)
                .expect(key)
                .to_string()
        };
        let segments: Vec<&str> = path
            .strip_prefix('/')
            .expect("a path from the root")
            .split('/')
            .collect();
        let room = |kind: &str, name: &str| {
            room_address(&format!("http://h/{kind}/{name}")).expect("a room")
        };
        match (method, segments.as_slice()) {
            ("GET", [kind, name]) => room(kind, name).ask_for_the_room(),
            ("POST", [kind, name, "tokens"]) => {
                let role = match text("role").as_str() {
                    "pub" => TokenRole::Pub,
                    "sub" => TokenRole::Sub,
                    other => panic!("a role: {other}"),
                };
                let namespace = body
                    .as_ref()
                    .and_then(|body| body.get("namespace"))
                    .and_then(fields_of)
                    .expect("a namespace");
                room(kind, name).ask_for_a_token(role, &namespace)
            }
            ("POST", ["meetings", name, "seats"]) => {
                room("meetings", name).ask_for_a_seat(&text("participant"))
            }
            ("POST", ["meetings", name, "seats", participant, "renew"]) => {
                room("meetings", name).renew_a_seat(participant, &text("lease"))
            }
            ("DELETE", ["meetings", name, "seats", participant]) => {
                room("meetings", name).leave_a_seat(participant, &text("lease"))
            }
            _ => panic!("a request of the README: {column}"),
        }
    }

    #[test]
    fn the_answers_of_the_shared_table_are_read() {
        let path = format!(
            "{}/../../fixtures/room-api/answers.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert!(
            table.ends_with('\n'),
            "{path}: its last row ends with a line feed"
        );
        assert!(table.lines().next().is_some(), "{path}: no rows");
        for row in table.lines() {
            let columns: Vec<&str> = row.split('\t').collect();
            let [name, asked, status, body, read_as] = columns.as_slice() else {
                panic!("a row of five columns: {row:?}");
            };
            let request = request_of(asked);
            // What this module sends is what the column names.
            let mut parts = asked.splitn(3, ' ');
            let method = match parts.next() {
                Some("GET") => Method::Get,
                Some("POST") => Method::Post,
                Some("DELETE") => Method::Delete,
                other => panic!("{name}: a method: {other:?}"),
            };
            assert_eq!(request.method, method, "{name}");
            assert_eq!(
                Some(format!("/{}", request.path).as_str()),
                parts.next(),
                "{name}"
            );
            assert_eq!(
                request
                    .body
                    .as_deref()
                    .map(|body| serde_json::from_str::<Value>(body).expect("JSON")),
                parts
                    .next()
                    .map(|body| serde_json::from_str::<Value>(body).expect("JSON")),
                "{name}"
            );
            let status: u16 = status.parse().expect("a status");
            let body = if *body == "-" { "" } else { body };
            let answer = read_answer(request.asked, status, body);
            assert_eq!(form_of(&answer), *read_as, "{name}");
        }
    }

    #[test]
    fn an_answer_with_a_status_its_request_does_not_have() {
        let seat = r#"{"participant":"a1","publish":{"namespace":["a"],"token":null},"subscribe":{"namespace":["a"],"token":null},"lease":"0f","expiresAt":1,"renewMs":1}"#;
        // (what was asked; the status; the body)
        let cases = [
            (
                Asked::Room(RoomKind::Broadcast),
                404,
                r#"{"error":"no such path"}"#,
            ),
            (Asked::Token, 400, r#"{"token":null}"#),
            (Asked::Seat, 201, seat),
            (Asked::Seat, 400, r#"{"full":{"most":49}}"#),
            (Asked::Renewal, 409, r#"{"refused":{"why":"taken"}}"#),
            (Asked::Renewal, 404, r#"{"gone":false}"#),
            (Asked::Leaving, 200, ""),
            (Asked::Leaving, 404, r#"{"error":"no such path"}"#),
            // A body that is no object.
            (Asked::Token, 200, "[]"),
            (Asked::Token, 200, ""),
        ];
        for (asked, status, body) in cases {
            assert!(
                read_answer(asked, status, body).is_err(),
                "{asked:?} {status} {body}"
            );
        }
    }

    #[test]
    fn a_relay_is_read_from_a_room_only_where_it_can_be_pinned() {
        let hash = "ab".repeat(32);
        // (the relay; whether the room is read)
        let cases = [
            (
                format!(r#"{{"url":"https://h/","certSha256Hex":"{hash}"}}"#),
                true,
            ),
            (
                format!(r#"{{"url":"http://h/","certSha256Hex":"{hash}"}}"#),
                false,
            ),
            (
                r#"{"url":"https://h/","certSha256Hex":"00"}"#.to_string(),
                false,
            ),
            (r#"{"url":"https://h/"}"#.to_string(), false),
            ("7".to_string(), false),
        ];
        for (relay, read) in cases {
            let body = format!(r#"{{"relay":{relay},"namespace":["a"],"transcript":null}}"#);
            assert_eq!(
                read_answer(Asked::Room(RoomKind::Broadcast), 200, &body).is_ok(),
                read,
                "{relay}"
            );
        }
    }

    #[test]
    fn a_seat_whose_participant_is_no_segment_of_a_path_is_not_read() {
        // (the participant the room answers with; whether the seat is read)
        let cases = [
            ("a1", true),
            ("a.b", true),
            ("", false),
            (".", false),
            ("..", false),
        ];
        for (participant, read) in cases {
            let body = format!(
                r#"{{"participant":"{participant}","publish":{{"namespace":["xmoq","meet","blue","a1"],"token":null}},"subscribe":{{"namespace":["xmoq","meet","blue","*"],"token":null}},"lease":"0f","expiresAt":1,"renewMs":1}}"#
            );
            assert_eq!(
                read_answer(Asked::Seat, 200, &body).is_ok(),
                read,
                "{participant}"
            );
        }
    }
}
