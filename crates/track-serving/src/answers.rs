//! The FETCH requests of the relay that a client answers, from the answer
//! it writes to the end of both directions of the request stream
//! ([`Fetches`]). Without input or output: what is to be done to the session
//! and the streams comes back as [`Step`]s, which [`crate::requests`]
//! carries out.

use std::collections::HashMap;

use xmoq_moqt::message::{ControlMessage, Fetch, RequestOk};
use xmoq_moqt::name::Location;
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{PeerRequestId, Session};
use xmoq_moqt::{UsageError, stream_reset_code};
use xmoq_moqt_io::PeerRequestClose;

use crate::fetch::Held;
use crate::streams::{Answered, Answering};

/// The FETCH requests of the relay answered at once, as many as the Rust
/// client answers (`MAX_ANSWERED_AT_ONCE` of `xmoq-moqt-client`); one more
/// is refused with `EXCESSIVE_LOAD`. WHY twice the 16 the relay asks one
/// publisher for: as there. A FETCH
/// that was answered and waits for the relay to end its request stream is
/// not counted; what bounds those is the session, which keeps no more than
/// 256 requests of the relay (`MAX_PEER_REQUESTS`). Provisional.
pub const FETCHES_AT_ONCE: usize = 32;

/// A request of the relay, as [`Fetches`] keeps it.
pub trait Request: Copy {
    /// Its Request ID, for the log.
    fn value(self) -> u64;
}

impl Request for PeerRequestId {
    fn value(self) -> u64 {
        PeerRequestId::value(self)
    }
}

/// What is to be done after a FETCH changed.
#[derive(Debug, PartialEq, Eq)]
pub enum Step<R> {
    /// Nothing is answered on the request stream from now on (§10.1: a
    /// `REQUEST_UPDATE` still uses up its Request ID):
    /// `Session::on_fetch_answer_ended`.
    AnswerEnded(R),
    /// The request is over: `Session::on_peer_request_closed`.
    Closed(R),
    /// The relay's direction of the request stream is stopped with `code`.
    Stop {
        /// The request stream.
        stream: u64,
        /// The code.
        code: u32,
    },
    /// A line for the log.
    Say(Said),
}

/// The lines of the log of an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said {
    /// The answer ended: written, or not.
    Served {
        /// Its Request ID.
        request: u64,
        /// How.
        how: Answered,
    },
    /// The relay's direction of a request stream that was answered was
    /// stopped.
    RequestStopped {
        /// Its Request ID.
        request: u64,
    },
}

impl std::fmt::Display for Said {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Served { request, how } => {
                write!(formatter, "FETCH_SERVED request_id={request} how={how:?}")
            }
            Self::RequestStopped { request } => {
                write!(formatter, "FETCH_REQUEST_STOPPED request_id={request}")
            }
        }
    }
}

/// A FETCH of the relay that is answered.
struct Fetching<R> {
    request: R,
    answering: Answering,
    /// Its answer was written, or ended otherwise.
    over: bool,
    /// The relay finished its direction of the request stream.
    peer_finished: bool,
    /// The bytes of payload the answer holds until it is written ([`Held`]).
    bytes: usize,
}

/// The FETCH requests of the relay that are answered, by their request
/// stream, and the bytes their answers hold.
pub struct Fetches<R = PeerRequestId> {
    answered: HashMap<u64, Fetching<R>>,
    most: usize,
    held: Held,
}

impl<R: Request> Fetches<R> {
    /// No FETCH, no more than `most` answered at once, and no more than
    /// `most_bytes` of payload held by their answers.
    #[must_use]
    pub fn new(most: usize, most_bytes: usize) -> Self {
        Self {
            answered: HashMap::new(),
            most,
            held: Held::new(most_bytes),
        }
    }

    /// Whether as many are answered as there may be: one more is refused
    /// with `EXCESSIVE_LOAD`.
    #[must_use]
    pub fn full(&self) -> bool {
        self.answered
            .values()
            .filter(|fetching| !fetching.over)
            .count()
            >= self.most
    }

    /// The bytes an answer may take now.
    #[must_use]
    pub fn room(&self) -> usize {
        self.held.room()
    }

    /// An answer of `bytes` is to be held, where it fits ([`Self::room`]).
    pub fn hold(&mut self, bytes: usize) -> bool {
        self.held.take(bytes)
    }

    /// The FETCH on `stream` is answered by `answering`, with `bytes` held
    /// ([`Self::hold`]).
    pub fn answering(&mut self, stream: u64, request: R, answering: Answering, bytes: usize) {
        self.answered.insert(
            stream,
            Fetching {
                request,
                answering,
                over: false,
                peer_finished: false,
                bytes,
            },
        );
    }

    /// The request of the FETCH on `stream`, where one is answered there.
    #[must_use]
    pub fn request(&self, stream: u64) -> Option<R> {
        self.answered.get(&stream).map(|fetching| fetching.request)
    }

    /// A `REQUEST_UPDATE` of the FETCH on `stream` was accepted, which
    /// changes nothing, and gets `response` (§10.9). None is dropped
    /// (§10.9: "exactly one"): where it does not fit, the FETCH is ended
    /// with `EXCESSIVE_LOAD` instead.
    pub fn updated(&mut self, stream: u64, response: ControlMessage) -> Vec<Step<R>> {
        let Some(fetching) = self.answered.get(&stream) else {
            return Vec::new();
        };
        if fetching.answering.says.try_send(response).is_ok() {
            return Vec::new();
        }
        self.end(stream, stream_reset_code::EXCESSIVE_LOAD)
    }

    /// What the task that answers the FETCH on `stream` said.
    pub fn over(&mut self, stream: u64, answered: Answered) -> Vec<Step<R>> {
        let Some(fetching) = self.answered.get_mut(&stream) else {
            return Vec::new();
        };
        let request = fetching.request;
        let served = Step::Say(Said::Served {
            request: request.value(),
            how: answered,
        });
        match answered {
            Answered::Written => {
                fetching.over = true;
                self.held.give_back(std::mem::take(&mut fetching.bytes));
                let mut steps = vec![served, Step::AnswerEnded(request)];
                if fetching.peer_finished {
                    self.answered.remove(&stream);
                    steps.push(Step::Closed(request));
                }
                steps
            }
            // The client's direction is over and the relay's is not: it is
            // stopped. WHY: a relay that never ends it would keep the request
            // in the session, which holds 256 of the relay's
            // (`MAX_PEER_REQUESTS`), and refuses every one after. The own
            // relay finishes its direction of a FETCH it passed on once the
            // fetch stream reached its FIN (`finish_if_served` of
            // `crates/relay-core/src/fetch.rs`), and stops a subscriber's
            // direction 2 seconds after its own FIN; the client stops the
            // relay's at its own FIN, which is 3 seconds after the end of the
            // fetch stream at the most (row on the request stream of a FETCH
            // that was served to its end, `docs/moqt-implementation-notes.md`).
            Answered::RequestOver { peer_ended: false } => {
                let mut steps = vec![Step::Say(Said::RequestStopped {
                    request: request.value(),
                })];
                steps.extend(self.end(stream, stream_reset_code::CANCELLED));
                steps
            }
            Answered::RequestOver { peer_ended: true } => Vec::new(),
            // The task has reset its directions (§5.2: "It MUST reset the
            // bidi request stream and unidirectional data stream").
            Answered::Stopped | Answered::Failed => {
                let mut steps = vec![served];
                steps.extend(self.end(stream, stream_reset_code::CANCELLED));
                steps
            }
        }
    }

    /// Ends the FETCH on `stream` on both streams with `code`, where they
    /// are not over, and forgets it.
    pub fn end(&mut self, stream: u64, code: u32) -> Vec<Step<R>> {
        let Some(mut fetching) = self.answered.remove(&stream) else {
            return Vec::new();
        };
        if let Some(cancel) = fetching.answering.cancel.take() {
            let _ = cancel.send(code);
        }
        self.held.give_back(fetching.bytes);
        let mut steps = Vec::new();
        if !fetching.peer_finished {
            steps.push(Step::Stop { stream, code });
        }
        steps.push(Step::Closed(fetching.request));
        steps
    }

    /// The relay's direction of the request stream on `stream` is over.
    pub fn closed(&mut self, stream: u64, how: PeerRequestClose) -> Vec<Step<R>> {
        let Some(fetching) = self.answered.get_mut(&stream) else {
            return Vec::new();
        };
        fetching.peer_finished = true;
        // A subscriber that finishes its direction has not given the FETCH
        // up (§3.3.2); one that resets it has.
        if !how.is_fin() {
            return self.end(stream, stream_reset_code::CANCELLED);
        }
        if let Some(ended) = fetching.answering.peer_ended.take() {
            let _ = ended.send(());
        }
        if !fetching.over {
            return Vec::new();
        }
        let request = fetching.request;
        self.answered.remove(&stream);
        vec![Step::Closed(request)]
    }
}

/// The Group Order of a FETCH (§10.2.8: "If omitted from FETCH, the
/// receiver uses Ascending").
#[must_use]
pub fn group_order(fetch: &Fetch) -> GroupOrder {
    fetch
        .parameters
        .iter()
        .find_map(|parameter| match parameter {
            Parameter::GroupOrder(order) => Some(*order),
            _ => None,
        })
        .unwrap_or(GroupOrder::Ascending)
}

/// Accepts the FETCH `request` of a track that goes on (End Of Track 0),
/// up to `end_location`: its `FETCH_OK`, and a `REQUEST_OK` for each
/// `REQUEST_UPDATE` that came before it (§10.9).
///
/// # Errors
///
/// Those of `Session::accept_fetch`.
pub fn accepted(
    session: &mut Session,
    request: PeerRequestId,
    end_location: Location,
) -> Result<Vec<ControlMessage>, UsageError> {
    let (ok, owed) = session.accept_fetch(request, false, end_location, vec![])?;
    let mut messages = vec![ok];
    messages.extend((0..owed).map(|_| ControlMessage::RequestOk(RequestOk::default())));
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use tokio::sync::{mpsc, oneshot};

    use super::*;

    impl Request for u64 {
        fn value(self) -> u64 {
            self
        }
    }

    /// The request stream of the FETCH in the tests, and its Request ID.
    const STREAM: u64 = 4;
    const REQUEST: u64 = 2;

    /// What the loop did to a FETCH.
    #[derive(Debug, Clone, Copy)]
    enum Event {
        Over(Answered),
        Closed(PeerRequestClose),
        End(u32),
    }

    /// The answering of a FETCH whose task is the test: what the loop told
    /// it comes out of the receivers.
    struct Task {
        peer_ended: oneshot::Receiver<()>,
        cancel: oneshot::Receiver<u32>,
        _says: mpsc::Receiver<ControlMessage>,
    }

    fn answering(says: usize) -> (Answering, Task) {
        let (says, said) = mpsc::channel(says);
        let (peer_ended, peer_ended_rx) = oneshot::channel();
        let (cancel, cancel_rx) = oneshot::channel();
        let answering = Answering {
            says,
            peer_ended: Some(peer_ended),
            cancel: Some(cancel),
        };
        let task = Task {
            peer_ended: peer_ended_rx,
            cancel: cancel_rx,
            _says: said,
        };
        (answering, task)
    }

    fn served(how: Answered) -> Step<u64> {
        Step::Say(Said::Served {
            request: REQUEST,
            how,
        })
    }

    // (what happened, in order; the steps of the last; whether the FETCH
    // is still kept; whether its task was told the relay's direction
    // ended; the code its task was cancelled with, if any)
    type Case = (
        &'static str,
        Vec<Event>,
        Vec<Step<u64>>,
        bool,
        bool,
        Option<u32>,
    );
    #[expect(
        clippy::too_many_lines,
        reason = "a table of cases, one row a way a FETCH ends"
    )]
    fn ending_cases() -> Vec<Case> {
        let fin = PeerRequestClose::Fin;
        let reset = PeerRequestClose::Reset(Some(0x1));
        let written = Answered::Written;
        vec![
            (
                "written, the relay's direction still open",
                vec![Event::Over(written)],
                vec![served(written), Step::AnswerEnded(REQUEST)],
                true,
                false,
                None,
            ),
            (
                "written, then the relay finishes",
                vec![Event::Over(written), Event::Closed(fin)],
                vec![Step::Closed(REQUEST)],
                false,
                true,
                None,
            ),
            (
                "the relay finishes, then written",
                vec![Event::Closed(fin), Event::Over(written)],
                vec![
                    served(written),
                    Step::AnswerEnded(REQUEST),
                    Step::Closed(REQUEST),
                ],
                false,
                true,
                None,
            ),
            (
                "§3.3.2: the relay resets its direction",
                vec![Event::Closed(reset)],
                vec![Step::Closed(REQUEST)],
                false,
                false,
                Some(stream_reset_code::CANCELLED),
            ),
            (
                "the client's direction is over before the relay's",
                vec![
                    Event::Over(written),
                    Event::Over(Answered::RequestOver { peer_ended: false }),
                ],
                vec![
                    Step::Say(Said::RequestStopped { request: REQUEST }),
                    Step::Stop {
                        stream: STREAM,
                        code: stream_reset_code::CANCELLED,
                    },
                    Step::Closed(REQUEST),
                ],
                false,
                false,
                Some(stream_reset_code::CANCELLED),
            ),
            (
                "the relay stopped the fetch stream",
                vec![Event::Over(Answered::Stopped)],
                vec![
                    served(Answered::Stopped),
                    Step::Stop {
                        stream: STREAM,
                        code: stream_reset_code::CANCELLED,
                    },
                    Step::Closed(REQUEST),
                ],
                false,
                false,
                Some(stream_reset_code::CANCELLED),
            ),
            (
                "ended with a code, after the relay finished",
                vec![Event::Closed(fin), Event::End(0x5)],
                vec![Step::Closed(REQUEST)],
                false,
                true,
                Some(0x5),
            ),
            (
                "after the relay's direction ended, the client's: nothing more",
                vec![
                    Event::Over(written),
                    Event::Closed(fin),
                    Event::Over(Answered::RequestOver { peer_ended: true }),
                ],
                vec![],
                false,
                true,
                None,
            ),
            (
                "the fetch stream could not be written",
                vec![Event::Over(Answered::Failed)],
                vec![
                    served(Answered::Failed),
                    Step::Stop {
                        stream: STREAM,
                        code: stream_reset_code::CANCELLED,
                    },
                    Step::Closed(REQUEST),
                ],
                false,
                false,
                Some(stream_reset_code::CANCELLED),
            ),
            (
                "what comes for a FETCH that is gone",
                vec![Event::End(0x1), Event::Over(written)],
                vec![],
                false,
                false,
                Some(0x1),
            ),
        ]
    }

    #[test]
    fn a_fetch_is_over_when_its_answer_and_both_directions_are() {
        for (name, events, last, kept, told_ended, cancelled) in ending_cases() {
            let mut fetches = Fetches::<u64>::new(FETCHES_AT_ONCE, 100);
            let (answering, mut task) = answering(1);
            assert!(fetches.hold(10));
            fetches.answering(STREAM, REQUEST, answering, 10);
            let mut steps = Vec::new();
            for event in events {
                steps = match event {
                    Event::Over(answered) => fetches.over(STREAM, answered),
                    Event::Closed(how) => fetches.closed(STREAM, how),
                    Event::End(code) => fetches.end(STREAM, code),
                };
            }
            assert_eq!(steps, last, "{name}");
            assert_eq!(fetches.request(STREAM).is_some(), kept, "{name}");
            assert_eq!(task.peer_ended.try_recv().is_ok(), told_ended, "{name}");
            assert_eq!(task.cancel.try_recv().ok(), cancelled, "{name}");
            // What the answer held is given back once it was written or ended.
            assert_eq!(fetches.room(), 100, "{name}: the bytes held");
        }
    }

    #[test]
    fn as_many_are_answered_at_once_as_there_may_be() {
        let mut fetches = Fetches::<u64>::new(2, 100);
        let mut tasks = Vec::new();
        for stream in [0, 4] {
            assert!(!fetches.full());
            let (answering, task) = answering(1);
            fetches.answering(stream, stream, answering, 0);
            tasks.push(task);
        }
        assert!(fetches.full());
        // One whose answer was written waits for the relay's direction, and
        // is not counted.
        let _ = fetches.over(0, Answered::Written);
        assert!(!fetches.full());
    }

    #[test]
    fn an_answer_is_held_where_it_fits() {
        let mut fetches = Fetches::<u64>::new(FETCHES_AT_ONCE, 100);
        assert!(fetches.hold(60));
        assert_eq!(fetches.room(), 40);
        assert!(!fetches.hold(41));
        assert!(fetches.hold(40));
        assert_eq!(fetches.room(), 0);
    }

    #[test]
    fn the_answer_to_an_update_that_does_not_fit_ends_the_fetch() {
        // (room for answers on the request stream; the steps)
        let cases: [(usize, Vec<Step<u64>>); 2] = [
            (1, vec![]),
            (
                // §10.9: "exactly one"; none is dropped.
                0,
                vec![
                    Step::Stop {
                        stream: STREAM,
                        code: stream_reset_code::EXCESSIVE_LOAD,
                    },
                    Step::Closed(REQUEST),
                ],
            ),
        ];
        for (room, expected) in cases {
            let mut fetches = Fetches::<u64>::new(FETCHES_AT_ONCE, 100);
            // A channel takes one message at the least: one that is full
            // stands for no room.
            let (answering, _task) = answering(1);
            if room == 0 {
                answering
                    .says
                    .try_send(ControlMessage::RequestOk(RequestOk::default()))
                    .unwrap();
            }
            fetches.answering(STREAM, REQUEST, answering, 0);
            let steps = fetches.updated(STREAM, ControlMessage::RequestOk(RequestOk::default()));
            assert_eq!(steps, expected, "room {room}");
        }
    }
}
