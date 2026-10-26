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
    /// The broadcast's catalog came on the session: it is there.
    Began,
    /// The catalog's subscription ended (the broadcast left), or the
    /// session ended, after it began.
    Ended,
}

/// Whether a participant of a meeting came or left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantState {
    /// Its namespace was announced.
    Joined,
    /// It was withdrawn, or not told of again on a new session.
    Left,
}

/// Why a frame was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameReason {
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
    /// A participant of the meeting came or left (`--meeting`): its
    /// broadcast under `xmoq/meet/<room>` was announced, or withdrawn.
    Participant {
        /// Which.
        state: ParticipantState,
        /// The last field of its namespace.
        id: String,
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
        asked: FrameReason,
        /// In a meeting, whose picture it is: written as `id`, and left out
        /// otherwise.
        of: Option<String>,
    },
    /// Something said in a broadcast, as the transcriber read it
    /// (`xmoq-transcript-tracks`).
    Speech {
        /// Its number in the transcript (the Group of `speech`).
        number: u64,
        /// The namespace of the broadcast whose voice it was.
        from: Vec<String>,
        /// What.
        text: String,
        /// When it began, on the clock of the Timestamps of LOC of that
        /// broadcast's audio (µs).
        start_us: u64,
        /// When it ended, on that clock (µs).
        end_us: u64,
        /// When the transcriber had read it, on its clock (ms).
        read_at: u64,
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
            Self::Participant { state, id } => json!({
                "type": "participant",
                "state": match state {
                    ParticipantState::Joined => "joined",
                    ParticipantState::Left => "left",
                },
                "id": id,
            }),
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
                of,
            } => {
                let mut frame = json!({
                "type": "frame",
                "path": path,
                "group": group,
                "object": object,
                "timestamp_us": timestamp_us,
                "width": width,
                "height": height,
                "asked": match asked {
                    FrameReason::Command => "command",
                    FrameReason::Every => "every",
                },
                });
                if let (Some(of), Value::Object(fields)) = (of, &mut frame) {
                    fields.insert("id".to_string(), json!(of));
                }
                frame
            }
            Self::Speech {
                number,
                from,
                text,
                start_us,
                end_us,
                read_at,
            } => json!({
                "type": "speech",
                "number": number,
                "from": from,
                "text": text,
                "start_us": start_us,
                "end_us": end_us,
                "read_at": read_at,
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

    /// The rows of `fixtures/agent-lines/lines.tsv`, which `@xmoq/meeting`
    /// reads as well: the lines both write, each with the event that is to
    /// write it.
    #[test]
    fn the_lines_both_write_are_as_the_shared_table_says() {
        let path = format!(
            "{}/../../fixtures/agent-lines/lines.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let events = [
            (
                "a participant came",
                Event::Participant {
                    state: ParticipantState::Joined,
                    id: "p1".to_string(),
                },
            ),
            (
                "a participant left",
                Event::Participant {
                    state: ParticipantState::Left,
                    id: "p1".to_string(),
                },
            ),
            (
                "a line of the chat",
                Event::Chat {
                    number: 7,
                    from: "alice".to_string(),
                    text: "hello".to_string(),
                    sent_at: 990,
                    gathered_at: 995,
                },
            ),
            (
                "a line with what a terminal would act on, and a line break",
                Event::Chat {
                    number: 8,
                    from: "b\"ob".to_string(),
                    text: "a\u{1b}[2J\nb".to_string(),
                    sent_at: 0,
                    gathered_at: 0,
                },
            ),
            (
                "a piece of speech",
                Event::Speech {
                    number: 3,
                    from: ["xmoq", "meet", "demo", "p1"].map(str::to_string).to_vec(),
                    text: "こんにちは".to_string(),
                    start_us: 10,
                    end_us: 20,
                    read_at: 30,
                },
            ),
            (
                "a picture of a participant, at a path from the folder",
                Event::Frame {
                    path: "frames/1700000000000/000001-g12-o7.png".to_string(),
                    group: 12,
                    object: 7,
                    timestamp_us: 1_700_000_000_000_000,
                    width: 320,
                    height: 180,
                    asked: FrameReason::Command,
                    of: Some("p1".to_string()),
                },
            ),
            (
                "an error",
                Event::Error {
                    message:
                        "no command /nothing: /frame is one, and // begins a line that is said"
                            .to_string(),
                },
            ),
        ];
        let rows: Vec<(&str, &str)> = table
            .lines()
            .map(|row| row.split_once('\t').expect("a name and a line"))
            .collect();
        // The names of the table are those of the events here, and no more:
        // a row added there and not here is counted, as `@xmoq/meeting` does.
        let mut named: Vec<&str> = rows.iter().map(|(name, _)| *name).collect();
        let mut ours: Vec<&str> = events.iter().map(|(name, _)| *name).collect();
        named.sort_unstable();
        ours.sort_unstable();
        assert_eq!(named, ours, "{path}");
        for (name, event) in &events {
            let line = rows
                .iter()
                .find(|(row, _)| row == name)
                .map(|(_, line)| *line)
                .expect("named");
            assert_eq!(event.line(1000), line, "{name}");
        }
    }

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
                    path: "frames/1700000000000/000003-g12-o7.png".to_string(),
                    group: 12,
                    object: 7,
                    timestamp_us: 1_700_000_000_000_000,
                    width: 1280,
                    height: 720,
                    asked: FrameReason::Command,
                    of: None,
                },
                r#"{"asked":"command","at":1000,"group":12,"height":720,"object":7,"path":"frames/1700000000000/000003-g12-o7.png","timestamp_us":1700000000000000,"type":"frame","width":1280}"#,
            ),
            (
                Event::Speech {
                    number: 3,
                    from: vec!["xmoq".to_string(), "demo".to_string()],
                    text: "こんにちは".to_string(),
                    start_us: 10,
                    end_us: 20,
                    read_at: 30,
                },
                r#"{"at":1000,"end_us":20,"from":["xmoq","demo"],"number":3,"read_at":30,"start_us":10,"text":"こんにちは","type":"speech"}"#,
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

    #[test]
    fn a_frame_and_a_participant_are_lines_of_json() {
        // (the event; the line written at 1000)
        let cases = [
            (
                Event::Frame {
                    path: "f.png".to_string(),
                    group: 0,
                    object: 0,
                    timestamp_us: 0,
                    width: 1,
                    height: 1,
                    asked: FrameReason::Every,
                    of: None,
                },
                r#"{"asked":"every","at":1000,"group":0,"height":1,"object":0,"path":"f.png","timestamp_us":0,"type":"frame","width":1}"#,
            ),
            (
                Event::Frame {
                    path: "f.png".to_string(),
                    group: 0,
                    object: 0,
                    timestamp_us: 0,
                    width: 1,
                    height: 1,
                    asked: FrameReason::Command,
                    of: Some("p1".to_string()),
                },
                r#"{"asked":"command","at":1000,"group":0,"height":1,"id":"p1","object":0,"path":"f.png","timestamp_us":0,"type":"frame","width":1}"#,
            ),
            (
                Event::Participant {
                    state: ParticipantState::Joined,
                    id: "p1".to_string(),
                },
                r#"{"at":1000,"id":"p1","state":"joined","type":"participant"}"#,
            ),
            (
                Event::Participant {
                    state: ParticipantState::Left,
                    id: "p1".to_string(),
                },
                r#"{"at":1000,"id":"p1","state":"left","type":"participant"}"#,
            ),
        ];
        for (event, line) in cases {
            assert_eq!(event.line(1000), line, "{event:?}");
        }
    }
}
