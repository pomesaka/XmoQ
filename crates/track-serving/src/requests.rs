//! The requests the relay opens to a client that serves tracks of one Object
//! a Group ([`PeerRequests`]): a FETCH is handed to the client to answer,
//! every other request is refused, and the FETCH requests being answered are
//! kept to their end ([`crate::answers`]). With input and output: it writes
//! the answers and the refusals, and the log.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use xmoq_moqt::message::{ControlMessage, Fetch, ReasonPhrase, request_error_code};
use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::session::{
    PeerRequest, PeerRequestEvent, PeerRequestId, PeerRequestOpened, Session,
};
use xmoq_moqt::{RequestFault, UsageError};
use xmoq_moqt_io::refuse::{on_refused_closed, on_refused_message};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, PeerRequestStreamEvent, SendStream as _, StreamStops, Trace, send_message,
};

use crate::answers::{self, Fetches, Step};
use crate::fetch::{Piece, payload_bytes};
use crate::streams::{Answer, Answered, answer_fetch};

type Send = <WtConnection as Connection>::Send;

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
            Self::Usage(_) | Self::ReaderPanicked => None,
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
pub struct AskedFetch {
    /// Its request stream.
    pub stream: u64,
    /// The sending half of it.
    pub send: Send,
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
    /// (`pieces_of` of [`crate::fetch`]), held ([`PeerRequests::hold`]).
    pub pieces: Vec<Piece>,
}

/// The requests of the relay on one session.
pub struct PeerRequests {
    fetches: Fetches,
    /// The requests the session keeps after they were refused, by their
    /// stream, until the stream is gone.
    refused: HashMap<u64, PeerRequestId>,
    /// Where the tasks that answer a FETCH say how it went.
    done: mpsc::Sender<(u64, Answered)>,
    /// The answers and refusals being written.
    replies: JoinSet<()>,
}

impl PeerRequests {
    /// None yet, with `fetches` for the FETCH requests, whose tasks say how
    /// they went on `done` ([`Self::over`]).
    #[must_use]
    pub fn new(fetches: Fetches, done: mpsc::Sender<(u64, Answered)>) -> Self {
        Self {
            fetches,
            refused: HashMap::new(),
            done,
            replies: JoinSet::new(),
        }
    }

    /// The answers and refusals being written, for the loop to take as they
    /// end.
    pub fn replies(&mut self) -> &mut JoinSet<()> {
        &mut self.replies
    }

    /// An event of the relay's request streams; a FETCH, for the client.
    ///
    /// # Errors
    ///
    /// Where the session cannot go on.
    pub fn on_event(
        &mut self,
        session: &mut Session,
        stops: &StreamStops,
        event: PeerRequestStreamEvent<Send>,
    ) -> Result<Option<AskedFetch>, RequestError> {
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
                if let Some(request) = self.fetches.request(stream) {
                    let event = session.on_peer_request_message(request, message)?;
                    if let PeerRequestEvent::UpdateAccepted { response, .. } = event {
                        let to_do = self.fetches.updated(stream, response);
                        apply(to_do, session, stops);
                    }
                } else {
                    // What crossed the refusal.
                    on_refused_message(session, &self.refused, stream, message)?;
                }
            }
            PeerRequestStreamEvent::Closed { stream, how } => {
                on_refused_closed(session, &mut self.refused, stream);
                let to_do = self.fetches.closed(stream, how);
                apply(to_do, session, stops);
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
        send: Send,
        request: PeerRequestId,
        code: u64,
    ) -> Result<(), RequestError> {
        let response = session.reject(request, code, 0, ReasonPhrase::default())?;
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
        asked: AskedFetch,
        code: u64,
    ) -> Result<(), RequestError> {
        self.refuse(session, asked.stream, asked.send, asked.request, code)
    }

    /// Whether as many FETCH requests are answered as there may be: one more
    /// is refused with `EXCESSIVE_LOAD`.
    #[must_use]
    pub fn full(&self) -> bool {
        self.fetches.full()
    }

    /// The bytes an answer may take now.
    #[must_use]
    pub fn room(&self) -> usize {
        self.fetches.room()
    }

    /// The answer of `pieces` is to be held, where it fits ([`Self::room`]).
    /// Where it does not, the FETCH is to be refused with `EXCESSIVE_LOAD`.
    pub fn hold(&mut self, pieces: &[Piece]) -> bool {
        self.fetches.hold(payload_bytes(pieces))
    }

    /// Answers the FETCH `asked` (End Of Track 0: the track goes on) on its
    /// request stream and a fetch stream of its own, with what was held for
    /// it ([`Self::hold`]).
    ///
    /// # Errors
    ///
    /// Where the session does not take the `FETCH_OK`.
    pub fn answer(
        &mut self,
        connection: &Arc<WtConnection>,
        session: &mut Session,
        asked: AskedFetch,
        answer: FetchAnswer,
    ) -> Result<(), RequestError> {
        let AskedFetch {
            stream,
            send,
            request,
            ..
        } = asked;
        let messages = answers::accepted(session, request, answer.end_location)?;
        let bytes = payload_bytes(&answer.pieces);
        let answer = Answer {
            messages,
            request_id: request.value(),
            group_order: answer.group_order,
            pieces: answer.pieces,
        };
        let task = answer_fetch(connection, stream, send, answer, self.done.clone());
        self.fetches.answering(stream, request, task, bytes);
        Ok(())
    }

    /// What the task that answers the FETCH on `stream` said.
    pub fn over(
        &mut self,
        session: &mut Session,
        stops: &StreamStops,
        stream: u64,
        answered: Answered,
    ) {
        let to_do = self.fetches.over(stream, answered);
        apply(to_do, session, stops);
    }

    /// Writes the last message on a request stream of the relay, then a FIN
    /// (§3.3.2), in a task of its own.
    fn reply(&mut self, mut send: Send, response: ControlMessage) {
        self.replies.spawn(async move {
            // An error means the relay is gone or stopped the stream.
            let _ = send_message(&mut send, &response, &Trace::default(), "reply").await;
            send.finish();
        });
    }
}

/// Carries `to_do` out on `session` and `stops`, and prints the lines.
fn apply(to_do: Vec<Step<PeerRequestId>>, session: &mut Session, stops: &StreamStops) {
    for step in to_do {
        match step {
            Step::AnswerEnded(request) => session.on_fetch_answer_ended(request),
            Step::Closed(request) => session.on_peer_request_closed(request),
            Step::Stop { stream, code } => stops.stop(stream, code),
            Step::Say(said) => println!("{said}"),
        }
    }
}
