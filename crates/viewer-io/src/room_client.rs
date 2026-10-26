//! A room over its HTTP API (`@xmoq/room`): where its relay is, tokens, and
//! the seats of a meeting. What is asked and how each answer is read is
//! `xmoq_viewer::room`'s; the forms are `fixtures/room-api/README.md`.
//!
//! WHY reqwest: https is not written by hand, and reqwest with
//! `rustls-no-provider` uses rustls with the ring provider and
//! `rustls-platform-verifier`, which quinn already brings into the build;
//! without its default features it brings neither aws-lc-rs (and its C
//! build) nor HTTP/2 nor the system's proxy settings. WHY NOT hyper alone:
//! the connection, TLS, the Host header, the body and the timeouts would be
//! written here. WHY NOT ureq: it blocks, and every request would go
//! through `spawn_blocking` on tokio.
//!
//! reqwest is a dependency that may be replaced: this file alone touches it.

use std::sync::Once;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::CONTENT_TYPE;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use xmoq_again::AskAgain;
use xmoq_chat_tracks::names::Person;
use xmoq_viewer::room::{
    self, Answer, NoSeat, Request, Room, RoomAddress, RoomKind, TakenSeat, TokenRole,
};

/// How long a room that refuses the connection is asked again, from the
/// first try. **Provisional.** WHY asked again at all: rooms often start
/// after the readers that ask them (a demo started beside them, a Worker
/// restarted), and a reader that gives up at the first refusal has to be
/// started again by hand. WHY a limit: a wrong address would otherwise be
/// tried for ever without a word.
pub const ROOM_COMES_WITHIN: Duration = Duration::from_secs(30);

/// The tokens of a room for the chat on tracks ([`RoomClient::chat_tokens`]),
/// each as an AUTHORIZATION TOKEN; none where the room gave none.
#[derive(Debug, Default, Clone)]
pub struct ChatTokens {
    /// `pub` in `xmoq/chat/<kind>/<room>/<name>`.
    pub publish: Option<Vec<u8>>,
    /// `sub` in `xmoq/chat/<kind>/<room>`.
    pub subscribe: Option<Vec<u8>>,
}

/// Whether the connection was refused: nobody listens there (yet).
fn refused(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused)
    })
}

/// The most bytes of one answer of a room. Its largest, a seat with two
/// tokens, is under 1 KiB; the bound keeps a server that is no room from
/// filling the memory.
const MOST_AN_ANSWER: usize = 64 * 1024;

/// The shortest wait between two renewals of a seat, whatever `renewMs` the
/// room says. **Provisional.** WHY: a `renewMs` of 0 would renew it as fast
/// as the room answers.
const LEAST_BETWEEN_RENEWALS: Duration = Duration::from_millis(100);

/// Has rustls take its ring provider for the process.
///
/// WHY: reqwest's `rustls-no-provider` takes the process's default provider,
/// and there is none until one is installed. WHY ring: quinn is built with
/// it; aws-lc-rs would be one more, built from C.
fn install_the_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // An error is a provider installed already, which is used instead.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// A room asked over HTTP.
#[derive(Debug, Clone)]
pub struct RoomClient {
    http: reqwest::Client,
    room: RoomAddress,
    base: reqwest::Url,
}

impl RoomClient {
    /// A client of `room`, each request of which is given up after `wait`,
    /// its answer's body read included.
    ///
    /// # Errors
    ///
    /// A base of the address that is no URL, or a client that cannot be
    /// built (no certificates of the platform to verify with).
    pub fn new(room: RoomAddress, wait: Duration) -> Result<Self> {
        install_the_crypto_provider();
        let base = reqwest::Url::parse(&room.base)
            .with_context(|| format!("the address of a room: {}", room.base))?;
        let http = reqwest::Client::builder()
            .timeout(wait)
            // WHY no redirect: a room answers each request itself, and a
            // redirect would send a seat's lease to another server.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("a client of the room")?;
        Ok(Self { http, room, base })
    }

    /// The room the address names.
    #[must_use]
    pub fn address(&self) -> &RoomAddress {
        &self.room
    }

    async fn ask(&self, request: Request) -> Result<Answer> {
        let url = self
            .base
            .join(&request.path)
            .with_context(|| format!("a path of the room: {}", request.path))?;
        let method = match request.method {
            room::Method::Get => reqwest::Method::GET,
            room::Method::Post => reqwest::Method::POST,
            room::Method::Delete => reqwest::Method::DELETE,
        };
        let mut asking = self.http.request(method, url.clone());
        if let Some(body) = request.body {
            asking = asking.header(CONTENT_TYPE, "application/json").body(body);
        }
        let mut response = asking
            .send()
            .await
            .with_context(|| format!("asking the room {url}"))?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .with_context(|| format!("the answer of the room {url}"))?
        {
            if body.len() + chunk.len() > MOST_AN_ANSWER {
                bail!("the answer of the room {url} is over {MOST_AN_ANSWER} bytes");
            }
            body.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(body)
            .with_context(|| format!("the answer of the room {url} is no text"))?;
        Ok(room::read_answer(request.asked, status, &body)?)
    }

    /// The room, of the kind its address names.
    ///
    /// # Errors
    ///
    /// A room that does not answer within the wait, or answers what is not
    /// the room.
    pub async fn room(&self) -> Result<Room> {
        match self.ask(self.room.ask_for_the_room()).await? {
            Answer::Room(room) => Ok(room),
            other => Err(anyhow!("the room answered {other:?} for itself")),
        }
    }

    /// [`Self::room`], asked again by the rule of `xmoq-again` (a refusal
    /// that names no time) while the connection is refused, for `within`
    /// from the first try. Any other failure ends it at once.
    ///
    /// # Errors
    ///
    /// Those of [`Self::room`]; a refusal that lasts `within`.
    pub async fn room_when_there(&self, within: Duration) -> Result<Room> {
        let until = Instant::now() + within;
        let mut again = AskAgain::default();
        loop {
            match self.room().await {
                Err(error) if refused(&error) => {
                    let pause = Duration::from_millis(again.after_refusal(None));
                    if Instant::now() + pause > until {
                        return Err(error.context(format!("refused for {} s", within.as_secs())));
                    }
                    tokio::time::sleep(pause).await;
                }
                answered => return answered,
            }
        }
    }

    /// A token for `role` in `namespace`; none where the room makes none.
    ///
    /// # Errors
    ///
    /// As [`Self::room`].
    pub async fn token(&self, role: TokenRole, namespace: &[String]) -> Result<Option<String>> {
        match self.ask(self.room.ask_for_a_token(role, namespace)).await? {
            Answer::Token(token) => Ok(token),
            other => Err(anyhow!("the room answered {other:?} for a token")),
        }
    }

    /// The two tokens the chat on tracks needs of the room: `pub` in the
    /// person's namespace `xmoq/chat/<kind>/<room>/<name>`, for its
    /// `PUBLISH_NAMESPACE`, and `sub` in `xmoq/chat/<kind>/<room>`, for its
    /// SUBSCRIBE and FETCH of `lines` (and the reader's
    /// `SUBSCRIBE_NAMESPACE`).
    ///
    /// WHY tokens of their own on the chat's requests, and not the session's
    /// token as well: that one is `sub` in the broadcast's namespace or the
    /// meeting's, which allows none of them.
    ///
    /// # Errors
    ///
    /// As [`Self::room`].
    pub async fn chat_tokens(&self, person: &Person) -> Result<ChatTokens> {
        let room_fields: Vec<String> = xmoq_chat_tracks::names::PREFIX
            .iter()
            .map(|field| (*field).to_string())
            .chain([
                person.room.kind.field().to_string(),
                person.room.name.clone(),
            ])
            .collect();
        let mine: Vec<String> = room_fields
            .iter()
            .cloned()
            .chain([person.name.clone()])
            .collect();
        let publish = self.token(TokenRole::Pub, &mine).await?;
        let subscribe = self.token(TokenRole::Sub, &room_fields).await?;
        Ok(ChatTokens {
            publish: publish.map(|text| room::authorization_token(&text)),
            subscribe: subscribe.map(|text| room::authorization_token(&text)),
        })
    }

    /// A seat in the meeting for `participant`, held and renewed until it is
    /// left or dropped; or the room's answer that is no seat.
    ///
    /// # Errors
    ///
    /// A room of a broadcast, which has no seats, and as [`Self::room`].
    pub async fn seat(&self, participant: &str) -> Result<Result<HeldSeat, NoSeat>> {
        if self.room.kind != RoomKind::Meeting {
            bail!("the room of a broadcast has no seats: {}", self.room.name);
        }
        match self.ask(self.room.ask_for_a_seat(participant)).await? {
            Answer::Seat(seat) => Ok(Ok(HeldSeat::hold(self.clone(), seat))),
            Answer::NoSeat(no_seat) => Ok(Err(no_seat)),
            other => Err(anyhow!("the room answered {other:?} for a seat")),
        }
    }
}

/// Why a held seat was lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatLost {
    /// The room answered a renewal with `gone`: the seat ran out.
    Gone,
    /// A renewal failed: the room did not answer within the wait, or
    /// answered what is not a seat.
    Failed(String),
}

/// A seat in a meeting, renewed every `renewMs` on a task of its own.
///
/// The task ends when the seat is dropped, when it is left, and when a
/// renewal is lost ([`Self::lost`]); it is not asked again, as `keepSeat` of
/// `@xmoq/room-client` does not: a seat runs out some renewals after the
/// last one, so the caller that is told at once can take a new seat before
/// the old one is gone. A seat that is dropped without [`Self::leave`] is
/// freed by the room when it runs out, at its `expires_at_ms`.
#[derive(Debug)]
pub struct HeldSeat {
    client: RoomClient,
    seat: watch::Receiver<TakenSeat>,
    lost: watch::Receiver<Option<SeatLost>>,
    renewing: JoinHandle<()>,
}

impl HeldSeat {
    fn hold(client: RoomClient, seat: TakenSeat) -> Self {
        let (seat_sender, seat) = watch::channel(seat);
        let (lost_sender, lost) = watch::channel(None);
        let renewing = tokio::spawn(renew(client.clone(), seat_sender, lost_sender));
        Self {
            client,
            seat,
            lost,
            renewing,
        }
    }

    /// The seat as it was taken or last renewed: its tokens are the newest.
    /// A clone of the receiver tells of each renewal.
    #[must_use]
    pub fn seat(&self) -> watch::Receiver<TakenSeat> {
        self.seat.clone()
    }

    /// Waits until the seat is lost, and says why. It is not lost while it
    /// is renewed.
    pub async fn lost(&self) -> SeatLost {
        let mut lost = self.lost.clone();
        match lost.wait_for(Option::is_some).await {
            Ok(why) => why.clone().unwrap_or_else(|| {
                SeatLost::Failed("the renewal of the seat said nothing".to_string())
            }),
            // The task ended without a word: it panicked.
            Err(_) => SeatLost::Failed("the renewal of the seat stopped".to_string()),
        }
    }

    /// Whether the seat is lost already ([`Self::lost`]).
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.lost.borrow().is_some() || self.renewing.is_finished()
    }

    /// Stops renewing the seat and leaves it: the room frees it at once.
    ///
    /// # Errors
    ///
    /// A room that does not answer within the wait, or answers what is not
    /// that the seat was left. The seat is freed when it runs out all the
    /// same.
    pub async fn leave(self) -> Result<()> {
        self.renewing.abort();
        let (participant, lease) = {
            let seat = self.seat.borrow();
            (seat.participant.clone(), seat.lease.clone())
        };
        let request = self.client.room.leave_a_seat(&participant, &lease);
        match self.client.ask(request).await? {
            Answer::Left => Ok(()),
            other => Err(anyhow!("the room answered {other:?} for leaving a seat")),
        }
    }
}

impl Drop for HeldSeat {
    fn drop(&mut self) {
        self.renewing.abort();
    }
}

/// Renews the seat every `renewMs` until a renewal is lost, which it says.
async fn renew(
    client: RoomClient,
    seat: watch::Sender<TakenSeat>,
    lost: watch::Sender<Option<SeatLost>>,
) {
    loop {
        let (participant, lease, renew_ms) = {
            let seat = seat.borrow();
            (seat.participant.clone(), seat.lease.clone(), seat.renew_ms)
        };
        tokio::time::sleep(Duration::from_millis(renew_ms).max(LEAST_BETWEEN_RENEWALS)).await;
        let why = match client
            .ask(client.room.renew_a_seat(&participant, &lease))
            .await
        {
            Ok(Answer::Seat(renewed)) => {
                seat.send_replace(renewed);
                continue;
            }
            Ok(Answer::NoSeat(NoSeat::Gone)) => SeatLost::Gone,
            Ok(other) => SeatLost::Failed(format!("the room answered {other:?} for a renewal")),
            Err(error) => SeatLost::Failed(format!("{error:#}")),
        };
        lost.send_replace(Some(why));
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;

    /// The status and the body of a row of `fixtures/room-api/answers.tsv`.
    fn row(name: &str) -> (u16, String) {
        let path = format!(
            "{}/../../fixtures/room-api/answers.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let columns: Vec<&str> = table
            .lines()
            .map(|row| row.split('\t').collect::<Vec<_>>())
            .find(|columns| columns.first() == Some(&name))
            .unwrap_or_else(|| panic!("{path}: no row {name}"));
        let body = if columns[3] == "-" { "" } else { columns[3] };
        (columns[2].parse().expect("a status"), body.to_string())
    }

    /// A row whose seat is to be renewed every 100 ms.
    fn renewed_soon(name: &str) -> (u16, String) {
        let (status, body) = row(name);
        (
            status,
            body.replace(r#""renewMs":20000"#, r#""renewMs":100"#),
        )
    }

    /// A request the fake room was sent: its method, its path and its body.
    type Sent = (String, String, String);

    /// Reads one HTTP/1.1 request.
    async fn read_request(stream: &mut TcpStream) -> Sent {
        let mut bytes = Vec::new();
        let head_end = loop {
            let mut buffer = [0; 1024];
            let read = stream.read(&mut buffer).await.expect("a request");
            assert_ne!(read, 0, "a request ends before its head does");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(at) = bytes.windows(4).position(|four| four == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8(bytes[..head_end].to_vec()).expect("a head of text");
        let length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().expect("a length"))
            })
            .unwrap_or(0);
        while bytes.len() < head_end + length {
            let mut buffer = [0; 1024];
            let read = stream.read(&mut buffer).await.expect("a body");
            assert_ne!(read, 0, "a request ends before its body does");
            bytes.extend_from_slice(&buffer[..read]);
        }
        let mut words = head.split(' ');
        let method = words.next().expect("a method").to_string();
        let path = words.next().expect("a path").to_string();
        let body = String::from_utf8(bytes[head_end..head_end + length].to_vec()).expect("text");
        (method, path, body)
    }

    /// A room on a port of its own that answers each connection, in order,
    /// with the next of `answers`, and tells of each request it was sent.
    /// Without answers, it reads the request and never answers.
    async fn fake_room(answers: Vec<(u16, String)>) -> (SocketAddr, mpsc::UnboundedReceiver<Sent>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("its address");
        (address, serve(listener, answers))
    }

    /// [`fake_room`] on `listener`.
    fn serve(listener: TcpListener, answers: Vec<(u16, String)>) -> mpsc::UnboundedReceiver<Sent> {
        let (sent, told) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut answers = answers.into_iter();
            loop {
                let (mut stream, _) = listener.accept().await.expect("a connection");
                let request = read_request(&mut stream).await;
                let _ = sent.send(request);
                let Some((status, body)) = answers.next() else {
                    // Held open, never answered.
                    tokio::spawn(async move {
                        let mut buffer = [0; 1];
                        let _ = stream.read(&mut buffer).await;
                    });
                    continue;
                };
                let length = if status == 204 {
                    String::new()
                } else {
                    format!("content-length: {}\r\n", body.len())
                };
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{length}connection: close\r\n\r\n{body}"
                );
                stream.write_all(answer.as_bytes()).await.expect("written");
                let _ = stream.shutdown().await;
            }
        });
        told
    }

    /// A port that nobody listens on: bound, then let go.
    async fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        listener.local_addr().expect("its address").port()
    }

    fn client(address: SocketAddr, room: &str, wait: Duration) -> RoomClient {
        let room = room::room_address(&format!("http://{address}/{room}")).expect("a room");
        RoomClient::new(room, wait).expect("a client")
    }

    const WAIT: Duration = Duration::from_secs(5);
    const LEASE: &str = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";

    #[tokio::test]
    async fn a_room_is_asked_for() {
        // (the room; the row it answers with; the path asked; the kind and
        // the namespace read)
        let cases = [
            (
                "broadcasts/blue",
                "broadcast",
                "/broadcasts/blue",
                "broadcast xmoq/broadcast/blue",
            ),
            (
                "meetings/blue",
                "meeting",
                "/meetings/blue",
                "meeting xmoq/meet/blue",
            ),
        ];
        for (room, answer, path, read) in cases {
            let (address, mut told) = fake_room(vec![row(answer)]).await;
            let answered = client(address, room, WAIT).room().await.expect("the room");
            let read_as = match answered {
                Room::Broadcast(room) => format!("broadcast {}", room.namespace.join("/")),
                Room::Meeting(room) => format!("meeting {}", room.namespace.join("/")),
            };
            assert_eq!(read_as, read, "{room}");
            assert_eq!(
                told.recv().await,
                Some(("GET".to_string(), path.to_string(), String::new())),
                "{room}"
            );
        }
    }

    #[tokio::test]
    async fn a_token_is_asked_for() {
        // (the row it answers with; the token read)
        let cases = [
            (
                "token",
                Some(
                    "v1|xmoq/chat/meeting/blue|sub|1793000000|abababababababababababababababababababababababababababababababab",
                ),
            ),
            ("token-none", None),
        ];
        for (answer, token) in cases {
            let (address, mut told) = fake_room(vec![row(answer)]).await;
            let namespace = ["xmoq", "chat", "meeting", "blue"].map(str::to_string);
            let answered = client(address, "meetings/blue", WAIT)
                .token(TokenRole::Sub, &namespace)
                .await
                .expect("a token");
            assert_eq!(answered.as_deref(), token, "{answer}");
            assert_eq!(
                told.recv().await,
                Some((
                    "POST".to_string(),
                    "/meetings/blue/tokens".to_string(),
                    r#"{"namespace":["xmoq","chat","meeting","blue"],"role":"sub"}"#.to_string()
                )),
                "{answer}"
            );
        }
    }

    #[tokio::test]
    async fn a_seat_is_taken_and_left() {
        let (address, mut told) = fake_room(vec![row("seat"), row("left")]).await;
        let held = client(address, "meetings/blue", WAIT)
            .seat("a1")
            .await
            .expect("an answer")
            .expect("a seat");
        assert_eq!(held.seat().borrow().lease, LEASE);
        held.leave().await.expect("left");
        assert_eq!(
            told.recv().await,
            Some((
                "POST".to_string(),
                "/meetings/blue/seats".to_string(),
                r#"{"participant":"a1"}"#.to_string()
            ))
        );
        assert_eq!(
            told.recv().await,
            Some((
                "DELETE".to_string(),
                "/meetings/blue/seats/a1".to_string(),
                format!(r#"{{"lease":"{LEASE}"}}"#)
            ))
        );
    }

    #[tokio::test]
    async fn no_seat_is_given() {
        // (the row it answers with; what is read)
        let cases = [
            ("full", NoSeat::Full { most: 49 }),
            ("refused-taken", NoSeat::Refused(room::Refusal::Taken)),
        ];
        for (answer, no_seat) in cases {
            let (address, _told) = fake_room(vec![row(answer)]).await;
            let answered = client(address, "meetings/blue", WAIT)
                .seat("a1")
                .await
                .expect("an answer");
            assert_eq!(answered.err(), Some(no_seat), "{answer}");
        }
    }

    #[tokio::test]
    async fn a_seat_is_renewed_until_it_is_gone() {
        let (address, mut told) = fake_room(vec![
            renewed_soon("seat"),
            renewed_soon("renewed"),
            row("renew-of-a-seat-that-is-gone"),
        ])
        .await;
        let held = client(address, "meetings/blue", WAIT)
            .seat("a1")
            .await
            .expect("an answer")
            .expect("a seat");
        let mut seat = held.seat();
        seat.changed().await.expect("renewed");
        assert_eq!(seat.borrow().expires_at_ms, 1_793_000_080_000);
        assert_eq!(held.lost().await, SeatLost::Gone);
        let renewal = (
            "POST".to_string(),
            "/meetings/blue/seats/a1/renew".to_string(),
            format!(r#"{{"lease":"{LEASE}"}}"#),
        );
        told.recv().await;
        assert_eq!(told.recv().await, Some(renewal.clone()));
        assert_eq!(told.recv().await, Some(renewal));
    }

    #[tokio::test]
    async fn a_room_that_does_not_answer_is_given_up() {
        let (address, _told) = fake_room(Vec::new()).await;
        let started = std::time::Instant::now();
        let answered = client(address, "broadcasts/blue", Duration::from_millis(200))
            .room()
            .await;
        assert!(answered.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_renewal_that_is_not_answered_loses_the_seat() {
        let (address, _told) = fake_room(vec![renewed_soon("seat")]).await;
        let held = client(address, "meetings/blue", Duration::from_millis(200))
            .seat("a1")
            .await
            .expect("an answer")
            .expect("a seat");
        assert!(matches!(held.lost().await, SeatLost::Failed(_)));
    }

    #[tokio::test]
    async fn a_room_of_a_broadcast_has_no_seats() {
        let (address, _told) = fake_room(Vec::new()).await;
        let answered = client(address, "broadcasts/blue", WAIT).seat("a1").await;
        assert!(answered.is_err());
    }

    #[tokio::test]
    async fn a_room_that_begins_listening_later_is_asked() {
        let port = free_port().await;
        let later = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let listener = TcpListener::bind(("127.0.0.1", port))
                .await
                .expect("the port");
            serve(listener, vec![row("broadcast")])
        });
        let answered = client(
            SocketAddr::from(([127, 0, 0, 1], port)),
            "broadcasts/blue",
            WAIT,
        )
        .room_when_there(ROOM_COMES_WITHIN)
        .await
        .expect("the room");
        assert!(matches!(answered, Room::Broadcast(_)), "{answered:?}");
        later.await.expect("the room listened");
    }

    #[tokio::test]
    async fn a_refusal_that_lasts_ends_it() {
        let port = free_port().await;
        let error = client(
            SocketAddr::from(([127, 0, 0, 1], port)),
            "broadcasts/blue",
            WAIT,
        )
        .room_when_there(Duration::ZERO)
        .await
        .expect_err("refused");
        assert!(refused(&error), "{error:#}");
        assert_eq!(error.to_string(), "refused for 0 s");
    }

    #[tokio::test]
    async fn a_connection_closed_without_a_word_is_not_asked_again() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("its address");
        let closing = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("a connection");
            drop(stream);
        });
        let error = client(address, "broadcasts/blue", WAIT)
            .room_when_there(ROOM_COMES_WITHIN)
            .await
            .expect_err("closed");
        assert!(!refused(&error), "{error:#}");
        closing.await.expect("closed");
    }

    #[tokio::test]
    async fn the_tokens_of_the_chat_are_asked_for() {
        let (address, mut told) = fake_room(vec![row("token"), row("token-none")]).await;
        let person = Person {
            room: xmoq_chat_tracks::names::Room {
                kind: xmoq_chat_tracks::names::Kind::Meeting,
                name: "blue".to_string(),
            },
            name: "agent".to_string(),
        };
        let tokens = client(address, "meetings/blue", WAIT)
            .chat_tokens(&person)
            .await
            .expect("the tokens");
        assert_eq!(
            tokens.publish,
            Some(room::authorization_token(
                "v1|xmoq/chat/meeting/blue|sub|1793000000|abababababababababababababababababababababababababababababababab"
            ))
        );
        assert_eq!(tokens.subscribe, None);
        // (the body of each request, in order)
        let bodies = [
            r#"{"namespace":["xmoq","chat","meeting","blue","agent"],"role":"pub"}"#,
            r#"{"namespace":["xmoq","chat","meeting","blue"],"role":"sub"}"#,
        ];
        for body in bodies {
            assert_eq!(
                told.recv().await,
                Some((
                    "POST".to_string(),
                    "/meetings/blue/tokens".to_string(),
                    body.to_string()
                ))
            );
        }
    }
}
