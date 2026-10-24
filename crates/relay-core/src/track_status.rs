//! `TRACK_STATUS` through the relay (§4: "Relays MUST implement all MOQT
//! messages defined in this document"; §10.14). Decided on 2026-10-24: a
//! track that has a source is answered from what the relay knows of it, as
//! its `SUBSCRIBE_OK` would be; one that has none is asked of the publishers
//! that announced a namespace of it, one after another, as a FETCH is
//! (`fetch.rs`); with nobody to ask, `DOES_NOT_EXIST`.

use std::collections::HashMap;

use xmoq_moqt::message::{ControlMessage, Subscribe, request_error_code};
use xmoq_moqt::name::FullTrackName;
use xmoq_moqt::params::{ANSWER_WAIT_MS, Parameter, answer_wait_ms};
use xmoq_moqt::session::{PeerRequestId, RequestEvent, RequestId};
use xmoq_moqt::{RequestFault, stream_reset_code};

use crate::routing::OwnRequestKind;
use crate::{
    Action, Event, OwnRequestEnd, OwnRequestKey, Refusal, Relay, RequestSlot, SessionKey, StreamKey,
};

/// A `TRACK_STATUS` the relay sent a publisher for a subscriber's.
#[derive(Debug)]
pub(crate) struct OwnTrackStatus {
    id: RequestId,
    /// The session and request stream of the subscriber's `TRACK_STATUS`.
    pub(crate) subscriber: (SessionKey, StreamKey),
}

/// A subscriber's `TRACK_STATUS` that waits for a publisher's answer.
#[derive(Debug)]
pub(crate) struct Relayed {
    request: PeerRequestId,
    track: FullTrackName,
    publisher: SessionKey,
    own: OwnRequestKey,
    /// The publishers asked so far.
    asked: Vec<SessionKey>,
    /// What is left of the time the subscriber waits for its answer, less
    /// each publisher's wait that ran out, as for a FETCH (`fetch.rs`,
    /// `answer_left_ms`).
    answer_left_ms: u64,
}

/// The subscribers' `TRACK_STATUS` requests that wait for a publisher, by
/// their session and request stream.
#[derive(Debug, Default)]
pub(crate) struct TrackStatuses(HashMap<(SessionKey, StreamKey), Relayed>);

impl Relay {
    /// A subscriber's `TRACK_STATUS`.
    ///
    /// §10.14: "If successful, the publisher responds with a `TRACK_STATUS_OK`
    /// with the same parameters and Track Properties it would have set in a
    /// `SUBSCRIBE_OK`": `LARGEST_OBJECT` where the relay knows a Location of
    /// the track (§10.2.11), and the Track Properties of its source (§9.6).
    /// "Relays without an Established subscription MAY forward `TRACK_STATUS`
    /// to one or more publishers": to one at a time.
    ///
    /// WHY `DOES_NOT_EXIST` at once where nobody is to be asked, and not a
    /// wait for a publisher as for a SUBSCRIBE with `RENDEZVOUS_TIMEOUT`: the
    /// draft has that parameter "in a SUBSCRIBE message" alone (§10.2.6), a
    /// wait would be the "downstream subscription state" a `TRACK_STATUS` does
    /// not create, and what is asked is how the track stands now.
    pub(crate) fn on_track_status(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        asked: &Subscribe,
        actions: &mut Vec<Action>,
    ) {
        let track = &asked.full_track_name;
        if let Some(live) = self.tracks.live(track)
            && live.sources().any(|source| !source.draining)
        {
            let parameters = live
                .largest
                .map(Parameter::LargestObject)
                .into_iter()
                .collect();
            let track_properties = live.properties.clone();
            return self.answer_track_status(
                session,
                stream,
                request,
                (parameters, track_properties),
                actions,
            );
        }
        let mut asked_of = Vec::new();
        match self.ask_track_status_in_turn((session, stream), track, &mut asked_of, actions) {
            Ok((publisher, own)) => {
                self.track_statuses.0.insert(
                    (session, stream),
                    Relayed {
                        request,
                        track: track.clone(),
                        publisher,
                        own,
                        asked: asked_of,
                        // §10.2.2: AUTHORIZATION_TOKEN is its one parameter,
                        // which adds nothing to the wait.
                        answer_left_ms: answer_wait_ms(ANSWER_WAIT_MS, &asked.parameters),
                    },
                );
                let slot = RequestSlot::TrackStatus {
                    request,
                    peer_finished: false,
                };
                self.put_slot(session, stream, slot, actions);
            }
            Err(code) => {
                let refusal = Refusal::code(code.unwrap_or(request_error_code::DOES_NOT_EXIST));
                self.refuse(session, stream, request, &refusal, actions);
            }
        }
    }

    /// `TRACK_STATUS_OK` and a FIN (§10.14: "The bidi stream is closed with
    /// a FIN after `TRACK_STATUS_OK` or `REQUEST_ERROR` are sent").
    fn answer_track_status(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        (parameters, track_properties): (Vec<Parameter>, Vec<xmoq_moqt::kvp::KeyValuePair>),
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let answer = state
            .moqt
            .accept_track_status(request, parameters, track_properties);
        self.respond(session, stream, answer, actions);
        actions.push(Action::FinishRequest { session, stream });
        self.finish_slot(session, stream, Some(request), actions);
    }

    /// Asks the publishers that announced a namespace of the track, in the
    /// order of their announcements, those in `asked` left out, until one can
    /// be sent the `TRACK_STATUS`. As `Relay::ask_in_turn` of `fetch.rs`.
    fn ask_track_status_in_turn(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        track: &FullTrackName,
        asked: &mut Vec<SessionKey>,
        actions: &mut Vec<Action>,
    ) -> Result<(SessionKey, OwnRequestKey), Option<u64>> {
        let mut refused = None;
        loop {
            let left =
                |publisher: &SessionKey| *publisher != subscriber.0 && !asked.contains(publisher);
            let Some(publisher) = self.matching_publishers(track).find(left) else {
                if asked.is_empty() {
                    // Refused all the same (known gap), as a FETCH is.
                    self.waits_for_upstream(track, actions);
                }
                return Err(refused);
            };
            asked.push(publisher);
            match self.ask_track_status_of(subscriber, publisher, track, actions) {
                Ok(own) => return Ok((publisher, own)),
                Err(code) => refused = Some(code),
            }
        }
    }

    /// Sends one publisher the relay's `TRACK_STATUS`, or says with which
    /// code the relay refuses to. It counts among the requests the relay has
    /// open on the publisher for the subscriber's session
    /// (`MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER`), as the relay's SUBSCRIBEs do.
    /// WHY NOT the subscriber's parameters: "Message Parameters are always
    /// intended for the peer endpoint only and are not forwarded by Relays"
    /// (§10.2.1).
    fn ask_track_status_of(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        publisher: SessionKey,
        track: &FullTrackName,
        actions: &mut Vec<Action>,
    ) -> Result<OwnRequestKey, u64> {
        let for_this_session = self
            .asked_for(subscriber.0)
            .get(&publisher)
            .copied()
            .unwrap_or(0);
        let for_one = self.requests_for_one;
        let Some(state) = self.sessions.get_mut(&publisher) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        // §10.4: no new request to a peer that sent GOAWAY.
        if state.moqt.peer_going_away() {
            return Err(request_error_code::DOES_NOT_EXIST);
        }
        if state.own_requests() >= crate::MAX_OWN_REQUESTS_PER_SESSION
            || for_this_session >= for_one
        {
            return Err(request_error_code::EXCESSIVE_LOAD);
        }
        let Ok((id, message)) = state.moqt.track_status(track.clone(), Vec::new()) else {
            return Err(request_error_code::INTERNAL_ERROR);
        };
        let own = OwnRequestKey(self.next_own_request);
        self.next_own_request = self.next_own_request.saturating_add(1);
        state
            .own_track_statuses
            .insert(own, OwnTrackStatus { id, subscriber });
        actions.push(Action::OpenRequest {
            session: publisher,
            request: own,
            message,
        });
        actions.push(Action::StartUpstreamTimer {
            session: publisher,
            request: own,
        });
        Ok(own)
    }

    fn is_own_track_status(&self, session: SessionKey, own: OwnRequestKey) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|state| state.own_track_statuses.contains_key(&own))
    }

    /// Takes the events about the relay's own `TRACK_STATUS` requests.
    /// Returns every other event.
    pub(crate) fn on_track_status_event(
        &mut self,
        event: Event,
        actions: &mut Vec<Action>,
    ) -> Option<Event> {
        match event {
            Event::OwnRequestMessage {
                session,
                request,
                message,
            } if self.is_own_track_status(session, request) => {
                self.on_own_track_status_message(session, request, message, actions);
            }
            Event::OwnRequestClosed {
                session,
                request,
                how,
            } if self.is_own_track_status(session, request) => {
                // What the stream of an answered request does is of no
                // matter: the relay let go of it with the answer.
                if how != OwnRequestEnd::Stopped {
                    let gone = Refusal::code(request_error_code::DOES_NOT_EXIST);
                    self.ask_the_next_track_status_publisher(session, request, &gone, actions);
                }
            }
            Event::UpstreamTimeout { session, request }
                if self.is_own_track_status(session, request) =>
            {
                self.on_track_status_timeout(session, request, actions);
            }
            other => return Some(other),
        }
        None
    }

    fn on_own_track_status_message(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(own_status) = state.own_track_statuses.get(&own) else {
            return;
        };
        let (id, subscriber) = (own_status.id, own_status.subscriber);
        let event = match state.moqt.on_request_message(id, message) {
            Ok(event) => event,
            Err(RequestFault::Peer(error)) => return self.fail_session(session, &error, actions),
            Err(RequestFault::Usage(error)) => {
                actions.push(crate::internal_error(session, &error));
                return self.drop_session(session, actions);
            }
        };
        match event {
            RequestEvent::TrackStatusAnswered {
                parameters,
                track_properties,
            } => {
                self.let_go_of_own_track_status(session, own, actions);
                actions.push(Action::FinishOwnRequest {
                    session,
                    request: own,
                });
                let Some(relayed) = self.track_statuses.0.remove(&subscriber) else {
                    return;
                };
                // The answer is the publisher's: its LARGEST_OBJECT and its
                // Track Properties (§9.6).
                let parameters = parameters
                    .into_iter()
                    .filter(|parameter| matches!(parameter, Parameter::LargestObject(_)))
                    .collect();
                let (session, stream) = subscriber;
                self.answer_track_status(
                    session,
                    stream,
                    relayed.request,
                    (parameters, track_properties),
                    actions,
                );
            }
            RequestEvent::Rejected(error) => {
                // The session ended the request with the answer.
                self.let_go_of_own_track_status(session, own, actions);
                actions.push(Action::FinishOwnRequest {
                    session,
                    request: own,
                });
                let refusal = Refusal::passed_on(&error, OwnRequestKind::Subscribe);
                self.ask_again_or_refuse(subscriber, &refusal, actions);
            }
            // Nothing else answers a TRACK_STATUS; the session has dealt
            // with it. A GOAWAY on its stream changes nothing: one answer is
            // all that is waited for.
            _ => {}
        }
    }

    /// The publisher's answer did not come in time. The next one is asked
    /// while the subscriber still waits, as for a FETCH (`fetch.rs`,
    /// `Relay::on_fetch_timeout`, which has why).
    fn on_track_status_timeout(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(subscriber) = self
            .sessions
            .get(&session)
            .and_then(|state| state.own_track_statuses.get(&own))
            .map(|own_status| own_status.subscriber)
        else {
            return;
        };
        let wait_ms = self.upstream_answer_wait_ms;
        let left = self.track_statuses.0.get_mut(&subscriber).map(|relayed| {
            relayed.answer_left_ms = relayed.answer_left_ms.saturating_sub(wait_ms);
            relayed.answer_left_ms
        });
        let too_late = Refusal::code(request_error_code::TIMEOUT);
        if left.is_some_and(|left| left > 0) {
            return self.ask_the_next_track_status_publisher(session, own, &too_late, actions);
        }
        self.cancel_own_track_status(session, own, actions);
        if let Some(relayed) = self.track_statuses.0.remove(&subscriber) {
            let (session, stream) = subscriber;
            self.refuse(session, stream, relayed.request, &too_late, actions);
        }
    }

    /// The publisher the `TRACK_STATUS` was asked of refused it, went, or did
    /// not answer in time: the next one is asked, or the subscriber is
    /// refused with `refusal` where nobody is left.
    fn ask_the_next_track_status_publisher(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        refusal: &Refusal,
        actions: &mut Vec<Action>,
    ) {
        let Some(subscriber) = self
            .sessions
            .get(&session)
            .and_then(|state| state.own_track_statuses.get(&own))
            .map(|own_status| own_status.subscriber)
        else {
            return;
        };
        self.cancel_own_track_status(session, own, actions);
        self.ask_again_or_refuse(subscriber, refusal, actions);
    }

    fn ask_again_or_refuse(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        refusal: &Refusal,
        actions: &mut Vec<Action>,
    ) {
        let Some(mut relayed) = self.track_statuses.0.remove(&subscriber) else {
            return;
        };
        let track = relayed.track.clone();
        match self.ask_track_status_in_turn(subscriber, &track, &mut relayed.asked, actions) {
            Ok((publisher, own)) => {
                relayed.publisher = publisher;
                relayed.own = own;
                self.track_statuses.0.insert(subscriber, relayed);
            }
            Err(code) => {
                let (session, stream) = subscriber;
                match code {
                    Some(code) => {
                        self.refuse(
                            session,
                            stream,
                            relayed.request,
                            &Refusal::code(code),
                            actions,
                        );
                    }
                    None => self.refuse(session, stream, relayed.request, refusal, actions),
                }
            }
        }
    }

    /// Lets go of the relay's `TRACK_STATUS` on the publisher: its timer is
    /// stopped, and the session forgets it. Where it is still open, it is
    /// cancelled.
    fn cancel_own_track_status(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        if let Some(own_status) = state.own_track_statuses.remove(&own) {
            state.moqt.cancel(own_status.id);
            actions.push(Action::StopUpstreamTimer {
                session,
                request: own,
            });
            actions.push(Action::CancelOwnRequest {
                session,
                request: own,
                code: stream_reset_code::CANCELLED,
            });
        }
    }

    /// The relay's `TRACK_STATUS` was answered: its timer is stopped, and the
    /// session has let go of it.
    fn let_go_of_own_track_status(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        if let Some(state) = self.sessions.get_mut(&session) {
            state.own_track_statuses.remove(&own);
        }
        actions.push(Action::StopUpstreamTimer {
            session,
            request: own,
        });
    }

    /// The subscriber gave its `TRACK_STATUS` up before it was answered:
    /// the relay's is cancelled.
    pub(crate) fn on_track_status_cancelled(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        if let Some(relayed) = self.track_statuses.0.remove(&(session, stream)) {
            self.cancel_own_track_status(relayed.publisher, relayed.own, actions);
        }
    }

    /// A session went: its `TRACK_STATUS` requests are let go of, and the
    /// ones it was asked for go to the next publisher.
    pub(crate) fn drop_track_statuses_of(
        &mut self,
        session: SessionKey,
        actions: &mut Vec<Action>,
    ) {
        let mut as_subscriber: Vec<(SessionKey, StreamKey)> = self
            .track_statuses
            .0
            .keys()
            .filter(|(subscriber, _)| *subscriber == session)
            .copied()
            .collect();
        // By key, so that the same events lead to the same actions.
        as_subscriber.sort_unstable_by_key(|(_, stream)| stream.0);
        for subscriber in as_subscriber {
            if let Some(relayed) = self.track_statuses.0.remove(&subscriber) {
                self.cancel_own_track_status(relayed.publisher, relayed.own, actions);
            }
        }
        let mut as_publisher: Vec<(SessionKey, StreamKey)> = self
            .track_statuses
            .0
            .iter()
            .filter(|(_, relayed)| relayed.publisher == session)
            .map(|(subscriber, _)| *subscriber)
            .collect();
        as_publisher.sort_unstable_by_key(|(subscriber, stream)| (subscriber.0, stream.0));
        let gone = Refusal::code(request_error_code::DOES_NOT_EXIST);
        for subscriber in as_publisher {
            self.ask_again_or_refuse(subscriber, &gone, actions);
        }
    }

    /// `TRACK_STATUS` requests of subscribers that wait for a publisher.
    pub(crate) fn track_status_count(&self) -> usize {
        self.track_statuses.0.len()
    }
}
