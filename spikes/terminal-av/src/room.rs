//! A room of spikes/durable-objects-room, over its WebSocket: where the
//! broadcast is, a token for the relay, and the chat.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone)]
pub struct Broadcast {
    pub relay_url: String,
    pub cert_sha256_hex: String,
    pub namespace: Vec<String>,
}

pub enum Said {
    /// A line of the chat, as the pages show it.
    Chat(String),
    /// The publisher left, or another came.
    Broadcast(Option<Broadcast>),
}

pub struct Room {
    socket: Socket,
    /// What the room said while something else was waited for.
    kept: VecDeque<Said>,
    broadcast: Option<Broadcast>,
}

fn broadcast_of(value: &Value) -> Option<Broadcast> {
    Some(Broadcast {
        relay_url: value.get("relayUrl")?.as_str()?.to_string(),
        cert_sha256_hex: value.get("certSha256Hex")?.as_str()?.to_string(),
        namespace: value.get("namespace")?.as_array()?.iter().filter_map(|field| field.as_str().map(str::to_string)).collect(),
    })
}

/// A line of the chat without what a terminal would act on: the text is
/// anybody's.
fn chat_line(value: &Value) -> String {
    let text = |key| value.get(key).and_then(Value::as_str).unwrap_or("");
    let line = format!("{} {}: {}", value.get("seq").and_then(Value::as_u64).unwrap_or(0), text("from"), text("text"));
    line.chars().filter(|char| !char.is_control()).take(120).collect()
}

impl Room {
    /// Joins the room at `url` (`http://host:port/rooms/<name>`) as `name`.
    pub async fn join(url: &str, name: &str) -> Result<Self> {
        let Some(rest) = url.strip_prefix("http") else { bail!("--room is http://host:port/rooms/<name>") };
        // WHY only letters and digits of the name: it goes into the address as it is.
        let name: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
        let address = format!("ws{}/ws?name={name}", rest.trim_end_matches('/'));
        let (socket, _) = tokio_tungstenite::connect_async(&address).await.with_context(|| format!("the room at {address}"))?;
        let mut room = Self { socket, kept: VecDeque::new(), broadcast: None };
        // The room says hello first: the chat so far and the broadcast it has.
        let hello = room.message().await?;
        if hello.get("type").and_then(Value::as_str) != Some("hello") {
            bail!("the room did not begin with hello: {hello}");
        }
        for chat in hello.get("history").and_then(Value::as_array).into_iter().flatten() {
            room.kept.push_back(Said::Chat(chat_line(chat)));
        }
        room.broadcast = hello.get("broadcast").and_then(broadcast_of);
        Ok(room)
    }

    async fn message(&mut self) -> Result<Value> {
        loop {
            let message = self.socket.next().await.ok_or_else(|| anyhow!("the room closed the WebSocket"))??;
            if let Message::Text(text) = message {
                return Ok(serde_json::from_str(text.as_str())?);
            }
        }
    }

    /// What a message of the room says to a viewer; None for the rest.
    fn said(&mut self, message: &Value) -> Option<Said> {
        match message.get("type").and_then(Value::as_str) {
            Some("chat") => Some(Said::Chat(chat_line(message))),
            Some("broadcast") => {
                self.broadcast = message.get("broadcast").and_then(broadcast_of);
                Some(Said::Broadcast(self.broadcast.clone()))
            }
            _ => None,
        }
    }

    /// The broadcast of the room, waited for where it has none yet.
    pub async fn broadcast(&mut self, wait: Duration) -> Result<Broadcast> {
        let waiting = async {
            while self.broadcast.is_none() {
                let message = self.message().await?;
                if let Some(said) = self.said(&message) {
                    self.kept.push_back(said);
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(wait, waiting).await.map_err(|_| anyhow!("the room has no broadcast"))??;
        self.broadcast.clone().ok_or_else(|| anyhow!("the room has no broadcast"))
    }

    /// A token for watching in the namespace, or None where the room gives none.
    pub async fn token(&mut self, namespace: &[String]) -> Result<Option<String>> {
        let ask = json!({"type": "token-for", "role": "sub", "namespace": namespace});
        self.socket.send(Message::text(ask.to_string())).await?;
        loop {
            let message = self.message().await?;
            if message.get("type").and_then(Value::as_str) == Some("token") {
                return Ok(message.get("token").and_then(Value::as_str).map(str::to_string));
            }
            if let Some(said) = self.said(&message) {
                self.kept.push_back(said);
            }
        }
    }

    pub async fn say(&mut self, text: &str) -> Result<()> {
        self.socket.send(Message::text(json!({"type": "chat", "text": text}).to_string())).await?;
        Ok(())
    }

    /// The next thing the room says to a viewer.
    pub async fn next(&mut self) -> Result<Said> {
        loop {
            if let Some(said) = self.kept.pop_front() {
                return Ok(said);
            }
            let message = self.message().await?;
            if let Some(said) = self.said(&message) {
                return Ok(said);
            }
        }
    }
}
