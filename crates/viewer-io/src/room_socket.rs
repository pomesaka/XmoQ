//! A room over its WebSocket (`xmoq_viewer::room` has what is said on it).

use std::collections::VecDeque;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use xmoq_chat_tracks::names::Person;
use xmoq_viewer::room::{self, Broadcast, Relay, Said};

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

/// A room joined over its WebSocket, and the broadcast it has.
pub struct RoomSocket {
    socket: Socket,
    /// The broadcasts the room said while something else was waited for.
    kept: VecDeque<Option<Broadcast>>,
    broadcast: Option<Broadcast>,
    /// The relay of the rooms, as the hello said it.
    relay: Option<Relay>,
    /// The prefix of the room's transcript, as the hello said it.
    transcript: Option<Vec<String>>,
}

impl RoomSocket {
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
        };
        // The room says hello first: the broadcast it has, its relay, and
        // its transcript's prefix.
        let Said::Hello {
            broadcast,
            relay,
            transcript,
        } = joined.message().await?
        else {
            bail!("the room did not begin with hello");
        };
        joined.broadcast = broadcast;
        joined.relay = relay;
        joined.transcript = transcript;
        Ok(joined)
    }

    /// The prefix a transcriber listens under for the room, where the
    /// room's hello said it.
    pub fn transcript(&self) -> Option<Vec<String>> {
        self.transcript.clone()
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
            Said::Hello { .. } | Said::Token(_) | Said::Other => false,
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
        self.socket
            .send(Message::text(room::ask_for_a_token(role, namespace)))
            .await?;
        loop {
            match self.message().await? {
                Said::Token(token) => return Ok(token),
                said => {
                    if self.heard(said) {
                        // A room that talks and gives no token: the oldest goes.
                        if self.kept.len() >= KEPT {
                            self.kept.pop_front();
                        }
                        self.kept.push_back(self.current());
                    }
                }
            }
        }
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
