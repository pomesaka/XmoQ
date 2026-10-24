//! A room over its WebSocket (`xmoq_watch::room` has what is said on it).

use std::collections::VecDeque;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use xmoq_watch::room::{self, Broadcast, Said};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The most a room's messages may wait while something else is waited for.
const KEPT: usize = 256;

/// The most bytes of one message of a room. The hello of a room that still
/// keeps a full history of its chat (50 lines of 500 characters) is under
/// 128 KiB.
const MOST_A_MESSAGE: usize = 256 * 1024;

pub struct RoomSocket {
    socket: Socket,
    /// The broadcasts the room said while something else was waited for.
    kept: VecDeque<Option<Broadcast>>,
    broadcast: Option<Broadcast>,
}

impl RoomSocket {
    /// Joins the room at `address` (`http://host:port/rooms/<name>`) as
    /// `name`.
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
        };
        // The room says hello first: the broadcast it has.
        let Said::Hello { broadcast } = joined.message().await? else {
            bail!("the room did not begin with hello");
        };
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

    /// The next broadcast the room says it has: the publisher left
    /// (`None`), or another came.
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
