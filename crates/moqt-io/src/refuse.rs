//! The requests a client refuses: what it does with each event of the
//! peer's request streams ([`refusal`]), and with what still comes on a
//! stream it refused ([`on_refused_message`], [`on_refused_closed`]).
//!
//! WHY here, once: the Rust client, the viewers and the clients that serve
//! a FETCH (`xmoq-publisher`) refuse requests by the same rule, and a
//! copy in each would part from the others. WHY in this crate: it is the
//! one all of them depend on that has the events of the peer's request
//! streams (`PeerRequestStreamEvent`).

use std::collections::HashMap;
use std::hash::BuildHasher;

use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::session::{PeerRequestId, Session};
use xmoq_moqt::{RequestFault, TerminationCode};

use crate::PeerRequestStreamEvent;

/// What a client that serves no requests does with one event of the peer's
/// request streams ([`refusal`]).
#[derive(Debug)]
pub enum Refusal<S> {
    /// It answers the request, with a `REQUEST_ERROR` and a FIN (§3.3.2).
    Answer {
        /// The reader's name of the request stream.
        stream: u64,
        /// The sending half of it.
        send: S,
        /// The `REQUEST_ERROR`.
        response: ControlMessage,
    },
    /// Nothing is to be done.
    Nothing,
    /// The session is closed.
    Close {
        /// The Session Termination Error Code (§3.5).
        code: u32,
        /// The reason.
        why: String,
        /// The fault of the peer, where it is one.
        error: Option<xmoq_moqt::Error>,
    },
}

impl<S> Refusal<S> {
    fn closed(error: xmoq_moqt::Error) -> Self {
        Self::Close {
            code: error.termination_code().map_or(0, TerminationCode::code),
            why: error.to_string(),
            error: Some(error),
        }
    }
}

/// What a client that serves no requests does with `event`, by `session`
/// and the requests it keeps as `refused`, by the reader's name of their
/// stream: every request is refused (`Session::refuse_peer_request`).
pub fn refusal<T, H: BuildHasher>(
    session: &mut Session,
    refused: &mut HashMap<u64, PeerRequestId, H>,
    event: PeerRequestStreamEvent<T>,
) -> Refusal<T> {
    match event {
        PeerRequestStreamEvent::Opened {
            stream,
            send,
            first,
        } => match session.refuse_peer_request(first) {
            Ok((response, request)) => {
                if let Some(request) = request {
                    refused.insert(stream, request);
                }
                Refusal::Answer {
                    stream,
                    send,
                    response,
                }
            }
            Err(error) => Refusal::closed(error),
        },
        PeerRequestStreamEvent::Message { stream, message } => {
            match on_refused_message(session, refused, stream, message) {
                Ok(()) => Refusal::Nothing,
                Err(error) => Refusal::closed(error),
            }
        }
        PeerRequestStreamEvent::Closed { stream, .. } => {
            on_refused_closed(session, refused, stream);
            Refusal::Nothing
        }
        // A stream that breaks the protocol in its bytes is reported here
        // alone, by the reader of the peer's request streams.
        PeerRequestStreamEvent::Failed(error) => Refusal::closed(error),
        // The stream is lost, and what the peer sent on it: the session
        // cannot go on with a hole in it, and the fault is on this side, as
        // the reader of the peer's other streams has it
        // (`UniEvent::ReaderPanicked`). §3.5: "INTERNAL_ERROR (0x1): An
        // implementation specific error occurred".
        PeerRequestStreamEvent::ReaderPanicked => Refusal::Close {
            code: TerminationCode::InternalError.code(),
            why: crate::client::ReaderPanicked.to_string(),
            error: None,
        },
    }
}

/// A message on `stream`, where it is one of the streams of `refused`, the
/// requests of the peer this endpoint ended itself (refused, or ended with
/// `PUBLISH_DONE` or its answer): the session takes what crossed that, as a
/// `REQUEST_UPDATE` uses up a Request ID (§10.1).
/// Nothing for another stream: the session keeps nothing of it
/// (`MAX_PEER_REQUESTS`).
///
/// # Errors
///
/// A fault of the peer, to close the session with its code.
pub fn on_refused_message<H: BuildHasher>(
    session: &mut Session,
    refused: &HashMap<u64, PeerRequestId, H>,
    stream: u64,
    message: ControlMessage,
) -> Result<(), xmoq_moqt::Error> {
    let Some(request) = refused.get(&stream) else {
        return Ok(());
    };
    match session.on_peer_request_message(*request, message) {
        Err(RequestFault::Peer(error)) => Err(error),
        // `Usage` is "`request` belongs to another session"
        // (`Session::on_peer_request_message`), and the requests of
        // `refused` are those the same session handed back: not reached.
        Err(RequestFault::Usage(_)) | Ok(_) => Ok(()),
    }
}

/// `stream` is gone; where it is one of the streams of `refused`, the
/// session forgets its request.
pub fn on_refused_closed<H: BuildHasher>(
    session: &mut Session,
    refused: &mut HashMap<u64, PeerRequestId, H>,
    stream: u64,
) {
    if let Some(request) = refused.remove(&stream) {
        session.on_peer_request_closed(request);
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::message::{RequestOk, RequestUpdate, Subscribe};
    use xmoq_moqt::name::{FullTrackName, TrackNamespace};
    use xmoq_moqt::session::{Role, Transport};

    use super::*;

    fn subscribe(request_id: u64) -> ControlMessage {
        let namespace = TrackNamespace::new(vec![b"a".to_vec()]).unwrap();
        ControlMessage::Subscribe(Subscribe {
            request_id,
            full_track_name: FullTrackName::new(namespace, b"b".to_vec()).unwrap(),
            parameters: vec![],
        })
    }

    /// What a client does with each event, after a SUBSCRIBE of the peer
    /// (Request ID 1) on stream 4 that it refused.
    #[test]
    fn a_refusing_client_answers_lets_be_or_closes_the_session() {
        // (the case; the event; what is done)
        let cases: Vec<(&str, PeerRequestStreamEvent<()>, &str)> = vec![
            (
                "another request",
                PeerRequestStreamEvent::Opened {
                    stream: 8,
                    send: (),
                    first: subscribe(3),
                },
                "answer 8",
            ),
            (
                "a request with a Request ID the peer used",
                PeerRequestStreamEvent::Opened {
                    stream: 8,
                    send: (),
                    first: subscribe(1),
                },
                "close 0x4",
            ),
            (
                // §10.1: an update uses up a Request ID of its own.
                "a REQUEST_UPDATE on the refused stream, with the Request ID of the SUBSCRIBE",
                PeerRequestStreamEvent::Message {
                    stream: 4,
                    message: ControlMessage::RequestUpdate(RequestUpdate {
                        request_id: 1,
                        parameters: vec![],
                    }),
                },
                "close 0x4",
            ),
            (
                "a message on a stream the session keeps nothing of",
                PeerRequestStreamEvent::Message {
                    stream: 12,
                    message: ControlMessage::RequestOk(RequestOk::default()),
                },
                "nothing",
            ),
            (
                "a stream that breaks the protocol in its bytes",
                PeerRequestStreamEvent::Failed(xmoq_moqt::Error::ProtocolViolation("bytes")),
                "close 0x3",
            ),
            (
                "a reader that panicked",
                PeerRequestStreamEvent::ReaderPanicked,
                "close 0x1",
            ),
        ];
        for (name, event, expected) in cases {
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            session.setup(vec![]).unwrap();
            let mut refused = HashMap::new();
            let first = PeerRequestStreamEvent::Opened {
                stream: 4,
                send: (),
                first: subscribe(1),
            };
            assert!(matches!(
                refusal(&mut session, &mut refused, first),
                Refusal::Answer { .. }
            ));
            let done = match refusal(&mut session, &mut refused, event) {
                Refusal::Answer { stream, .. } => format!("answer {stream}"),
                Refusal::Nothing => "nothing".to_owned(),
                Refusal::Close { code, .. } => format!("close {code:#x}"),
            };
            assert_eq!(done, expected, "{name}");
        }
    }
}
