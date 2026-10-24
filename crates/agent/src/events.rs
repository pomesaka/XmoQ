//! What the reader writes: one event a line, as JSON (JSON Lines), to
//! stdout and to `<DIR>/events.jsonl`.
//!
//! Every line has `type` and `at`, the reader's clock in milliseconds since
//! the Unix epoch when it wrote the line. The keys are in the order of their
//! names, as `serde_json` writes them.

use serde_json::{Value, json};

/// Whether a broadcast began or ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastState {
    /// The room has a broadcast, and the reader connects to it.
    Began,
    /// The broadcast left, or its session ended.
    Ended,
}

/// Why a frame was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// The agent asked for it (`/frame`).
    Command,
    /// `--frame-every-ms`.
    Every,
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A line of the room's chat.
    Chat {
        /// Its number in the room (the Group of `lines`).
        number: u64,
        /// Who said it.
        from: String,
        /// What.
        text: String,
        /// When it was said, on the clock of who said it (ms).
        sent_at: u64,
        /// When the gatherer took it, on its clock (ms).
        gathered_at: u64,
    },
    /// Lines of the chat that were said while the reader was away and are
    /// not fetched: those after `after` and before `before`, more than
    /// `--chat-catch-up-lines`. Written before the lines after them.
    ChatGap {
        /// The last line the reader wrote.
        after: u64,
        /// The first line fetched.
        before: u64,
    },
    /// Somebody's chat namespace was announced: they came.
    Joined {
        /// Their name in the chat.
        name: String,
    },
    /// Somebody's chat namespace was withdrawn: they left.
    Left {
        /// Their name in the chat.
        name: String,
    },
    /// The room's broadcast began or ended.
    Broadcast {
        /// Which.
        state: BroadcastState,
        /// The fields of its Track Namespace.
        namespace: Vec<String>,
    },
    /// A picture of the broadcast, written to a file.
    Frame {
        /// Where the PNG is.
        path: String,
        /// The Group of the frame.
        group: u64,
        /// The Object of the frame.
        object: u64,
        /// The Timestamp of LOC of the frame (µs).
        timestamp_us: u64,
        /// Its size.
        width: u32,
        /// Its size.
        height: u32,
        /// Why it was taken.
        asked: Asked,
    },
    /// Something said in the broadcast, as text.
    Speech {
        /// Who said it.
        from: String,
        /// What.
        text: String,
        /// When it began, on the clock of the Timestamps of LOC (µs).
        start_us: u64,
        /// When it ended, on that clock (µs).
        end_us: u64,
    },
    /// Something went wrong; the reader goes on.
    Error {
        /// What, for a person or an agent to read.
        message: String,
    },
}

impl Event {
    /// The event as one line of JSON, without its newline, written at `at`
    /// (ms since the Unix epoch).
    #[must_use]
    pub fn line(&self, at: u64) -> String {
        let value = match self {
            Self::Chat {
                number,
                from,
                text,
                sent_at,
                gathered_at,
            } => json!({
                "type": "chat",
                "number": number,
                "from": from,
                "text": text,
                "sent_at": sent_at,
                "gathered_at": gathered_at,
            }),
            Self::ChatGap { after, before } => {
                json!({"type": "chat_gap", "after": after, "before": before})
            }
            Self::Joined { name } => json!({"type": "joined", "name": name}),
            Self::Left { name } => json!({"type": "left", "name": name}),
            Self::Broadcast { state, namespace } => json!({
                "type": "broadcast",
                "state": match state {
                    BroadcastState::Began => "began",
                    BroadcastState::Ended => "ended",
                },
                "namespace": namespace,
            }),
            Self::Frame {
                path,
                group,
                object,
                timestamp_us,
                width,
                height,
                asked,
            } => json!({
                "type": "frame",
                "path": path,
                "group": group,
                "object": object,
                "timestamp_us": timestamp_us,
                "width": width,
                "height": height,
                "asked": match asked {
                    Asked::Command => "command",
                    Asked::Every => "every",
                },
            }),
            Self::Speech {
                from,
                text,
                start_us,
                end_us,
            } => json!({
                "type": "speech",
                "from": from,
                "text": text,
                "start_us": start_us,
                "end_us": end_us,
            }),
            Self::Error { message } => json!({"type": "error", "message": message}),
        };
        let Value::Object(mut fields) = value else {
            return String::new();
        };
        fields.insert("at".to_string(), json!(at));
        Value::Object(fields).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_event_is_one_line_of_json() {
        // (the event; the line written at 1000)
        let cases = [
            (
                Event::Chat {
                    number: 7,
                    from: "alice".to_string(),
                    text: "hello".to_string(),
                    sent_at: 990,
                    gathered_at: 995,
                },
                r#"{"at":1000,"from":"alice","gathered_at":995,"number":7,"sent_at":990,"text":"hello","type":"chat"}"#,
            ),
            // What a terminal would act on, and a line break, are escaped:
            // the line stays one line.
            (
                Event::Chat {
                    number: 8,
                    from: "b\"ob".to_string(),
                    text: "a\u{1b}[2J\nb".to_string(),
                    sent_at: 0,
                    gathered_at: 0,
                },
                r#"{"at":1000,"from":"b\"ob","gathered_at":0,"number":8,"sent_at":0,"text":"a\u001b[2J\nb","type":"chat"}"#,
            ),
            (
                Event::ChatGap {
                    after: 3,
                    before: 40,
                },
                r#"{"after":3,"at":1000,"before":40,"type":"chat_gap"}"#,
            ),
            (
                Event::Joined {
                    name: "alice".to_string(),
                },
                r#"{"at":1000,"name":"alice","type":"joined"}"#,
            ),
            (
                Event::Left {
                    name: "alice".to_string(),
                },
                r#"{"at":1000,"name":"alice","type":"left"}"#,
            ),
            (
                Event::Broadcast {
                    state: BroadcastState::Began,
                    namespace: vec!["xmoq".to_string(), "demo".to_string()],
                },
                r#"{"at":1000,"namespace":["xmoq","demo"],"state":"began","type":"broadcast"}"#,
            ),
            (
                Event::Broadcast {
                    state: BroadcastState::Ended,
                    namespace: vec!["xmoq".to_string(), "demo".to_string()],
                },
                r#"{"at":1000,"namespace":["xmoq","demo"],"state":"ended","type":"broadcast"}"#,
            ),
            (
                Event::Frame {
                    path: "/tmp/out/frames/000003-g12-o7.png".to_string(),
                    group: 12,
                    object: 7,
                    timestamp_us: 1_700_000_000_000_000,
                    width: 1280,
                    height: 720,
                    asked: Asked::Command,
                },
                r#"{"asked":"command","at":1000,"group":12,"height":720,"object":7,"path":"/tmp/out/frames/000003-g12-o7.png","timestamp_us":1700000000000000,"type":"frame","width":1280}"#,
            ),
            (
                Event::Frame {
                    path: "f.png".to_string(),
                    group: 0,
                    object: 0,
                    timestamp_us: 0,
                    width: 1,
                    height: 1,
                    asked: Asked::Every,
                },
                r#"{"asked":"every","at":1000,"group":0,"height":1,"object":0,"path":"f.png","timestamp_us":0,"type":"frame","width":1}"#,
            ),
            (
                Event::Speech {
                    from: "xmoq/demo".to_string(),
                    text: "こんにちは".to_string(),
                    start_us: 10,
                    end_us: 20,
                },
                r#"{"at":1000,"end_us":20,"from":"xmoq/demo","start_us":10,"text":"こんにちは","type":"speech"}"#,
            ),
            (
                Event::Error {
                    message: "frame: not built yet".to_string(),
                },
                r#"{"at":1000,"message":"frame: not built yet","type":"error"}"#,
            ),
        ];
        for (event, line) in cases {
            assert_eq!(event.line(1000), line, "{event:?}");
        }
    }
}
