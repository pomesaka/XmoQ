//! A room over its WebSocket (`xmoq_viewer::room` has what is said on it).

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;
use xmoq_again::AskAgain;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use xmoq_chat_tracks::names::Person;
use xmoq_viewer::room::{self, Broadcast, Relay, Said, Seat};

/// The tokens of a room for the chat on tracks ([`RoomSocket::chat_tokens`]),
/// each as an AUTHORIZATION TOKEN; none where the room gave none.
#[derive(Debug, Default, Clone)]
pub struct ChatTokens {
    /// `pub` in `xmoq/chat/<room>/<name>`.
    pub publish: Option<Vec<u8>>,
    /// `sub` in `xmoq/chat/<room>`.
    pub subscribe: Option<Vec<u8>>,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The most a room's messages may wait while something else is waited for.
const KEPT: usize = 256;

/// The most bytes of one message of a room. The hello of a room that still
/// keeps a full history of its chat (50 lines of 500 characters) is under
/// 128 KiB.
const MOST_A_MESSAGE: usize = 256 * 1024;

/// The room's meeting has no seat left: it takes this many.
#[derive(Debug)]
pub struct MeetingFull(pub u64);

impl std::fmt::Display for MeetingFull {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the meeting is full: it takes {}", self.0)
    }
}

impl std::error::Error for MeetingFull {}

/// A room joined over its WebSocket, and the broadcast it has.
pub struct RoomSocket {
    socket: Socket,
    /// The broadcasts the room said while something else was waited for.
    kept: VecDeque<Option<Broadcast>>,
    broadcast: Option<Broadcast>,
    /// The relay of the rooms, as the hello said it.
    relay: Option<Relay>,
    /// The prefix of the transcript of the room's broadcast, as the hello
    /// said it.
    transcript: Option<Vec<String>>,
    /// The same of the room's meeting.
    meeting_transcript: Option<Vec<String>>,
}

/// How long a room that refuses the connection is asked again, from the
/// first try. **Provisional.** WHY asked again at all: rooms often start
/// after the readers that join them (a demo started beside them, a Worker
/// restarted), and a reader that gives up at the first refusal has to be
/// started again by hand. WHY a limit: a wrong address would otherwise be
/// tried for ever without a word.
pub const ROOM_COMES_WITHIN: Duration = Duration::from_secs(30);

/// Whether the connection was refused: nobody listens there (yet).
fn refused(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused)
            || matches!(
                cause.downcast_ref::<tokio_tungstenite::tungstenite::Error>(),
                Some(tokio_tungstenite::tungstenite::Error::Io(io))
                    if io.kind() == std::io::ErrorKind::ConnectionRefused
            )
    })
}

impl RoomSocket {
    /// [`Self::join`], asked again by the rule of `xmoq-again` (a refusal
    /// that names no time) while the connection is refused, for `within`
    /// from the first try. Any other failure ends it at once.
    ///
    /// # Errors
    ///
    /// Those of [`Self::join`]; a refusal that lasts `within`.
    pub async fn join_when_there(address: &str, name: &str, within: Duration) -> Result<Self> {
        let until = Instant::now() + within;
        let mut again = AskAgain::default();
        loop {
            match Self::join(address, name).await {
                Err(error) if refused(&error) => {
                    let pause = Duration::from_millis(again.after_refusal(None));
                    if Instant::now() + pause > until {
                        return Err(error.context(format!("refused for {} s", within.as_secs())));
                    }
                    tokio::time::sleep(pause).await;
                }
                joined => return joined,
            }
        }
    }

    /// Joins the room at `address` (`http://host:port/rooms/<name>`) as
    /// `name`.
    ///
    /// # Errors
    ///
    /// An address that is no room's, a WebSocket that does not open, and a
    /// room that does not begin with hello.
    pub async fn join(address: &str, name: &str) -> Result<Self> {
        let address = room::websocket_address(address, name)?;
        // WHY a limit of its own: the library's is 64 MiB a message, and
        // what a room says is where a broadcast is.
        let limits = WebSocketConfig::default()
            .max_message_size(Some(MOST_A_MESSAGE))
            .max_frame_size(Some(MOST_A_MESSAGE));
        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(&address, Some(limits), false)
                .await
                .with_context(|| format!("the room at {address}"))?;
        let mut joined = Self {
            socket,
            kept: VecDeque::new(),
            broadcast: None,
            relay: None,
            transcript: None,
            meeting_transcript: None,
        };
        // The room says hello first: the broadcast it has, its relay, and
        // the prefixes of its transcripts.
        let Said::Hello {
            broadcast,
            relay,
            transcript,
            meeting_transcript,
        } = joined.message().await?
        else {
            bail!("the room did not begin with hello");
        };
        joined.broadcast = broadcast;
        joined.relay = relay;
        joined.transcript = transcript;
        joined.meeting_transcript = meeting_transcript;
        Ok(joined)
    }

    /// The prefix a transcriber listens under for the room's broadcast,
    /// where the room's hello said it.
    pub fn transcript(&self) -> Option<Vec<String>> {
        self.transcript.clone()
    }

    /// The prefix a transcriber listens under for the room's meeting, where
    /// the room's hello said it.
    pub fn meeting_transcript(&self) -> Option<Vec<String>> {
        self.meeting_transcript.clone()
    }

    /// The relay of the rooms, where the room's hello said it: there with a
    /// broadcast and without one.
    pub fn relay(&self) -> Option<Relay> {
        self.relay.clone()
    }

    async fn message(&mut self) -> Result<Said> {
        loop {
            let message = self
                .socket
                .next()
                .await
                .ok_or_else(|| anyhow!("the room closed the WebSocket"))??;
            if let Message::Text(text) = message {
                return Ok(room::read(text.as_str())?);
            }
        }
    }

    /// Whether a message of the room says which broadcast it has now,
    /// which [`Self::current`] gives from then on.
    fn heard(&mut self, said: Said) -> bool {
        match said {
            Said::Broadcast(broadcast) => {
                self.broadcast = broadcast;
                true
            }
            Said::Hello { .. }
            | Said::Token(_)
            | Said::Seat(_)
            | Said::MeetingFull(_)
            | Said::Other => false,
        }
    }

    /// Says `message`, and waits for what `answer` takes as its answer. The
    /// broadcasts said meanwhile are kept for [`Self::next`].
    async fn asked<T>(
        &mut self,
        message: String,
        answer: impl Fn(&Said) -> Option<T>,
    ) -> Result<T> {
        self.socket.send(Message::text(message)).await?;
        loop {
            let said = self.message().await?;
            match answer(&said) {
                Some(answer) => return Ok(answer),
                None => {
                    if self.heard(said) {
                        // A room that talks and does not answer: the oldest goes.
                        if self.kept.len() >= KEPT {
                            self.kept.pop_front();
                        }
                        self.kept.push_back(self.current());
                    }
                }
            }
        }
    }

    /// The broadcast the room has now, as it last said.
    pub fn current(&self) -> Option<Broadcast> {
        self.broadcast.clone()
    }

    /// A token for `role` in the namespace, or `None` where the room gives
    /// none.
    ///
    /// # Errors
    ///
    /// A WebSocket that closes or breaks, and a message that is no JSON.
    pub async fn token(
        &mut self,
        role: room::TokenRole,
        namespace: &[String],
    ) -> Result<Option<String>> {
        self.asked(room::ask_for_a_token(role, namespace), |said| match said {
            Said::Token(token) => Some(token.clone()),
            _ => None,
        })
        .await
    }

    /// A seat in the room's meeting as `participant`: asked again, the same
    /// on the same WebSocket, it has a new token. A room that does not take
    /// the id does not answer, and this waits for ever.
    ///
    /// # Errors
    ///
    /// As [`Self::token`]; a meeting that is full ([`MeetingFull`]); a seat
    /// for another participant.
    pub async fn seat(&mut self, participant: &str) -> Result<Seat> {
        let seat = self
            .asked(room::ask_for_a_seat(participant), |said| match said {
                Said::Seat(seat) => Some(Ok(seat.clone())),
                Said::MeetingFull(most) => Some(Err(*most)),
                _ => None,
            })
            .await?
            .map_err(MeetingFull)?;
        if seat.participant != participant {
            bail!(
                "the room gave a seat for {}, not for {participant}",
                seat.participant
            );
        }
        Ok(seat)
    }

    /// The two tokens the chat on tracks needs of the room: `pub` in the
    /// person's namespace `xmoq/chat/<room>/<name>`, for its
    /// `PUBLISH_NAMESPACE`, and `sub` in `xmoq/chat/<room>`, for its SUBSCRIBE
    /// and FETCH of `lines` (and the reader's `SUBSCRIBE_NAMESPACE`).
    ///
    /// WHY tokens of their own on the chat's requests, and not the session's
    /// token as well: that one is `sub` in the broadcast's namespace, which
    /// allows none of them, and the SETUP carries it already.
    ///
    /// # Errors
    ///
    /// As [`Self::token`].
    pub async fn chat_tokens(&mut self, person: &Person) -> Result<ChatTokens> {
        let prefix = xmoq_chat_tracks::names::PREFIX.map(str::to_string);
        let room_fields: Vec<String> = prefix.into_iter().chain([person.room.clone()]).collect();
        let mine: Vec<String> = room_fields
            .iter()
            .cloned()
            .chain([person.name.clone()])
            .collect();
        let publish = self.token(room::TokenRole::Pub, &mine).await?;
        let subscribe = self.token(room::TokenRole::Sub, &room_fields).await?;
        Ok(ChatTokens {
            publish: publish.map(|text| room::authorization_token(&text)),
            subscribe: subscribe.map(|text| room::authorization_token(&text)),
        })
    }

    /// The next broadcast the room says it has: the publisher left
    /// (`None`), or another came.
    ///
    /// # Errors
    ///
    /// As [`Self::token`].
    pub async fn next(&mut self) -> Result<Option<Broadcast>> {
        loop {
            if let Some(heard) = self.kept.pop_front() {
                return Ok(heard);
            }
            let said = self.message().await?;
            if self.heard(said) {
                return Ok(self.current());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A port that nobody listens on: bound, then let go.
    async fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    }

    /// A room that says hello, from `delay` on, on `port`.
    fn room_later(port: u16, delay: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(Message::text(
                    r#"{"type":"hello","connected":1,"broadcast":null}"#,
                ))
                .await
                .unwrap();
            // Held open until the reader has read the hello.
            let _ = socket.next().await;
        })
    }

    #[tokio::test]
    async fn a_room_that_begins_listening_later_is_joined() {
        let port = free_port().await;
        let room = room_later(port, Duration::from_millis(1500));
        let joined = RoomSocket::join_when_there(
            &format!("http://127.0.0.1:{port}/rooms/demo"),
            "agent",
            ROOM_COMES_WITHIN,
        )
        .await
        .unwrap();
        assert_eq!(joined.current(), None);
        drop(joined);
        room.await.unwrap();
    }

    #[tokio::test]
    async fn a_refusal_that_lasts_ends_it() {
        let port = free_port().await;
        let error = RoomSocket::join_when_there(
            &format!("http://127.0.0.1:{port}/rooms/demo"),
            "agent",
            Duration::ZERO,
        )
        .await
        .err()
        .unwrap();
        assert!(refused(&error));
        assert_eq!(error.to_string(), "refused for 0 s");
    }

    /// A room that says hello, reads one message, says `answers`, and keeps
    /// what it read.
    fn room_answering(
        listener: TcpListener,
        answers: Vec<String>,
    ) -> tokio::task::JoinHandle<String> {
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(Message::text(
                    r#"{"type":"hello","connected":1,"broadcast":null}"#,
                ))
                .await
                .unwrap();
            let asked = socket.next().await.unwrap().unwrap().into_text().unwrap();
            for answer in answers {
                socket.send(Message::text(answer)).await.unwrap();
            }
            // Held open until the reader is done.
            let _ = socket.next().await;
            asked.to_string()
        })
    }

    #[tokio::test]
    async fn a_seat_is_asked_for() {
        let broadcast = r#"{"type":"broadcast","broadcast":{"relayUrl":"https://h/","certSha256Hex":"00","namespace":["xmoq","demo"]}}"#;
        let seat = r#"{"type":"meeting","participant":"a1","publish":{"namespace":["xmoq","meet","r","a1"],"token":null},"subscribe":{"namespace":["xmoq","meet","r","*"],"token":"v1|t"}}"#;
        let another = seat.replace(r#""participant":"a1""#, r#""participant":"b2""#);
        // (what the room says after join-meeting; the seat's token, or the
        // error; whether a broadcast is kept for `next`; whether the error
        // is `MeetingFull`, which ends an agent where the others join anew)
        let cases = [
            (vec![seat.to_string()], Ok(Some("v1|t")), false, false),
            // What the room says meanwhile is not the answer, and a broadcast
            // among it is kept.
            (
                vec![
                    r#"{"type":"connected","connected":2}"#.to_string(),
                    broadcast.to_string(),
                    seat.to_string(),
                ],
                Ok(Some("v1|t")),
                true,
                false,
            ),
            (
                vec![r#"{"type":"meeting-full","most":49}"#.to_string()],
                Err("the meeting is full: it takes 49"),
                false,
                true,
            ),
            (
                vec![another],
                Err("the room gave a seat for b2, not for a1"),
                false,
                false,
            ),
        ];
        for (answers, expected, kept, full) in cases {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let room = room_answering(listener, answers);
            let mut joined = RoomSocket::join(&format!("http://127.0.0.1:{port}/rooms/r"), "agent")
                .await
                .unwrap();
            let got = joined.seat("a1").await;
            assert_eq!(
                got.as_ref()
                    .map(|seat| seat.subscribe_token.as_deref())
                    .map_err(ToString::to_string),
                expected.map_err(str::to_string),
                "{expected:?}"
            );
            assert_eq!(joined.kept.is_empty(), !kept, "{expected:?}");
            assert_eq!(
                got.as_ref()
                    .is_err_and(|error| error.downcast_ref::<MeetingFull>().is_some()),
                full,
                "{expected:?}"
            );
            drop(joined);
            assert_eq!(
                room.await.unwrap(),
                r#"{"participant":"a1","type":"join-meeting"}"#
            );
        }
    }

    #[tokio::test]
    async fn a_connection_closed_without_a_word_is_not_asked_again() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let closing = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });
        let error = RoomSocket::join_when_there(
            &format!("http://127.0.0.1:{port}/rooms/demo"),
            "agent",
            ROOM_COMES_WITHIN,
        )
        .await
        .err()
        .unwrap();
        assert!(!refused(&error));
        closing.await.unwrap();
    }
}
