//! A room over its WebSocket (`xmoq_watch::room` has what is said on it).

use std::collections::VecDeque;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use xmoq_watch::room::{self, Broadcast, Said};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The most a room's messages may wait while something else is waited for.
const KEPT: usize = 256;

/// What a room says to a viewer while it watches.
pub enum Heard {
    /// A line of the chat, ready to be shown.
    Chat(String),
    /// The publisher left, or another came.
    Broadcast(Option<Broadcast>),
}

pub struct RoomSocket {
    socket: Socket,
    /// What the room said while something else was waited for.
    kept: VecDeque<Heard>,
    broadcast: Option<Broadcast>,
}

impl RoomSocket {
    /// Joins the room at `address` (`http://host:port/rooms/<name>`) as
    /// `name`.
    pub async fn join(address: &str, name: &str) -> Result<Self> {
        let address = room::websocket_address(address, name)?;
        let (socket, _) = tokio_tungstenite::connect_async(&address)
            .await
            .with_context(|| format!("the room at {address}"))?;
        let mut joined = Self {
            socket,
            kept: VecDeque::new(),
            broadcast: None,
        };
        // The room says hello first: the chat so far and the broadcast it has.
        let Said::Hello { history, broadcast } = joined.message().await? else {
            bail!("the room did not begin with hello");
        };
        joined.kept.extend(history.into_iter().map(Heard::Chat));
        joined.broadcast = broadcast;
        Ok(joined)
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

    /// What a message of the room says to a viewer; `None` for the rest.
    fn heard(&mut self, said: Said) -> Option<Heard> {
        match said {
            Said::Chat(line) => Some(Heard::Chat(line)),
            Said::Broadcast(broadcast) => {
                self.broadcast.clone_from(&broadcast);
                Some(Heard::Broadcast(broadcast))
            }
            Said::Hello { .. } | Said::Token(_) | Said::Other => None,
        }
    }

    /// The broadcast the room has now, as it last said.
    pub fn current(&self) -> Option<Broadcast> {
        self.broadcast.clone()
    }

    /// A token for watching in the namespace, or `None` where the room
    /// gives none.
    pub async fn token(&mut self, namespace: &[String]) -> Result<Option<String>> {
        self.socket
            .send(Message::text(room::ask_for_a_token(namespace)))
            .await?;
        loop {
            match self.message().await? {
                Said::Token(token) => return Ok(token),
                said => {
                    if let Some(heard) = self.heard(said) {
                        // A room that talks and gives no token: the oldest goes.
                        if self.kept.len() >= KEPT {
                            self.kept.pop_front();
                        }
                        self.kept.push_back(heard);
                    }
                }
            }
        }
    }

    pub async fn say(&mut self, text: &str) -> Result<()> {
        self.socket.send(Message::text(room::say(text))).await?;
        Ok(())
    }

    /// The next thing the room says to a viewer.
    pub async fn next(&mut self) -> Result<Heard> {
        loop {
            if let Some(heard) = self.kept.pop_front() {
                return Ok(heard);
            }
            let said = self.message().await?;
            if let Some(heard) = self.heard(said) {
                return Ok(heard);
            }
        }
    }
}
