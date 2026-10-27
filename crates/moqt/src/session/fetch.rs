//! The FETCH requests this endpoint sends (§5.2, §10.12, §10.13), and the
//! fetch streams that answer them (§11.4.4).

use super::{Request, RequestEvent, RequestId, Session, check_own_parameters};
use crate::immutable::track_properties_refusal;
use crate::message::{ControlMessage, Fetch, FetchOk, FetchRange};
use crate::name::{FullTrackName, Location};
use crate::params::{MessageKind, Parameter};
use crate::{Error, UsageError};

/// A FETCH this endpoint sent that is not over.
#[derive(Debug)]
pub(super) struct OwnFetch {
    /// The track: that of the FETCH, or of the SUBSCRIBE a Joining FETCH
    /// names.
    pub(super) track: FullTrackName,
    /// The Start Location, where this endpoint knows it: not for a Relative
    /// Joining FETCH, whose start the publisher counts from the subscription.
    start: Option<Location>,
    /// Whether `FETCH_OK` came.
    pub(super) answered: bool,
    pub(super) stream: StreamSeen,
}

impl OwnFetch {
    /// Whether nothing more is to come for the FETCH: it was answered, and
    /// its stream ended.
    fn is_over(&self) -> bool {
        self.answered && self.stream == StreamSeen::Ended
    }
}

/// What a fetch stream that answers no FETCH of this endpoint closes the
/// session with. [`Session::on_fetch_stream`] returns it.
///
/// FIXME: drop this once every reader of streams asks its session. It is
/// here for one that has no session at hand where the stream arrives.
pub const NO_FETCH_FOR_THE_STREAM: Error =
    Error::ProtocolViolation("fetch stream for no FETCH of this endpoint");

/// What this endpoint saw of the fetch stream of a FETCH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamSeen {
    No,
    Open,
    Ended,
}

/// What a fetch stream that arrived belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStreamOpened {
    /// A FETCH that is open. The caller reads the stream and reports its end
    /// with [`Session::on_fetch_stream_closed`].
    Fetch(RequestId),
    /// A request of this endpoint that is over. For a FETCH that was
    /// cancelled or rejected, the publisher opened the stream before it knew
    /// (§5.2: "some Objects could be received even if the FETCH results in
    /// error"). The caller stops the stream; nothing is wrong.
    Ended,
}

/// Whether a range that starts at `start` and has the End Location `end`
/// ends before it starts. An End Location names the last Object plus 1, and
/// with Object 0 the whole Group (§10.12.1).
pub(super) fn ends_before(start: Location, end: Location) -> bool {
    let last = if end.object == 0 {
        Location {
            group: end.group,
            object: u64::MAX,
        }
    } else {
        end
    };
    last < start
}

impl Session {
    /// Asks for a range of a track. Returns the handle of the request and the
    /// FETCH to send first on a new bidirectional stream.
    ///
    /// The request is over when it was rejected, when it was cancelled, or
    /// when both its `FETCH_OK` came and its fetch stream ended. WHY both,
    /// where §5.2 has the state kept "until ... the FETCH data stream
    /// receives a FIN or is reset": the response "can come at any time
    /// relative to object delivery" (§10.12.3), and a `FETCH_OK` that comes
    /// after the stream ended would have no request to go to. A publisher
    /// that never answers leaves the request open; this crate has no clock,
    /// so the caller cancels it when it has waited long enough.
    ///
    /// The peer finishing its direction of the request stream: before the
    /// answer, no answer can come any more, and the caller cancels the FETCH
    /// ([`Self::cancel`]); after `FETCH_OK` it ends nothing, and the caller
    /// does not cancel. A reset of the request stream by the peer ends the
    /// FETCH whenever it comes, and the caller cancels. A `STOP_SENDING` of
    /// the peer for this endpoint's direction ends nothing by itself.
    ///
    /// # Errors
    ///
    /// Fails when a parameter may not appear in FETCH, when a Standalone
    /// FETCH ends before it starts (§10.12.3), and when a Joining FETCH names
    /// a request that is not a SUBSCRIBE of this endpoint that is Pending or
    /// Established (§10.12.2).
    pub fn fetch(
        &mut self,
        range: FetchRange,
        parameters: Vec<Parameter>,
    ) -> Result<(RequestId, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::Fetch)?;
        let joined = |joining_request_id: &u64| match self.requests.get(joining_request_id) {
            Some(Request::Subscribe {
                full_track_name, ..
            }) => Ok(full_track_name.clone()),
            _ => Err(UsageError("a Joining FETCH names an open SUBSCRIBE")),
        };
        let (track, start) = match &range {
            FetchRange::Standalone {
                full_track_name,
                start,
                end,
            } => {
                if ends_before(*start, *end) {
                    return Err(UsageError("the range ends before it starts"));
                }
                (full_track_name.clone(), Some(*start))
            }
            FetchRange::RelativeJoining {
                joining_request_id, ..
            } => (joined(joining_request_id)?, None),
            FetchRange::AbsoluteJoining {
                joining_request_id,
                joining_start,
            } => (
                joined(joining_request_id)?,
                // §10.12.2.1: "the publisher sets the Start Location to
                // {Joining Start, 0}".
                Some(Location {
                    group: *joining_start,
                    object: 0,
                }),
            ),
        };
        let request = self.open(Request::Fetch(OwnFetch {
            track,
            start,
            answered: false,
            stream: StreamSeen::No,
        }))?;
        let message = ControlMessage::Fetch(Fetch {
            request_id: request.0,
            range,
            parameters,
        });
        Ok((request, message))
    }

    pub(super) fn on_fetch_ok(
        &mut self,
        request: RequestId,
        ok: FetchOk,
    ) -> Result<RequestEvent, Error> {
        let Some(Request::Fetch(fetch)) = self.requests.get_mut(&request.0) else {
            return Err(Error::ProtocolViolation(
                "message not allowed in this request state",
            ));
        };
        // §10.13: "If End Location is smaller than the Start Location in the
        // corresponding FETCH the receiver MUST close the session with a
        // PROTOCOL_VIOLATION."
        if fetch
            .start
            .is_some_and(|start| ends_before(start, ok.end_location))
        {
            return Err(Error::ProtocolViolation(
                "FETCH_OK ends before the FETCH starts",
            ));
        }
        if let Some(refusal) = track_properties_refusal(&ok.track_properties) {
            self.end_request(request);
            return Ok(RequestEvent::FetchNotProcessed { refusal });
        }
        fetch.answered = true;
        if fetch.is_over() {
            self.end_request(request);
        }
        Ok(RequestEvent::FetchEstablished {
            end_of_track: ok.end_of_track,
            end_location: ok.end_location,
            track_properties: ok.track_properties,
        })
    }

    /// What the Request ID of a `FETCH_HEADER` refers to (§11.4.4). The
    /// stream may come before the `FETCH_OK` (§10.13).
    ///
    /// WHY any request of this endpoint that is over counts as a FETCH that
    /// ended, and no list of the requests that ended is kept: a list has an
    /// end, and what falls off it reads as a FETCH that was never sent, which
    /// closes the session. In a relay the session is the publisher's, and
    /// one subscriber that sends a FETCH and cancels it, over and over, could push another
    /// subscriber's cancelled FETCH off the list while its stream is on the
    /// way, and so end the session every subscriber of that publisher uses.
    /// The price: a second stream for a FETCH that is over, and a fetch
    /// stream for a SUBSCRIBE that is over, are stopped and not found out.
    ///
    /// # Errors
    ///
    /// A protocol violation for an ID this endpoint never gave a request,
    /// for the ID of an open request that is no FETCH, and for a second
    /// stream of an open FETCH (§10.12.3: "The Objects in the response are
    /// delivered on a single unidirectional stream"). The draft names no
    /// code for any of them.
    pub fn on_fetch_stream(&mut self, request_id: u64) -> Result<FetchStreamOpened, Error> {
        let handle = RequestId(request_id, self.tag);
        let issued = self.was_issued(handle);
        match self.requests.get_mut(&request_id) {
            Some(Request::Fetch(fetch)) if fetch.stream == StreamSeen::No => {
                fetch.stream = StreamSeen::Open;
                Ok(FetchStreamOpened::Fetch(handle))
            }
            Some(Request::Fetch(_)) => Err(Error::ProtocolViolation(
                "a second fetch stream for one FETCH",
            )),
            None if issued => Ok(FetchStreamOpened::Ended),
            // An open request that is no FETCH, or an ID that was never
            // given to a request.
            Some(_) | None => Err(NO_FETCH_FOR_THE_STREAM),
        }
    }

    /// Records that the fetch stream of `request` ended, by FIN or by reset.
    /// Does nothing for a request that is over or is no FETCH.
    pub fn on_fetch_stream_closed(&mut self, request: RequestId) {
        if request.1 != self.tag {
            return;
        }
        let Some(Request::Fetch(fetch)) = self.requests.get_mut(&request.0) else {
            return;
        };
        fetch.stream = StreamSeen::Ended;
        if fetch.is_over() {
            self.end_request(request);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvp::KeyValuePair;
    use crate::message::{ReasonPhrase, RequestError};
    use crate::name::{FullTrackName, TrackNamespace};
    use crate::session::{RequestState, Role, Transport};
    use crate::{Error, RequestFault};

    fn client() -> Session {
        Session::new(Role::Client, Transport::WebTransport)
    }

    fn track() -> FullTrackName {
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        FullTrackName::new(namespace, b"a".to_vec()).unwrap()
    }

    fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    fn standalone(start: Location, end: Location) -> FetchRange {
        FetchRange::Standalone {
            full_track_name: track(),
            start,
            end,
        }
    }

    fn fetch_ok(end_location: Location, track_properties: Vec<KeyValuePair>) -> ControlMessage {
        ControlMessage::FetchOk(FetchOk {
            end_of_track: false,
            end_location,
            parameters: vec![],
            track_properties,
        })
    }

    fn established(end_location: Location) -> RequestEvent {
        RequestEvent::FetchEstablished {
            end_of_track: false,
            end_location,
            track_properties: vec![],
        }
    }

    fn request_error() -> RequestError {
        RequestError {
            error_code: 0x11,
            retry_interval: 0,
            error_reason: ReasonPhrase::default(),
            redirect: None,
        }
    }

    /// A FETCH for {1, 0} to {3, 0}, which gets Request ID 0.
    fn fetching() -> (Session, RequestId) {
        let mut session = client();
        let (request, _) = session
            .fetch(standalone(at(1, 0), at(3, 0)), vec![])
            .unwrap();
        (session, request)
    }

    /// The rows of `fixtures/own-requests/events.tsv`, which `@xmoq/moqt`
    /// reads as well: what comes from the peer for a request of this end,
    /// a client, on the request's stream or as a fetch stream, and what the
    /// session makes of it. §10.13: "If End Location is smaller than the
    /// Start Location in the corresponding FETCH the receiver MUST close the
    /// session with a `PROTOCOL_VIOLATION`", an End Location naming the last
    /// Object plus one, and {Group, 0} the whole Group. §2.5.1 and §2.4.2: a
    /// Mandatory Track Property in `FETCH_OK` ends the FETCH, as one in
    /// `SUBSCRIBE_OK` ends the subscription. The draft has no case of a fetch
    /// stream for no open FETCH (§5.2, §10.12.3): one for a request that is
    /// over is stopped, as §5.2 has Objects come for a FETCH that ended; any
    /// other is a violation, as is a `FETCH_OK` on a stream that carries no
    /// FETCH.
    #[test]
    fn what_comes_for_a_request_of_this_end_is_taken_as_the_shared_table_says() {
        let rows = crate::test_util::table_rows::<5>("own-requests/events.tsv");
        // A table that lost rows would pass.
        assert_eq!(rows.len(), 11);
        for [name, request, at_where, hex, verdict] in rows {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect();
            let mut session = client();
            // Request ID 0, the client's first.
            let handle = match request.as_str() {
                "fetch" | "cancelled-fetch" => {
                    session
                        .fetch(standalone(at(4, 2), at(9, 0)), vec![])
                        .unwrap()
                        .0
                }
                "subscribe" => session.subscribe(track(), vec![]).unwrap().0,
                other => panic!("{name}: {other}"),
            };
            if request == "cancelled-fetch" {
                session.cancel(handle);
            }
            let closed = |error: &Error| {
                let code = error.termination_code().expect("it closes the session");
                format!("closed:{:#x}", code.code())
            };
            let got = match at_where.as_str() {
                "request-stream" => {
                    let message = match ControlMessage::decode(&bytes) {
                        Ok(Some((message, _))) => message,
                        other => panic!("{name}: {other:?}"),
                    };
                    match session.on_request_message(handle, message) {
                        Ok(RequestEvent::FetchNotProcessed { .. }) => "over".to_string(),
                        Ok(_) => "taken".to_string(),
                        Err(RequestFault::Peer(error)) => closed(&error),
                        Err(RequestFault::Usage(error)) => panic!("{name}: {error}"),
                    }
                }
                "fetch-stream" | "two-fetch-streams" => {
                    // FETCH_HEADER: the type 0x05 and a Request ID of one byte.
                    let request_id = u64::from(bytes[1]);
                    if at_where == "two-fetch-streams" {
                        session.on_fetch_stream(request_id).unwrap();
                    }
                    match session.on_fetch_stream(request_id) {
                        Ok(FetchStreamOpened::Fetch(_)) => "taken".to_string(),
                        Ok(FetchStreamOpened::Ended) => "stopped".to_string(),
                        Err(error) => closed(&error),
                    }
                }
                other => panic!("{name}: {other}"),
            };
            assert_eq!(got, verdict, "{name}");
        }
    }

    #[test]
    fn a_fetch_carries_what_it_was_opened_with() {
        let mut session = client();
        let range = standalone(at(1, 2), at(3, 0));
        let parameters = vec![Parameter::SubscriberPriority(7)];
        let (request, message) = session.fetch(range.clone(), parameters.clone()).unwrap();
        assert_eq!(request.value(), 0);
        assert_eq!(
            message,
            ControlMessage::Fetch(Fetch {
                request_id: 0,
                range,
                parameters,
            })
        );
        assert_eq!(session.request_state(request), Some(RequestState::Pending));
    }

    #[test]
    fn a_range_ends_before_it_starts_by_the_last_object_it_names() {
        // (Start Location, End Location, ends before it starts).
        let cases = [
            (at(1, 0), at(1, 0), false),
            // The whole of Group 1, which Object 5 of it is in.
            (at(1, 5), at(1, 0), false),
            (at(1, 5), at(1, 5), false),
            (at(1, 5), at(1, 4), true),
            (at(2, 0), at(1, 0), true),
            (at(2, 0), at(1, 9), true),
            (at(0, 0), at(0, 1), false),
        ];
        for (start, end, expected) in cases {
            assert_eq!(ends_before(start, end), expected, "{start:?} to {end:?}");
        }
    }

    #[test]
    fn refuses_to_send_a_fetch_that_cannot_be_served() {
        let mut session = client();
        let (subscribe, _) = session.subscribe(track(), vec![]).unwrap();
        let (publish, _) = session.publish(track(), 0, vec![], vec![]).unwrap();
        let joining = |joining_request_id| FetchRange::RelativeJoining {
            joining_request_id,
            joining_start: 0,
        };
        let absolute = |joining_request_id| FetchRange::AbsoluteJoining {
            joining_request_id,
            joining_start: 0,
        };
        let not_a_subscribe = UsageError("a Joining FETCH names an open SUBSCRIBE");
        let cases = [
            (
                standalone(at(2, 0), at(1, 0)),
                vec![],
                UsageError("the range ends before it starts"),
            ),
            (
                standalone(at(0, 0), at(1, 0)),
                vec![Parameter::Forward(true)],
                UsageError("parameter not allowed in this message"),
            ),
            // A PUBLISH of this endpoint, and an ID it has not used.
            (joining(publish.value()), vec![], not_a_subscribe),
            (joining(40), vec![], not_a_subscribe),
            (absolute(41), vec![], not_a_subscribe),
        ];
        for (range, parameters, error) in cases {
            assert_eq!(
                session.fetch(range.clone(), parameters).map(|_| ()),
                Err(error),
                "{range:?}"
            );
        }
        // The SUBSCRIBE without an answer is one a FETCH may join (§10.12.2:
        // "in the Established or Pending (subscriber) state").
        assert!(session.fetch(joining(subscribe.value()), vec![]).is_ok());
        // Once it is cancelled it is not.
        session.cancel(subscribe);
        assert!(session.fetch(joining(subscribe.value()), vec![]).is_err());
    }

    /// One thing that happens to a FETCH, and what it has to lead to.
    #[derive(Clone, Copy)]
    enum Step {
        Answer(Location),
        Rejected,
        Stream,
        StreamClosed,
        Cancel,
        State(Option<RequestState>),
        /// A fetch stream arrives and is one of a FETCH that is over.
        StreamOfAnEnded,
        /// A fetch stream arrives and closes the session.
        StreamRefused(&'static str),
        /// A `FETCH_OK` arrives and closes the session.
        AnswerRefused(&'static str),
    }

    #[test]
    fn fetch_transitions() {
        use Step::{
            Answer, AnswerRefused, Cancel, Rejected, State, Stream, StreamClosed, StreamOfAnEnded,
            StreamRefused,
        };
        let pending = State(Some(RequestState::Pending));
        let live = State(Some(RequestState::Established));
        let over = State(Some(RequestState::Terminated));
        let second = "a second fetch stream for one FETCH";
        let cases: Vec<(&str, Vec<Step>)> = vec![
            (
                "the answer, then the stream",
                vec![Answer(at(3, 0)), live, Stream, live, StreamClosed, over],
            ),
            (
                "the stream, then the answer",
                vec![Stream, pending, Answer(at(3, 0)), live, StreamClosed, over],
            ),
            (
                "the stream ends before the answer comes",
                vec![Stream, StreamClosed, pending, Answer(at(3, 0)), over],
            ),
            (
                "no answer: the request stays until it is cancelled",
                vec![Stream, StreamClosed, pending, Cancel, over],
            ),
            (
                "rejected, and the stream that was on its way",
                vec![Rejected, over, StreamOfAnEnded],
            ),
            (
                "rejected after the stream came",
                vec![Stream, Rejected, over, StreamOfAnEnded],
            ),
            (
                "cancelled, and the stream that was on its way",
                vec![Answer(at(3, 0)), Cancel, over, StreamOfAnEnded],
            ),
            (
                "two streams for one FETCH",
                vec![Stream, StreamRefused(second)],
            ),
            (
                "a stream after the FETCH was served is not found out",
                vec![Answer(at(3, 0)), Stream, StreamClosed, StreamOfAnEnded],
            ),
            (
                "an answer that ends before the start",
                vec![AnswerRefused("FETCH_OK ends before the FETCH starts")],
            ),
            (
                "two answers",
                vec![
                    Answer(at(3, 0)),
                    AnswerRefused("message not allowed in this request state"),
                ],
            ),
            (
                "an answer after the FETCH was served",
                vec![
                    Answer(at(3, 0)),
                    Stream,
                    StreamClosed,
                    AnswerRefused("message after the request ended"),
                ],
            ),
        ];
        for (name, steps) in cases {
            run_steps(name, steps);
        }
    }

    fn run_steps(name: &str, steps: Vec<Step>) {
        use Step::{
            Answer, AnswerRefused, Cancel, Rejected, State, Stream, StreamClosed, StreamOfAnEnded,
            StreamRefused,
        };
        let (mut session, request) = fetching();
        for (index, step) in steps.into_iter().enumerate() {
            let context = format!("{name}, step {index}");
            let violation = |what| Err(Error::ProtocolViolation(what));
            match step {
                Answer(end) => assert_eq!(
                    session.on_request_message(request, fetch_ok(end, vec![])),
                    Ok(established(end)),
                    "{context}"
                ),
                Rejected => assert_eq!(
                    session
                        .on_request_message(request, ControlMessage::RequestError(request_error())),
                    Ok(RequestEvent::Rejected(request_error())),
                    "{context}"
                ),
                Stream => assert_eq!(
                    session.on_fetch_stream(0),
                    Ok(FetchStreamOpened::Fetch(request)),
                    "{context}"
                ),
                StreamClosed => session.on_fetch_stream_closed(request),
                Cancel => session.cancel(request),
                State(state) => assert_eq!(session.request_state(request), state, "{context}"),
                StreamOfAnEnded => assert_eq!(
                    session.on_fetch_stream(0),
                    Ok(FetchStreamOpened::Ended),
                    "{context}"
                ),
                StreamRefused(what) => {
                    assert_eq!(session.on_fetch_stream(0), violation(what), "{context}");
                }
                AnswerRefused(what) => assert_eq!(
                    // {0, 5}: before the start {1, 0} of the FETCH.
                    session.on_request_message(request, fetch_ok(at(0, 5), vec![])),
                    Err(RequestFault::Peer(Error::ProtocolViolation(what))),
                    "{context}"
                ),
            }
        }
    }

    #[test]
    fn the_end_of_a_fetch_ok_is_compared_with_the_start_this_endpoint_knows() {
        let mut session = client();
        let (subscribe, _) = session.subscribe(track(), vec![]).unwrap();
        // (the range; the End Location of the FETCH_OK; whether it is taken).
        let cases = [
            (standalone(at(4, 2), at(9, 0)), at(4, 2), true),
            (standalone(at(4, 2), at(9, 0)), at(4, 0), true),
            (standalone(at(4, 2), at(9, 0)), at(4, 1), false),
            // {Joining Start, 0} is the start (§10.12.2.1).
            (
                FetchRange::AbsoluteJoining {
                    joining_request_id: subscribe.value(),
                    joining_start: 4,
                },
                at(3, 7),
                false,
            ),
            // The start depends on what the publisher holds.
            (
                FetchRange::RelativeJoining {
                    joining_request_id: subscribe.value(),
                    joining_start: 4,
                },
                at(0, 1),
                true,
            ),
        ];
        for (range, end, taken) in cases {
            let (request, _) = session.fetch(range.clone(), vec![]).unwrap();
            let result = session.on_request_message(request, fetch_ok(end, vec![]));
            assert_eq!(result.is_ok(), taken, "{range:?} answered with {end:?}");
        }
    }

    #[test]
    fn a_fetch_ok_with_a_mandatory_track_property_ends_the_fetch() {
        let (mut session, request) = fetching();
        // 0x4000 is in the range of Mandatory Track Properties (§2.5.1).
        let properties = vec![KeyValuePair::varint(0x4000, 1).unwrap()];
        assert_eq!(
            session.on_request_message(request, fetch_ok(at(3, 0), properties)),
            Ok(RequestEvent::FetchNotProcessed {
                refusal: crate::immutable::TrackRefusal::Unsupported(0x4000)
            })
        );
        assert_eq!(
            session.request_state(request),
            Some(RequestState::Terminated)
        );
        // The stream the publisher opened is one to stop.
        assert_eq!(session.on_fetch_stream(0), Ok(FetchStreamOpened::Ended));
    }

    #[test]
    fn a_fetch_stream_needs_a_request_this_endpoint_made() {
        let mut session = client();
        let (subscribe, _) = session.subscribe(track(), vec![]).unwrap();
        let (ended, _) = session.publish(track(), 0, vec![], vec![]).unwrap();
        session.cancel(ended);
        // An open SUBSCRIBE, an ID this endpoint has not used yet, and an ID
        // of the peer's parity.
        for request_id in [subscribe.value(), 4, 1] {
            assert_eq!(
                session.on_fetch_stream(request_id),
                Err(NO_FETCH_FOR_THE_STREAM),
                "{request_id}"
            );
        }
        // A request that is over, here a PUBLISH: the stream is one to stop.
        assert_eq!(
            session.on_fetch_stream(ended.value()),
            Ok(FetchStreamOpened::Ended)
        );
    }

    #[test]
    fn a_stream_for_a_fetch_that_ended_long_ago_is_stopped_like_any_other() {
        let mut session = client();
        let mut requests = Vec::new();
        for _ in 0..1000 {
            let (request, _) = session
                .fetch(standalone(at(0, 0), at(1, 0)), vec![])
                .unwrap();
            session.cancel(request);
            requests.push(request);
        }
        for request in [requests[0], requests[999]] {
            assert_eq!(
                session.on_fetch_stream(request.value()),
                Ok(FetchStreamOpened::Ended)
            );
        }
    }

    #[test]
    fn the_end_of_a_stream_is_taken_for_an_open_fetch_of_this_session_only() {
        let (mut session, fetch) = fetching();
        let (subscribe, _) = session.subscribe(track(), vec![]).unwrap();
        let (mut other, of_another_session) = fetching();
        other.cancel(of_another_session);
        let (ended, _) = session
            .fetch(standalone(at(0, 0), at(1, 0)), vec![])
            .unwrap();
        session.cancel(ended);

        // A request that is no FETCH, one that is over, and the handle of
        // another session, which has the Request ID of the open FETCH: none
        // of them changes anything.
        for request in [subscribe, ended, of_another_session] {
            session.on_fetch_stream_closed(request);
        }
        assert_eq!(session.request_state(fetch), Some(RequestState::Pending));
        assert_eq!(
            session.request_state(subscribe),
            Some(RequestState::Pending)
        );
        // The stream of the open FETCH is still to come.
        assert_eq!(
            session.on_fetch_stream(fetch.value()),
            Ok(FetchStreamOpened::Fetch(fetch))
        );
    }
}
