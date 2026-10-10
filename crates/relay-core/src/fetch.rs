//! FETCH through the relay (§9.4, §10.12): a Standalone FETCH of a
//! subscriber is asked of one publisher, and the publisher's fetch stream is
//! passed on as it is.
//!
//! The relay has no cache, so it holds no Object a FETCH could be answered
//! with, and it does not look into a fetch stream. What it decides is whom
//! to ask, what the subscriber is told on its request stream, and when the
//! whole thing is cut short; moving the bytes is the I/O side's
//! ([`Action::PipeFetch`]).

use xmoq_moqt::fetch::{FetchHeader, FetchStreamEncoder, RangeEnd};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{ControlMessage, Fetch, FetchRange, RequestOk, request_error_code};
use xmoq_moqt::name::{FullTrackName, Location};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{FetchStreamOpened, PeerRequestId, RequestEvent, RequestId};
use xmoq_moqt::{RequestFault, stream_reset_code};

use crate::events::{Event, FetchPipeEnd};
use crate::routing::{MAX_OWN_REQUESTS_PER_SESSION, OwnRequestKind, Refusal};
use crate::{
    Action, OwnRequestEnd, OwnRequestKey, Relay, RequestSlot, SessionKey, StreamKey, internal_error,
};

/// FETCH requests one session may have open at once. One more is answered
/// with `EXCESSIVE_LOAD`.
///
/// Provisional: the draft sets no limit, and `XmoQ` has not decided one.
pub(crate) const MAX_FETCHES_PER_SESSION: usize = 16;

/// FETCH requests the relay may have open on one publisher's session at once.
/// One more is answered with `EXCESSIVE_LOAD`.
///
/// WHY a number of its own below [`MAX_OWN_REQUESTS_PER_SESSION`]: the
/// publisher's fetch stream stays open for as long as the subscriber takes
/// to read it, and each such stream is one of the unidirectional streams the
/// publisher may have open towards the relay (64). Slow subscribers with
/// many FETCH requests would otherwise use up all of them, and the publisher could
/// open no subgroup stream for anybody. Provisional.
pub(crate) const MAX_OWN_FETCHES_PER_SESSION: usize = 16;

/// How long a publisher may send nothing on a fetch stream while the
/// subscriber can take more, when the FETCH has no `FILL_TIMEOUT`
/// (§10.2.5: "the relay waits for an implementation specific duration").
/// Provisional.
pub(crate) const FETCH_UPSTREAM_WAIT_MS: u64 = 5_000;

/// The longest the relay waits that way, whatever `FILL_TIMEOUT` says
/// (§10.2.5: "the relay MAY use a shorter timeout without informing the
/// subscriber"). Provisional.
pub(crate) const MAX_FETCH_UPSTREAM_WAIT_MS: u64 = 30_000;

/// How long a subscriber may take nothing of a fetch stream before the relay
/// stops passing it on. Provisional.
pub(crate) const FETCH_DOWNSTREAM_WAIT_MS: u64 = 10_000;

/// `REQUEST_UPDATE`s for a FETCH that may wait for the answer to the FETCH,
/// each of which is owed a `REQUEST_OK` after it. One more and the FETCH is
/// refused with `EXCESSIVE_LOAD`, after which none of them gets an answer.
///
/// WHY a limit: the answers are sent in one go when the publisher's comes,
/// and how many there are is the subscriber's to choose. Provisional.
pub(crate) const MAX_HELD_FETCH_UPDATES: u32 = 64;

/// How far the passing on of the publisher's fetch stream has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pipe {
    /// The publisher has opened no fetch stream yet.
    NotOpen,
    Running,
    /// Passed on to its FIN.
    Done,
}

/// A FETCH of a subscriber that the relay asked a publisher for.
#[derive(Debug)]
pub(crate) struct RelayedFetch {
    request: PeerRequestId,
    track: FullTrackName,
    publisher: SessionKey,
    own: OwnRequestKey,
    /// Whether the subscriber has its `FETCH_OK`.
    answered: bool,
    pipe: Pipe,
    upstream_wait_ms: u64,
    /// `REQUEST_UPDATE`s that wait for the answer.
    held_updates: u32,
}

/// The relay's FETCH to a publisher, as the publisher's session holds it.
#[derive(Debug)]
pub(crate) struct OwnFetch {
    id: RequestId,
    /// The session and request stream of the subscriber's FETCH.
    subscriber: (SessionKey, StreamKey),
}

/// The `FILL_TIMEOUT` of a FETCH, in milliseconds (§10.2.5).
fn fill_timeout(parameters: &[Parameter]) -> Option<u64> {
    parameters.iter().find_map(|parameter| match parameter {
        Parameter::FillTimeout(millis) => Some(*millis),
        _ => None,
    })
}

/// What of a subscriber's FETCH goes into the relay's own.
///
/// WHY `GROUP_ORDER`, where §7.3 has "Relays SHOULD NOT directly use
/// Subscriber Priority or Group Order from incoming subscriptions for
/// upstream subscriptions": that is said of subscriptions, whose Objects the
/// relay sends in an order of its own choosing. A fetch stream is passed on
/// as it comes, and "A publisher MUST send fetched groups in the requested
/// group order" (§10.12.3) is kept only if the publisher was asked for that
/// order.
///
/// WHY `FILL_TIMEOUT`: it says how long the subscriber waits, which a relay
/// further upstream can use and an original publisher ignores (§10.2.1:
/// "relays can consider received parameter values"). It is passed on as it
/// came; that relay shortens it as it sees fit.
///
/// WHY NOT `SUBSCRIBER_PRIORITY`: §7.3, and nothing here sends by priority.
/// WHY NOT `AUTHORIZATION_TOKEN`: what a subscriber's token means to a
/// publisher is undecided, as for SUBSCRIBE.
fn upstream_parameters(parameters: &[Parameter]) -> Vec<Parameter> {
    parameters
        .iter()
        .filter(|parameter| {
            matches!(
                parameter,
                Parameter::GroupOrder(_) | Parameter::FillTimeout(_)
            )
        })
        .cloned()
        .collect()
}

/// The last Location an End Location covers: an End Location names the last
/// Object plus 1, and with Object 0 the whole Group (§10.12.1).
fn last_location(end: Location) -> Location {
    Location {
        group: end.group,
        object: end.object.checked_sub(1).unwrap_or(u64::MAX),
    }
}

impl Relay {
    /// Takes the events that are about a FETCH the relay passes on: the ones
    /// of a fetch stream, and what comes back on the relay's own FETCH.
    /// Returns every other event.
    pub(crate) fn on_fetch_event(
        &mut self,
        event: Event,
        actions: &mut Vec<Action>,
    ) -> Option<Event> {
        match event {
            Event::FetchStreamOpened {
                session,
                stream,
                request_id,
            } => self.on_fetch_stream_opened(session, stream, request_id, actions),
            Event::FetchPipeEnded {
                session,
                stream,
                how,
            } => self.on_fetch_pipe_ended(session, stream, how, actions),
            Event::FetchStreamWritten { session, stream } => {
                self.on_fetch_stream_written(session, stream);
            }
            Event::OwnRequestMessage {
                session,
                request,
                message,
            } if self.is_own_fetch(session, request) => {
                self.on_own_fetch_message(session, request, message, actions);
            }
            Event::OwnRequestClosed {
                session,
                request,
                how,
            } if self.is_own_fetch(session, request) => {
                self.on_own_fetch_closed(session, request, how, actions);
            }
            Event::UpstreamTimeout { session, request } if self.is_own_fetch(session, request) => {
                self.on_fetch_timeout(session, request, actions);
            }
            other => return Some(other),
        }
        None
    }

    /// A subscriber sent FETCH.
    pub(crate) fn on_fetch(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        fetch: &Fetch,
        actions: &mut Vec<Action>,
    ) {
        let refuse = |relay: &mut Self, error_code, actions: &mut Vec<Action>| {
            relay.refuse_fetch(
                session,
                stream,
                request,
                &Refusal::code(error_code),
                actions,
            );
        };
        // FIXME: serve a Joining FETCH (§10.12.2).
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = &fetch.range
        else {
            return refuse(self, request_error_code::NOT_SUPPORTED, actions);
        };
        let open = self
            .fetches
            .keys()
            .chain(&self.written_fetches)
            .filter(|(subscriber, _)| *subscriber == session)
            .count();
        if open >= MAX_FETCHES_PER_SESSION {
            return refuse(self, request_error_code::EXCESSIVE_LOAD, actions);
        }
        let fill_timeout = fill_timeout(&fetch.parameters);
        if fill_timeout == Some(0) {
            let (name, range) = (full_track_name.clone(), (*start, *end));
            return self.answer_without_asking(session, stream, request, &name, range, actions);
        }
        let Some(publisher) = self.whom_to_ask(full_track_name, session) else {
            return refuse(self, request_error_code::DOES_NOT_EXIST, actions);
        };
        let Some(state) = self.sessions.get_mut(&publisher) else {
            return refuse(self, request_error_code::DOES_NOT_EXIST, actions);
        };
        // The relay's FETCH and SUBSCRIBE requests share the publisher's
        // session, and its patience.
        if state.own_requests() >= MAX_OWN_REQUESTS_PER_SESSION
            || state.own_fetches.len() >= MAX_OWN_FETCHES_PER_SESSION
        {
            return refuse(self, request_error_code::EXCESSIVE_LOAD, actions);
        }
        let asked = state
            .moqt
            .fetch(fetch.range.clone(), upstream_parameters(&fetch.parameters));
        let Ok((id, message)) = asked else {
            // The range and the parameters passed the session's checks when
            // they came in.
            return refuse(self, request_error_code::INTERNAL_ERROR, actions);
        };
        let own = OwnRequestKey(self.next_own_request);
        self.next_own_request = self.next_own_request.saturating_add(1);
        state.own_fetches.insert(
            own,
            OwnFetch {
                id,
                subscriber: (session, stream),
            },
        );
        actions.push(Action::OpenRequest {
            session: publisher,
            request: own,
            message,
        });
        actions.push(Action::StartUpstreamTimer {
            session: publisher,
            request: own,
        });
        self.fetches.insert(
            (session, stream),
            RelayedFetch {
                request,
                track: full_track_name.clone(),
                publisher,
                own,
                answered: false,
                pipe: Pipe::NotOpen,
                upstream_wait_ms: fill_timeout
                    .unwrap_or(FETCH_UPSTREAM_WAIT_MS)
                    .min(MAX_FETCH_UPSTREAM_WAIT_MS),
                held_updates: 0,
            },
        );
        if let Some(state) = self.sessions.get_mut(&session) {
            state.requests.insert(
                stream,
                RequestSlot::Fetch {
                    request,
                    peer_finished: false,
                },
            );
        }
    }

    /// The one publisher a FETCH for the track is sent to (§9.5: "the Relay
    /// MAY send the FETCH to any of them"): the first source of the track
    /// that is not on its way out, and without one the first publisher that
    /// announced a matching namespace.
    ///
    /// WHY NOT ask another when that one refuses: the relay cannot tell a
    /// refusal that another publisher would not repeat from one it would,
    /// and a FETCH that goes the round of four publishers holds the
    /// subscriber for four times the wait.
    fn whom_to_ask(&self, track: &FullTrackName, subscriber: SessionKey) -> Option<SessionKey> {
        let source = self.tracks.live(track).and_then(|live| {
            live.sources()
                .filter(|source| !source.draining)
                .map(|source| source.session_and_alias().0)
                .find(|session| *session != subscriber)
        });
        source.or_else(|| {
            self.matching_publishers(track)
                .find(|session| *session != subscriber)
        })
    }

    /// A FETCH with `FILL_TIMEOUT` 0 (§10.2.5): "the relay MUST NOT wait for
    /// upstream delivery and MUST report any unavailable Objects as Unknown
    /// gaps". Without a cache every Object of the range is unavailable, so
    /// the answer is a fetch stream with one End of Unknown Range indicator.
    ///
    /// The `FETCH_OK` needs an End Location, which the relay knows only for
    /// a track it has a source of, from the largest Location it has seen.
    /// WHY `TIMEOUT` where it does not: "the `FETCH_OK` MUST NOT be sent
    /// until the End Location is known" (§10.13), learning it means asking
    /// the publisher and waiting for the answer, and the subscriber said it
    /// waits for nothing. WHY NOT ask anyway, reading "upstream delivery" as
    /// the delivery of Objects only: waiting for the answer is waiting.
    fn answer_without_asking(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        track: &FullTrackName,
        (start, end): (Location, Location),
        actions: &mut Vec<Action>,
    ) {
        let largest = self.tracks.live(track).and_then(|live| live.largest);
        let refusal = match largest {
            // §10.12.3: "If ... Start Location is greater than the Largest
            // Object ... the publisher MUST return REQUEST_ERROR with error
            // code INVALID_RANGE."
            Some(largest) if start > largest => Some(request_error_code::INVALID_RANGE),
            Some(_) => None,
            // A source that has said nothing of a largest Location and sent
            // no Object has published none, as far as the relay knows
            // (§10.2.11: "If omitted from a message, the sending endpoint has
            // not published or received any Objects in the Track"), and
            // §10.12.3 has `INVALID_RANGE` for that.
            None if self.tracks.live(track).is_some() => Some(request_error_code::INVALID_RANGE),
            None if self.whom_to_ask(track, session).is_none() => {
                Some(request_error_code::DOES_NOT_EXIST)
            }
            // A publisher of the namespace, of which the relay knows nothing
            // about the track.
            None => Some(request_error_code::TIMEOUT),
        };
        let (Some(largest), None) = (largest, refusal) else {
            let error_code = refusal.unwrap_or(request_error_code::INTERNAL_ERROR);
            self.refuse_fetch(
                session,
                stream,
                request,
                &Refusal::code(error_code),
                actions,
            );
            return;
        };
        // §10.13: "If the requested FETCH End Location was beyond the Largest
        // known (possibly final) Object, End Location is {Largest.Group,
        // Largest.Object + 1}".
        let end = if last_location(end) > largest {
            Location {
                group: largest.group,
                object: largest.object.saturating_add(1),
            }
        } else {
            end
        };
        // §9.6: "Relays MUST forward all Track Properties ... in the
        // responses to requests for that Track".
        let properties = self
            .tracks
            .live(track)
            .map(|live| live.properties.clone())
            .unwrap_or_default();
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let request_id = request.value();
        let (ok, owed) = match state.moqt.accept_fetch(request, false, end, properties) {
            Ok(accepted) => accepted,
            Err(error) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
                return;
            }
        };
        state
            .requests
            .insert(stream, RequestSlot::Finished(Some(request)));
        // An indicator is a Location, written the same in either Group
        // Order. WHY the last Location of the range also for a Descending
        // FETCH, which begins there: §11.4.4.2 has an indicator cover what
        // lies "between the last serialized Object, if any, and this
        // Location", and does not say from where when no Object came before
        // and the Groups run downwards. A subscriber may read this one as
        // covering nothing, or everything.
        // The header is taken off: the I/O side writes it.
        let mut encoder =
            FetchStreamEncoder::new(FetchHeader { request_id }, GroupOrder::Ascending);
        encoder.take_bytes();
        encoder.end_of_range(RangeEnd::Unknown, last_location(end));
        actions.push(Action::SendRequest {
            session,
            stream,
            message: ok,
        });
        acknowledge_updates(session, stream, owed, actions);
        actions.push(Action::WriteFetchStream {
            session,
            stream,
            request_id,
            body: encoder.take_bytes(),
            wait_ms: FETCH_DOWNSTREAM_WAIT_MS,
        });
        actions.push(Action::FinishRequest { session, stream });
        // Counted as one of the session's FETCHes until its stream is
        // written, or given up.
        self.written_fetches.insert((session, stream));
    }

    /// The I/O side is done with a fetch stream the relay made itself.
    fn on_fetch_stream_written(&mut self, session: SessionKey, stream: StreamKey) {
        self.written_fetches.remove(&(session, stream));
    }

    /// The track was found malformed, by what a subscription to it brought
    /// (§2.4.2): "If a relay detects a Malformed Track, it MUST immediately
    /// terminate downstream subscriptions with `PUBLISH_DONE` and reset any
    /// fetch streams with Status Code `MALFORMED_TRACK`."
    pub(crate) fn fetches_of_a_malformed_track(
        &mut self,
        track: &FullTrackName,
        actions: &mut Vec<Action>,
    ) {
        let mut of_the_track: Vec<(SessionKey, StreamKey)> = self
            .fetches
            .iter()
            .filter(|(_, fetch)| fetch.track == *track)
            .map(|(subscriber, _)| *subscriber)
            .collect();
        of_the_track.sort_unstable_by_key(|(session, stream)| (session.0, stream.0));
        let malformed = Refusal::code(request_error_code::MALFORMED_TRACK);
        for subscriber in of_the_track {
            self.fail_fetch(
                subscriber,
                &malformed,
                stream_reset_code::MALFORMED_TRACK,
                actions,
            );
        }
    }

    /// Answers a FETCH with `REQUEST_ERROR` and a FIN. The session keeps a
    /// rejected FETCH until its stream is gone, for the `REQUEST_UPDATE`
    /// that can still be on its way; the slot tells it when that is.
    pub(crate) fn refuse_fetch(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        refusal: &Refusal,
        actions: &mut Vec<Action>,
    ) {
        self.refuse(session, stream, request, refusal, actions);
        if let Some(state) = self.sessions.get_mut(&session) {
            state
                .requests
                .insert(stream, RequestSlot::Finished(Some(request)));
        }
    }

    /// Whether a request of the relay on `session` is a FETCH.
    fn is_own_fetch(&self, session: SessionKey, own: OwnRequestKey) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|state| state.own_fetches.contains_key(&own))
    }

    /// Forgets a relayed FETCH on the publisher's side: the relay's request
    /// is cancelled, and a fetch stream that is not passed on yet is stopped
    /// when it comes.
    fn cancel_upstream(&mut self, fetch: &RelayedFetch, actions: &mut Vec<Action>) {
        let (session, request) = (fetch.publisher, fetch.own);
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        if let Some(own) = state.own_fetches.remove(&request) {
            state.moqt.cancel(own.id);
            actions.push(Action::CancelOwnRequest {
                session,
                request,
                code: stream_reset_code::CANCELLED,
            });
        }
    }

    /// Ends a relayed FETCH that did not come to its end: the subscriber is
    /// refused if it has no answer yet, and what is passed on is cut with
    /// `pipe_code`.
    fn fail_fetch(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        refusal: &Refusal,
        pipe_code: u32,
        actions: &mut Vec<Action>,
    ) {
        let Some(fetch) = self.fetches.remove(&subscriber) else {
            return;
        };
        let (session, stream) = subscriber;
        if fetch.pipe == Pipe::Running {
            actions.push(Action::AbortFetchPipe {
                session,
                stream,
                code: pipe_code,
            });
        }
        self.cancel_upstream(&fetch, actions);
        match (fetch.answered, fetch.pipe) {
            (false, _) => self.refuse_fetch(session, stream, fetch.request, refusal, actions),
            // WHY a reset of the request stream: the subscriber has its
            // FETCH_OK and waits for a fetch stream, which it is owed also
            // for a range without Objects (§10.12.3: "the publisher opens
            // the unidirectional stream, sends the FETCH_HEADER ... and
            // closes the stream with a FIN"). No stream will come, and a FIN
            // on the request stream would not say so.
            (true, Pipe::NotOpen) => {
                if let Some(state) = self.sessions.get_mut(&session) {
                    state.requests.remove(&stream);
                    state.moqt.on_peer_request_closed(fetch.request);
                }
                actions.push(Action::ResetRequest {
                    session,
                    stream,
                    code: pipe_code,
                });
            }
            // The reset of the fetch stream says what happened; nothing more
            // is to be said on the request stream.
            (true, Pipe::Running | Pipe::Done) => {
                self.finish_fetch_request(session, stream, fetch.request, actions);
            }
        }
    }

    /// Ends the relay's direction of an answered FETCH's request stream.
    fn finish_fetch_request(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        if state.peer_finished(stream) {
            state.requests.remove(&stream);
            state.moqt.on_peer_request_closed(request);
        } else {
            state
                .requests
                .insert(stream, RequestSlot::Finished(Some(request)));
        }
        actions.push(Action::FinishRequest { session, stream });
    }

    /// The FETCH is served when the subscriber has its `FETCH_OK` and the
    /// fetch stream was passed on to its FIN.
    fn finish_if_served(&mut self, subscriber: (SessionKey, StreamKey), actions: &mut Vec<Action>) {
        let served = self
            .fetches
            .get(&subscriber)
            .is_some_and(|fetch| fetch.answered && fetch.pipe == Pipe::Done);
        if !served {
            return;
        }
        let Some(fetch) = self.fetches.remove(&subscriber) else {
            return;
        };
        if let Some(state) = self.sessions.get_mut(&fetch.publisher)
            && state.own_fetches.remove(&fetch.own).is_some()
        {
            // The session let go of the request when both had come.
            actions.push(Action::FinishOwnRequest {
                session: fetch.publisher,
                request: fetch.own,
            });
        }
        self.finish_fetch_request(subscriber.0, subscriber.1, fetch.request, actions);
    }

    /// A message on the request stream of the relay's FETCH.
    fn on_own_fetch_message(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(own_fetch) = state.own_fetches.get(&own) else {
            return;
        };
        let subscriber = own_fetch.subscriber;
        let event = match state.moqt.on_request_message(own_fetch.id, message) {
            Ok(event) => event,
            Err(RequestFault::Peer(error)) => {
                self.fail_session(session, &error, actions);
                return;
            }
            Err(RequestFault::Usage(error)) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
                return;
            }
        };
        match event {
            RequestEvent::FetchEstablished {
                end_of_track,
                end_location,
                track_properties,
            } => {
                // WHY the timer goes on where no fetch stream has opened yet:
                // the answer promises one, and a publisher that never opens
                // it would leave the subscriber waiting with its FETCH_OK for
                // as long as the two sessions live. The same time again, from
                // the answer. With the stream open or passed on, the timer
                // is not running.
                let waits_for_the_stream = self
                    .fetches
                    .get(&subscriber)
                    .is_some_and(|fetch| fetch.pipe == Pipe::NotOpen);
                actions.push(if waits_for_the_stream {
                    Action::StartUpstreamTimer {
                        session,
                        request: own,
                    }
                } else {
                    Action::StopUpstreamTimer {
                        session,
                        request: own,
                    }
                });
                self.answer_fetch(
                    subscriber,
                    end_of_track,
                    end_location,
                    track_properties,
                    actions,
                );
            }
            RequestEvent::Rejected(error) => {
                actions.push(Action::StopUpstreamTimer {
                    session,
                    request: own,
                });
                // The session has let go of the request.
                state_forgets(self, session, own);
                actions.push(Action::FinishOwnRequest {
                    session,
                    request: own,
                });
                // §10.12.3: "If the upstream FETCH fails, the relay sends a
                // REQUEST_ERROR and can reset the unidirectional stream."
                let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
                self.fail_fetch(
                    subscriber,
                    &Refusal::passed_on(&error, OwnRequestKind::Fetch),
                    unknown,
                    actions,
                );
            }
            // §2.5.1: a Mandatory Track Property the relay does not
            // understand. The session ended the request; its stream is
            // cancelled.
            RequestEvent::UnsupportedFetch { .. } => {
                actions.push(Action::StopUpstreamTimer {
                    session,
                    request: own,
                });
                state_forgets(self, session, own);
                actions.push(Action::CancelOwnRequest {
                    session,
                    request: own,
                    code: stream_reset_code::CANCELLED,
                });
                let unsupported = Refusal::code(request_error_code::UNSUPPORTED_EXTENSION);
                let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
                self.fail_fetch(subscriber, &unsupported, unknown, actions);
            }
            // Nothing else answers a FETCH; the session has dealt with it.
            _ => {}
        }
    }

    /// The publisher said `FETCH_OK`: the subscriber gets one with the same
    /// End Location and Track Properties (§9.6).
    fn answer_fetch(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        end_of_track: bool,
        end_location: Location,
        track_properties: Vec<KeyValuePair>,
        actions: &mut Vec<Action>,
    ) {
        let (session, stream) = subscriber;
        let Some(fetch) = self.fetches.get_mut(&subscriber) else {
            return;
        };
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let accepted =
            state
                .moqt
                .accept_fetch(fetch.request, end_of_track, end_location, track_properties);
        let (ok, owed) = match accepted {
            Ok(accepted) => accepted,
            // The publisher's session took this End Location for the same
            // range, so it is not before the Start Location here either.
            Err(error) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
                return;
            }
        };
        fetch.answered = true;
        actions.push(Action::SendRequest {
            session,
            stream,
            message: ok,
        });
        acknowledge_updates(session, stream, owed, actions);
        self.finish_if_served(subscriber, actions);
    }

    /// The request stream of the relay's FETCH ended, or never came about.
    fn on_own_fetch_closed(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        how: OwnRequestEnd,
        actions: &mut Vec<Action>,
    ) {
        if how == OwnRequestEnd::Stopped {
            return;
        }
        let Some(subscriber) = self.subscriber_of(session, own) else {
            return;
        };
        let answered = self
            .fetches
            .get(&subscriber)
            .is_some_and(|fetch| fetch.answered);
        // After its FETCH_OK a publisher may finish its direction; how the
        // FETCH went on shows on the fetch stream.
        if answered && how == OwnRequestEnd::Fin {
            return;
        }
        let gone = Refusal::code(request_error_code::DOES_NOT_EXIST);
        let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
        self.fail_fetch(subscriber, &gone, unknown, actions);
    }

    /// The publisher did not answer the relay's FETCH in time, or answered
    /// and did not open its fetch stream in time.
    fn on_fetch_timeout(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(subscriber) = self.subscriber_of(session, own) else {
            return;
        };
        // §10.6.2, TIMEOUT.
        let too_late = Refusal::code(request_error_code::TIMEOUT);
        let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
        self.fail_fetch(subscriber, &too_late, unknown, actions);
    }

    fn subscriber_of(
        &self,
        session: SessionKey,
        own: OwnRequestKey,
    ) -> Option<(SessionKey, StreamKey)> {
        let own_fetch = self.sessions.get(&session)?.own_fetches.get(&own)?;
        Some(own_fetch.subscriber)
    }

    /// The `FETCH_HEADER` of a stream of a publisher arrived.
    fn on_fetch_stream_opened(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request_id: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let stop = Action::StopDataStream {
            session,
            stream,
            code: stream_reset_code::CANCELLED,
        };
        let id = match state.moqt.on_fetch_stream(request_id) {
            Ok(FetchStreamOpened::Fetch(id)) => id,
            // Of a FETCH the relay cancelled: the publisher opened the stream
            // before it knew (§5.2).
            Ok(FetchStreamOpened::Ended) => {
                actions.push(stop);
                return;
            }
            Err(error) => {
                self.fail_session(session, &error, actions);
                return;
            }
        };
        let own = state
            .own_fetches
            .iter()
            .find(|(_, own_fetch)| own_fetch.id == id)
            .map(|(own, own_fetch)| (*own, own_fetch.subscriber));
        let Some((own, subscriber)) = own else {
            actions.push(stop);
            return;
        };
        let Some(fetch) = self.fetches.get_mut(&subscriber) else {
            actions.push(stop);
            return;
        };
        fetch.pipe = Pipe::Running;
        // WHY the wait for the answer ends here: a publisher may send Objects
        // before its FETCH_OK (§10.13), and holds the FETCH_OK back until it
        // knows the End Location. From here on what is watched is that the
        // stream goes on; the wait for the answer starts anew when the
        // stream has ended.
        actions.push(Action::StopUpstreamTimer {
            session,
            request: own,
        });
        actions.push(Action::PipeFetch {
            from: session,
            from_stream: stream,
            session: subscriber.0,
            stream: subscriber.1,
            request_id: fetch.request.value(),
            upstream_wait_ms: fetch.upstream_wait_ms,
            downstream_wait_ms: FETCH_DOWNSTREAM_WAIT_MS,
        });
    }

    /// The passing on of a fetch stream ended by itself.
    fn on_fetch_pipe_ended(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        how: FetchPipeEnd,
        actions: &mut Vec<Action>,
    ) {
        let subscriber = (session, stream);
        let Some(fetch) = self.fetches.get_mut(&subscriber) else {
            return;
        };
        if fetch.pipe != Pipe::Running {
            return;
        }
        // The stream is over for the publisher's session too.
        let (publisher, own, answered) = (fetch.publisher, fetch.own, fetch.answered);
        fetch.pipe = Pipe::Done;
        if let Some(state) = self.sessions.get_mut(&publisher)
            && let Some(own_fetch) = state.own_fetches.get(&own)
        {
            state.moqt.on_fetch_stream_closed(own_fetch.id);
        }
        // The I/O side has ended both streams. `Done` keeps `fail_fetch`
        // from asking for that again.
        let refusal = match how {
            FetchPipeEnd::Finished => {
                if !answered {
                    // The stream came whole and the FETCH_OK is still out.
                    actions.push(Action::StartUpstreamTimer {
                        session: publisher,
                        request: own,
                    });
                }
                self.finish_if_served(subscriber, actions);
                return;
            }
            // §10.12.3, on a publisher that cannot say what comes next.
            FetchPipeEnd::UpstreamAborted(_) | FetchPipeEnd::UpstreamLost => {
                request_error_code::INTERNAL_ERROR
            }
            FetchPipeEnd::UpstreamStalled => request_error_code::TIMEOUT,
            // The subscriber gave the FETCH up, or is gone: §5.2 has the
            // publisher of a FETCH reset the request stream for the first.
            FetchPipeEnd::DownstreamStopped(_)
            | FetchPipeEnd::DownstreamStalled
            | FetchPipeEnd::DownstreamLost => {
                let code = if how == FetchPipeEnd::DownstreamStalled {
                    stream_reset_code::TOO_FAR_BEHIND
                } else {
                    stream_reset_code::CANCELLED
                };
                self.drop_fetch(subscriber, code, actions);
                return;
            }
        };
        let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
        self.fail_fetch(subscriber, &Refusal::code(refusal), unknown, actions);
    }

    /// Lets go of a FETCH whose subscriber does not want it any more, or
    /// cannot take it: the relay's FETCH is cancelled, what is passed on is
    /// cut, and the subscriber's request stream is reset with `code` (§5.2:
    /// "It MUST reset the bidi request stream and unidirectional data stream
    /// associated with the FETCH").
    fn drop_fetch(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        code: u32,
        actions: &mut Vec<Action>,
    ) {
        let Some(fetch) = self.fetches.remove(&subscriber) else {
            return;
        };
        let (session, stream) = subscriber;
        if fetch.pipe == Pipe::Running {
            actions.push(Action::AbortFetchPipe {
                session,
                stream,
                code,
            });
        }
        self.cancel_upstream(&fetch, actions);
        if let Some(state) = self.sessions.get_mut(&session) {
            state.requests.remove(&stream);
            state.moqt.on_peer_request_closed(fetch.request);
            actions.push(Action::ResetRequest {
                session,
                stream,
                code,
            });
        }
    }

    /// A `REQUEST_UPDATE` came for a FETCH that has no answer yet.
    pub(crate) fn on_fetch_update_held(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let subscriber = (session, stream);
        let Some(fetch) = self.fetches.get_mut(&subscriber) else {
            return;
        };
        fetch.held_updates = fetch.held_updates.saturating_add(1);
        if fetch.held_updates > MAX_HELD_FETCH_UPDATES {
            let too_many = Refusal::code(request_error_code::EXCESSIVE_LOAD);
            let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
            self.fail_fetch(subscriber, &too_many, unknown, actions);
        }
    }

    /// The subscriber cancelled its FETCH: its request stream was reset or
    /// stopped (§5.2).
    pub(crate) fn on_fetch_cancelled(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        self.drop_fetch((session, stream), stream_reset_code::CANCELLED, actions);
    }

    /// What a session that is gone leaves of relayed FETCH requests.
    pub(crate) fn drop_fetches_of(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        self.written_fetches
            .retain(|(subscriber, _)| *subscriber != session);
        // By key, so that the same events lead to the same actions.
        let mut as_subscriber: Vec<(SessionKey, StreamKey)> = self
            .fetches
            .keys()
            .filter(|(subscriber, _)| *subscriber == session)
            .copied()
            .collect();
        as_subscriber.sort_unstable_by_key(|(_, stream)| stream.0);
        for subscriber in as_subscriber {
            let Some(fetch) = self.fetches.remove(&subscriber) else {
                continue;
            };
            if fetch.pipe == Pipe::Running {
                actions.push(Action::AbortFetchPipe {
                    session: subscriber.0,
                    stream: subscriber.1,
                    code: stream_reset_code::SESSION_CLOSED,
                });
            }
            self.cancel_upstream(&fetch, actions);
        }
        let mut as_publisher: Vec<(SessionKey, StreamKey)> = self
            .fetches
            .iter()
            .filter(|(_, fetch)| fetch.publisher == session)
            .map(|(subscriber, _)| *subscriber)
            .collect();
        as_publisher.sort_unstable_by_key(|(subscriber, stream)| (subscriber.0, stream.0));
        // As for a SUBSCRIBE whose publisher left before it answered.
        let gone = Refusal::code(request_error_code::DOES_NOT_EXIST);
        for subscriber in as_publisher {
            self.fail_fetch(
                subscriber,
                &gone,
                stream_reset_code::UNKNOWN_OBJECT_STATUS,
                actions,
            );
        }
    }

    /// FETCH requests of subscribers the relay has asked a publisher for.
    pub(crate) fn fetch_count(&self) -> usize {
        self.fetches.len() + self.written_fetches.len()
    }
}

/// Removes the relay's FETCH from the publisher's session state, after the
/// session itself let go of the request.
fn state_forgets(relay: &mut Relay, session: SessionKey, own: OwnRequestKey) {
    if let Some(state) = relay.sessions.get_mut(&session) {
        state.own_fetches.remove(&own);
    }
}

/// The `REQUEST_OK` messages owed to `REQUEST_UPDATE`s that came before the
/// FETCH was answered.
fn acknowledge_updates(
    session: SessionKey,
    stream: StreamKey,
    owed: u64,
    actions: &mut Vec<Action>,
) {
    for _ in 0..owed {
        actions.push(Action::SendRequest {
            session,
            stream,
            message: ControlMessage::RequestOk(RequestOk::default()),
        });
    }
}

/// What the reset of a publisher's fetch stream becomes for the subscriber
/// whose stream it is passed on to: the Stream Reset Error Code to reset
/// that one with (§3.3.3).
///
/// WHY not the publisher's code in every case: as for a `REQUEST_ERROR`
/// ([`Refusal::passed_on`]), a code speaks about the two endpoints of the
/// stream it ends. What says something about the Objects, or says nothing,
/// is passed on. What is about the publisher's session with the relay is
/// not the subscriber's business: that the publisher is going becomes
/// `UNKNOWN_OBJECT_STATUS`, which is what it means for the rest of the
/// range, and the others, with any code unknown here and a reset the
/// transport gave no code for, become `INTERNAL_ERROR`.
#[must_use]
pub fn fetch_reset_passed_on(code: Option<u32>) -> u32 {
    use stream_reset_code as reset;
    match code {
        Some(
            passed @ (reset::INTERNAL_ERROR
            | reset::CANCELLED
            | reset::DELIVERY_TIMEOUT
            | reset::UNKNOWN_OBJECT_STATUS
            | reset::EXCESSIVE_LOAD
            | reset::MALFORMED_TRACK),
        ) => passed,
        Some(reset::SESSION_CLOSED | reset::GOING_AWAY) => reset::UNKNOWN_OBJECT_STATUS,
        // TOO_FAR_BEHIND and EXPIRED_AUTH_TOKEN are about the relay as the
        // publisher's subscriber.
        _ => reset::INTERNAL_ERROR,
    }
}
