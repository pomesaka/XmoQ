//! The requests the relay opens to a client that serves tracks of one Object
//! a Group ([`PeerRequests`]): a FETCH is handed to the client to answer,
//! every other request is refused, and the FETCH requests being answered are
//! kept to their end ([`crate::answers`]). With input and output: it writes
//! the answers and the refusals, and the log.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::task::JoinSet;
use xmoq_moqt::fetch::FetchHeader;
use xmoq_moqt::message::{ControlMessage, Fetch, ReasonPhrase, request_error_code, retry_interval};
use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::session::{
    PeerRequest, PeerRequestEvent, PeerRequestId, PeerRequestOpened, Session,
};
use xmoq_moqt::{RequestFault, UsageError};
use xmoq_moqt_io::refuse::{on_refused_closed, on_refused_message};
use xmoq_moqt_io::{
    Connection, PeerRequestStreamEvent, SendStream, StreamStops, Trace, send_message,
};

use crate::answers::{self, Answer, Answers, Bounds, Conclusion, Happened, Note, Pace};
use crate::fetch::Piece;

/// Why the session cannot go on after a request of the relay.
#[derive(Debug)]
pub enum RequestError {
    /// The relay broke a rule: the session is closed with the code of the
    /// error (`termination_code` of `xmoq-moqt-io`'s `client`).
    Peer(xmoq_moqt::Error),
    /// The client used its session wrongly.
    Usage(String),
    /// A reader of a request stream panicked.
    ReaderPanicked,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Peer(error) => write!(formatter, "{error}"),
            Self::Usage(why) => write!(formatter, "the client used its session wrongly: {why}"),
            Self::ReaderPanicked => write!(formatter, "a reader of a request stream panicked"),
        }
    }
}

impl std::error::Error for RequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Peer(error) => Some(error),
            Self::Usage(_) => None,
            // What the code of the close is found by (`termination_code` of
            // `xmoq-moqt-io`'s `client`).
            Self::ReaderPanicked => Some(&xmoq_moqt_io::client::ReaderPanicked),
        }
    }
}

impl From<xmoq_moqt::Error> for RequestError {
    fn from(error: xmoq_moqt::Error) -> Self {
        Self::Peer(error)
    }
}

impl From<UsageError> for RequestError {
    fn from(error: UsageError) -> Self {
        Self::Usage(error.to_string())
    }
}

impl From<RequestFault> for RequestError {
    fn from(fault: RequestFault) -> Self {
        match fault {
            RequestFault::Peer(error) => Self::Peer(error),
            RequestFault::Usage(error) => error.into(),
        }
    }
}

/// A FETCH of the relay for the client to answer ([`PeerRequests::answer`])
/// or refuse ([`PeerRequests::refuse`]).
pub struct AskedFetch<S> {
    /// Its request stream.
    pub stream: u64,
    /// The sending half of it.
    pub send: S,
    /// The request.
    pub request: PeerRequestId,
    /// The FETCH.
    pub fetch: Fetch,
}

/// The answer to a FETCH ([`PeerRequests::answer`]).
pub struct FetchAnswer {
    /// Where its `FETCH_OK` ends (`Plan::end_location` of [`crate::fetch`]).
    pub end_location: Location,
    /// Its Group Order ([`answers::group_order`]).
    pub group_order: GroupOrder,
    /// What the fetch stream carries, in the order it is sent
    /// (`pieces_of` of [`crate::fetch`]), within what the answers may hold
    /// ([`PeerRequests::fits`]).
    pub pieces: Vec<Piece>,
}

/// The requests of the relay on one session, of any [`Connection`]: what
/// is generic is the streams its methods are given.
pub struct PeerRequests {
    answers: Answers,
    /// The requests the session keeps after the client ended them, refused
    /// or answered, by their stream, until the stream is gone.
    refused: HashMap<u64, PeerRequestId>,
    /// The answers and refusals being written.
    replies: JoinSet<()>,
}

impl PeerRequests {
    /// None yet; the FETCH requests are answered within `bounds`.
    #[must_use]
    pub fn new(bounds: Bounds) -> Self {
        Self {
            // Such a client answers as fast as it can.
            answers: Answers::new(Pace::default(), bounds),
            refused: HashMap::new(),
            replies: JoinSet::new(),
        }
    }

    /// An event of the relay's request streams; a FETCH, for the client.
    ///
    /// # Errors
    ///
    /// Where the session cannot go on.
    pub fn on_event<S: SendStream>(
        &mut self,
        session: &mut Session,
        stops: &StreamStops,
        event: PeerRequestStreamEvent<S>,
    ) -> Result<Option<AskedFetch<S>>, RequestError> {
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                send,
                first,
            } => match session.on_peer_request_opened(first)? {
                PeerRequestOpened::Refused { response, request } => {
                    if let Some(request) = request {
                        self.refused.insert(stream, request);
                    }
                    self.reply(send, response);
                }
                PeerRequestOpened::Request(request, PeerRequest::Fetch(fetch)) => {
                    return Ok(Some(AskedFetch {
                        stream,
                        send,
                        request,
                        fetch,
                    }));
                }
                // Such a client is asked for nothing else: it announces no
                // namespace, and is the subscriber of no PUBLISH it takes.
                PeerRequestOpened::Request(request, _) => {
                    self.refuse(
                        session,
                        stream,
                        send,
                        request,
                        request_error_code::NOT_SUPPORTED,
                    )?;
                }
            },
            PeerRequestStreamEvent::Message { stream, message } => {
                if let Some(request) = self.answers.request_on(stream) {
                    // A REQUEST_UPDATE for a FETCH that is answered changes
                    // nothing and gets its REQUEST_OK.
                    let event = session.on_peer_request_message(request, message)?;
                    if let PeerRequestEvent::UpdateAccepted { response, .. } = event
                        && let Some(over) = self.answers.say(stream, response)
                    {
                        self.over(session, stops, stream, &over);
                    }
                } else {
                    // What crossed the refusal or the end of the answer.
                    on_refused_message(session, &self.refused, stream, message)?;
                }
            }
            PeerRequestStreamEvent::Closed { stream, how } => {
                if let Some(over) = self.answers.happened(stream, Happened::PeerClosed(how)) {
                    self.over(session, stops, stream, &over);
                }
                on_refused_closed(session, &mut self.refused, stream);
            }
            PeerRequestStreamEvent::Failed(error) => return Err(error.into()),
            PeerRequestStreamEvent::ReaderPanicked => return Err(RequestError::ReaderPanicked),
        }
        Ok(None)
    }

    /// Refuses `request` with `code`.
    ///
    /// # Errors
    ///
    /// Where the session does not take the refusal.
    pub fn refuse(
        &mut self,
        session: &mut Session,
        stream: u64,
        send: impl SendStream,
        request: PeerRequestId,
        code: u64,
    ) -> Result<(), RequestError> {
        // EXCESSIVE_LOAD with a Retry Interval: room comes back once other
        // answers are written.
        let retry = retry_interval::made_up(code);
        let response = session.reject(request, code, retry, ReasonPhrase::default())?;
        self.refused.insert(stream, request);
        self.reply(send, response);
        Ok(())
    }

    /// Refuses the FETCH `asked` with `code`.
    ///
    /// # Errors
    ///
    /// Where the session does not take the refusal.
    pub fn refuse_fetch(
        &mut self,
        session: &mut Session,
        asked: AskedFetch<impl SendStream>,
        code: u64,
    ) -> Result<(), RequestError> {
        self.refuse(session, asked.stream, asked.send, asked.request, code)
    }

    /// Whether as many FETCH requests are answered as there may be: one more
    /// is refused with `EXCESSIVE_LOAD`.
    #[must_use]
    pub fn full(&self) -> bool {
        !self.answers.has_room()
    }

    /// The bytes an answer may take now.
    #[must_use]
    pub fn room(&self) -> usize {
        self.answers.room()
    }

    /// Whether an answer of `pieces` fits in what the answers may hold.
    /// Where it does not, the FETCH is to be refused with `EXCESSIVE_LOAD`.
    #[must_use]
    pub fn fits(&self, pieces: &[Piece]) -> bool {
        self.answers.fits(pieces)
    }

    /// Answers the FETCH `asked` (End Of Track 0: the track goes on) on its
    /// request stream and a fetch stream of its own, with an answer that
    /// fits ([`Self::fits`]).
    ///
    /// # Errors
    ///
    /// Where the session does not take the `FETCH_OK`.
    pub fn answer<C: Connection>(
        &mut self,
        connection: &Arc<C>,
        session: &mut Session,
        asked: AskedFetch<C::Send>,
        answer: FetchAnswer,
    ) -> Result<(), RequestError> {
        let AskedFetch {
            stream,
            send,
            request,
            ..
        } = asked;
        let messages = answers::accepted(session, request, answer.end_location)?;
        let answer = Answer {
            messages,
            header: FetchHeader {
                request_id: request.value(),
            },
            group_order: answer.group_order,
            pieces: answer.pieces,
        };
        self.answers
            .start(stream, request, send, answer, connection, &Trace::default());
        Ok(())
    }

    /// The next note of a task that answers a FETCH, for [`Self::on_note`];
    /// the answers and refusals written meanwhile are let go of. Never,
    /// while no task has one.
    pub async fn next_note(&mut self) -> Note {
        loop {
            tokio::select! {
                Some(_) = self.replies.join_next() => {}
                note = self.answers.next_note() => return note,
            }
        }
    }

    /// What a task that answers a FETCH noted.
    pub fn on_note(&mut self, session: &mut Session, stops: &StreamStops, note: Note) {
        let Note(stream, happened) = note;
        if let Some(over) = self.answers.happened(stream, happened) {
            self.over(session, stops, stream, &over);
        }
    }

    /// The FETCH on `stream` is over, or a step after that: the log, the
    /// session and the subscriber's direction.
    fn over(&mut self, session: &mut Session, stops: &StreamStops, stream: u64, over: &Conclusion) {
        if let Some(served) = over.served {
            println!("{served}");
        }
        answers::conclude(over, stream, session, &mut self.refused, stops);
    }

    /// Writes the last message on a request stream of the relay, then a FIN
    /// (§3.3.2), in a task of its own.
    fn reply(&mut self, mut send: impl SendStream, response: ControlMessage) {
        self.replies.spawn(async move {
            // An error means the relay is gone or stopped the stream.
            let _ = send_message(&mut send, &response, &Trace::default(), "reply").await;
            send.finish();
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use xmoq_moqt::message::{FetchRange, Subscribe};
    use xmoq_moqt::name::{FullTrackName, TrackNamespace};
    use xmoq_moqt::session::{Role, Transport};
    use xmoq_moqt_io::memory;
    use xmoq_moqt_io::{RequestClose, RequestReader, RequestStreamEvent};

    use super::*;
    use crate::answers::{MAX_ANSWERED_AT_ONCE, SAY_QUEUE_OF_A_PAGE};

    fn track() -> FullTrackName {
        FullTrackName::new(
            TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
            b"a".to_vec(),
        )
        .unwrap()
    }

    /// A SUBSCRIBE and a FETCH of the relay, with its first Request ID
    /// (§10.1: a server's are odd).
    fn subscribe() -> ControlMessage {
        ControlMessage::Subscribe(Subscribe {
            request_id: 1,
            full_track_name: track(),
            parameters: vec![],
        })
    }

    fn fetch() -> ControlMessage {
        ControlMessage::Fetch(Fetch {
            request_id: 1,
            range: FetchRange::Standalone {
                full_track_name: track(),
                start: Location {
                    group: 0,
                    object: 0,
                },
                end: Location {
                    group: 1,
                    object: 0,
                },
            },
            parameters: vec![],
        })
    }

    #[tokio::test]
    async fn a_fetch_of_the_relay_is_handed_on_and_any_other_request_refused() {
        // (case, the request, the code of its refusal; none: handed on)
        let cases = [
            ("a FETCH", fetch(), None),
            (
                "a SUBSCRIBE",
                subscribe(),
                Some(request_error_code::NOT_SUPPORTED),
            ),
        ];
        for (case, request, refused) in cases {
            let (client, relay) = memory::pair();
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let mut requests = PeerRequests::new(Bounds {
                at_once: MAX_ANSWERED_AT_ONCE,
                bytes: 1024,
                say_queue: SAY_QUEUE_OF_A_PAGE,
            });
            let (mut relay_send, relay_recv) = relay.open_bi().await.unwrap();
            send_message(&mut relay_send, &request, &Trace::default(), "relay")
                .await
                .unwrap();
            let (send, recv) = client.accept_bi().await.unwrap();
            let mut reader = RequestReader::new(recv, &Trace::default(), "client");
            let Ok(RequestStreamEvent::Message(first)) = reader.next_event().await else {
                panic!("{case}: the request");
            };
            let event = PeerRequestStreamEvent::Opened {
                stream: 0,
                send,
                first,
            };
            let asked = requests
                .on_event(&mut session, &StreamStops::default(), event)
                .unwrap();
            match refused {
                None => {
                    let asked = asked.expect(case);
                    assert_eq!((asked.stream, asked.fetch.request_id), (0, 1), "{case}");
                }
                Some(code) => {
                    assert!(asked.is_none(), "{case}");
                    // The refusal, then the client's FIN (§3.3.2).
                    let mut relay_reader =
                        RequestReader::new(relay_recv, &Trace::default(), "relay");
                    let in_time = Duration::from_secs(5);
                    let answer = tokio::time::timeout(in_time, relay_reader.next_event())
                        .await
                        .expect("in time")
                        .unwrap();
                    let RequestStreamEvent::Message(ControlMessage::RequestError(error)) = answer
                    else {
                        panic!("{case}: {answer:?}");
                    };
                    assert_eq!(error.error_code, code, "{case}");
                    let end = tokio::time::timeout(in_time, relay_reader.next_event())
                        .await
                        .expect("in time")
                        .unwrap();
                    assert!(
                        matches!(end, RequestStreamEvent::Closed(RequestClose::Fin)),
                        "{case}: {end:?}"
                    );
                }
            }
        }
    }
}
