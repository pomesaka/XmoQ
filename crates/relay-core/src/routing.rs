//! Routing by namespace (§9.5): a SUBSCRIBE for a track nobody sent a PUBLISH
//! for is passed on to a publisher that announced a matching namespace with
//! `PUBLISH_NAMESPACE`, and answered when that publisher has answered.
//!
//! FIXME: send the SUBSCRIBE to every matching publisher, and take Objects of
//! one track from several of them (§9.5: "Relays MUST send SUBSCRIBE messages
//! to all matching publishers"; §9.3: "Relays MUST handle Objects for the same
//! Track from multiple publishers"). Only the publisher whose namespace was
//! announced first is asked, and what it answers is the answer. The same goes
//! for a `PUBLISH_NAMESPACE` that arrives while a track is subscribed to
//! elsewhere (§9.5: "it MUST send a SUBSCRIBE to the publisher that sent the
//! `PUBLISH_NAMESPACE` for each matching subscription"). `docs/roadmap.md`,
//! Phase 0b, has the item.

use std::collections::{BTreeSet, HashMap};

use xmoq_moqt::message::{
    ControlMessage, ReasonPhrase, RequestError, Subscribe, request_error_code,
};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{self, Parameter};
use xmoq_moqt::session::{PeerRequestId, RequestEvent, RequestId};
use xmoq_moqt::{RequestFault, stream_reset_code};

use crate::{
    Action, OwnRequestEnd, OwnRequestKey, Relay, RequestSlot, SessionKey, StreamKey, Subscriber,
    Track, TrackEnd, TrackSource, internal_error,
};

/// Namespaces one session may have announced at once. One more is refused
/// with `EXCESSIVE_LOAD`. Provisional.
pub const MAX_NAMESPACES_PER_SESSION: usize = 64;

/// Subscribers that may wait for one upstream SUBSCRIBE to be answered. One
/// more is refused with `EXCESSIVE_LOAD`. Provisional.
pub const MAX_WAITING_SUBSCRIBERS: usize = 64;

/// Requests the relay may have open on one session at once: its SUBSCRIBEs
/// to a publisher. A subscriber whose SUBSCRIBE would need one more is
/// refused with `EXCESSIVE_LOAD`. Provisional.
///
/// A SUBSCRIBE that the publisher answered with `DUPLICATE_SUBSCRIPTION`
/// counts until the PUBLISH it announced has come or the wait for it is over:
/// subscribers still wait on it, and a publisher that answers every SUBSCRIBE
/// that way must not get more of them for it.
///
/// WHY a limit per publisher: each one costs the publisher a stream and
/// state, and subscribers choose the track names. Without it, the
/// subscribers of a relay could open as many requests on one publisher as
/// they can name tracks, and a publisher that defends itself by closing the
/// session would take every other track of that session with it. Draft 18
/// has no message by which a peer tells how many requests it takes (none was
/// found in the text), so the number is the relay's own.
pub const MAX_OWN_REQUESTS_PER_SESSION: usize = 64;

/// Bytes of an upstream Error Reason that are passed on. Provisional.
///
/// WHY cut it: the publisher chooses the text, up to 1024 bytes, and every
/// subscriber that waited gets a copy.
pub const MAX_PASSED_ON_REASON_LEN: usize = 128;

/// A namespace a session announced with `PUBLISH_NAMESPACE` and has not
/// withdrawn. The relay keeps them in the order they were announced.
#[derive(Debug)]
pub(crate) struct Announcement {
    pub(crate) session: SessionKey,
    /// The request stream of the `PUBLISH_NAMESPACE`.
    pub(crate) stream: StreamKey,
    pub(crate) namespace: TrackNamespace,
}

/// A request the relay opened on a session.
#[derive(Debug)]
pub(crate) struct OwnRequest {
    pub(crate) id: RequestId,
    pub(crate) track: FullTrackName,
    /// The publisher answered `DUPLICATE_SUBSCRIPTION`: the request is over on
    /// its stream and in the session, and what is still waited for is the
    /// publisher's PUBLISH for the track. Nothing that arrives for the request
    /// means anything any more.
    pub(crate) awaits_publish: bool,
}

/// A downstream SUBSCRIBE that waits for the upstream one to be answered.
#[derive(Debug)]
pub(crate) struct Waiter {
    session: SessionKey,
    stream: StreamKey,
    request: PeerRequestId,
    forward: bool,
}

/// A track the relay has asked a publisher for and has no answer about yet.
/// §9.4: no `SUBSCRIBE_OK` goes downstream before the upstream subscription
/// is Established.
#[derive(Debug)]
pub(crate) struct PendingTrack {
    /// The session of the publisher that was asked.
    session: SessionKey,
    request: OwnRequestKey,
    waiters: Vec<Waiter>,
}

/// What every waiter of a pending track is told when the track does not come.
struct Refusal {
    error_code: u64,
    retry_interval: u64,
    reason: ReasonPhrase,
}

impl Refusal {
    fn code(error_code: u64) -> Self {
        Self {
            error_code,
            retry_interval: 0,
            reason: ReasonPhrase::default(),
        }
    }

    /// What a publisher's `REQUEST_ERROR` becomes for the subscribers.
    ///
    /// WHY not the publisher's code in every case: a code speaks about the
    /// two endpoints of the request it answers, and the subscriber reads it as
    /// a statement about its own request to the relay. Codes that mean the
    /// same one hop on are passed on. The others are replaced: what only says
    /// that this publisher will not serve the track becomes `DOES_NOT_EXIST`,
    /// and what a publisher may not answer this SUBSCRIBE with, or a code
    /// this relay does not know, becomes `INTERNAL_ERROR`, so that no
    /// subscriber has to interpret it.
    ///
    /// The reason is passed on with the publisher's code only; under another
    /// code it would explain something the subscriber was not told. The
    /// Retry Interval is the publisher's, except under `INTERNAL_ERROR` put in
    /// place of another code: when to retry was said about something the
    /// subscriber does not learn, and 0 ("SHOULD NOT be retried", §10.6.2)
    /// promises nothing.
    fn passed_on(error: &RequestError) -> Self {
        use request_error_code as code;
        let error_code = match error.error_code {
            // About the track at the publisher or its load, or generic.
            passed @ (code::INTERNAL_ERROR
            | code::TIMEOUT
            | code::EXCESSIVE_LOAD
            | code::DOES_NOT_EXIST
            | code::UNSUPPORTED_EXTENSION
            // FIXME: decide what a subscriber gets for these three. As they
            // are, they tell a subscriber to mend its token and try again,
            // which cannot help: the relay sends no token upstream. What to
            // do instead follows from the decision on authorization
            // (`docs/open-questions.md`).
            | code::UNAUTHORIZED
            | code::MALFORMED_AUTH_TOKEN
            | code::EXPIRED_AUTH_TOKEN) => passed,
            // The publisher does not answer SUBSCRIBE (the relay does), or is
            // going away (the relay is not).
            //
            // WHY not REDIRECT itself: it carries a Redirect, which names a
            // URI and a track for the publisher's peer, the relay. §10.6.2:
            // "Relays are not required to follow redirects from upstream and
            // MAY forward" one; this relay does neither.
            //
            // DUPLICATE_SUBSCRIPTION does not get here: it is no refusal (see
            // where the publisher's answer is handled).
            code::NOT_SUPPORTED | code::GOING_AWAY | code::REDIRECT => code::DOES_NOT_EXIST,
            // Codes that do not answer this SUBSCRIBE. §10.6.2 defines
            // MALFORMED_TRACK "In response to a FETCH", and INVALID_RANGE for
            // a "specified Filter or range of Locations", of which the
            // relay's SUBSCRIBE has none. UNINTERESTED, PREFIX_OVERLAP and
            // the like answer a PUBLISH or a namespace request. The rest is
            // unknown here.
            _ => code::INTERNAL_ERROR,
        };
        let passed = error_code == error.error_code;
        Self {
            error_code,
            retry_interval: if passed || error_code == code::DOES_NOT_EXIST {
                error.retry_interval
            } else {
                0
            },
            reason: if passed {
                shortened(&error.error_reason)
            } else {
                ReasonPhrase::default()
            },
        }
    }
}

/// The first bytes of a publisher's reason, for the subscribers.
///
/// WHY not simply the first 128 bytes: §1.4.4 has a Reason Phrase be UTF-8,
/// and a cut inside a character would make the relay the sender of a broken
/// one. A reason that is not UTF-8 to begin with is not passed on at all.
fn shortened(reason: &ReasonPhrase) -> ReasonPhrase {
    let Ok(text) = std::str::from_utf8(reason.as_bytes()) else {
        return ReasonPhrase::default();
    };
    let end = (0..=MAX_PASSED_ON_REASON_LEN.min(text.len()))
        .rev()
        .find(|end| text.is_char_boundary(*end))
        .unwrap_or(0);
    // At most 128 bytes, which a Reason Phrase always takes.
    ReasonPhrase::new(text.as_bytes().get(..end).unwrap_or_default()).unwrap_or_default()
}

impl Relay {
    /// Records a namespace a session announced. `false` when the session has
    /// as many as it may.
    pub(crate) fn add_namespace(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        namespace: TrackNamespace,
    ) -> bool {
        let announced = self
            .namespaces
            .iter()
            .filter(|entry| entry.session == session)
            .count();
        if announced >= MAX_NAMESPACES_PER_SESSION {
            return false;
        }
        self.namespaces.push(Announcement {
            session,
            stream,
            namespace,
        });
        true
    }

    /// §9.5: withdrawing stops new subscriptions for the namespace. What is
    /// already subscribed or asked for stays.
    pub(crate) fn withdraw_namespace(&mut self, session: SessionKey, stream: StreamKey) {
        self.namespaces
            .retain(|entry| !(entry.session == session && entry.stream == stream));
    }

    /// A SUBSCRIBE for a track that has no Established upstream subscription.
    /// Returns the subscriber's slot when the SUBSCRIBE now waits, and `None`
    /// when it was refused.
    pub(crate) fn subscribe_through_namespace(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        subscribe: &Subscribe,
        actions: &mut Vec<Action>,
    ) -> Result<RequestSlot, u64> {
        let track = &subscribe.full_track_name;
        let waiter = Waiter {
            session,
            stream,
            request,
            forward: params::forward(&subscribe.parameters),
        };
        let waiting = RequestSlot::Waiting {
            request,
            track: track.clone(),
        };
        // §9.4 lets a relay make one upstream subscription for several
        // subscribers; the ones that come while it is not answered wait for
        // the same answer.
        if let Some(pending) = self.pending.get_mut(track) {
            if pending.waiters.len() >= MAX_WAITING_SUBSCRIBERS {
                return Err(request_error_code::EXCESSIVE_LOAD);
            }
            pending.waiters.push(waiter);
            return Ok(waiting);
        }
        // §9.5: Namespace Prefix Matching, field by field. §9.5 also has a
        // publisher announce a namespace explicitly to get subscriptions
        // routed to it, so sessions that only sent PUBLISH are not asked.
        // The first match by the order of announcing; see the FIXME at the
        // top for the others.
        let Some(publisher) = self
            .namespaces
            .iter()
            .find(|entry| entry.namespace.is_prefix_of(track.namespace()))
            .map(|entry| entry.session)
        else {
            // §10.2.6: without RENDEZVOUS_TIMEOUT, or with 0, "MUST
            // immediately return REQUEST_ERROR with error code
            // DOES_NOT_EXIST if no publisher is available".
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        let Some(state) = self.sessions.get_mut(&publisher) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        if state.own.len() >= MAX_OWN_REQUESTS_PER_SESSION {
            return Err(request_error_code::EXCESSIVE_LOAD);
        }
        // §9.2: "Relays SHOULD set the Forward flag to 1 when a new
        // subscription needs to be sent upstream", and §9.5: "MUST use
        // Forward=1 when subscribing upstream". No parameter says that:
        // §10.2.12, "If the parameter is omitted from any other message, the
        // default value is 1."
        //
        // WHY omitted and not sent as 1: the two mean the same, and moqtail-ts
        // 0.13.0 copies the parameters of a SUBSCRIBE into its SUBSCRIBE_OK,
        // where FORWARD may not appear (§10.2.12, §10.2.1): a SUBSCRIBE that
        // carries it cannot be answered by such a publisher.
        //
        // Nothing of the downstream SUBSCRIBE is copied: §7 keeps its
        // priorities out, and what a token of the subscriber means to the
        // publisher is undecided.
        let Ok((id, message)) = state.moqt.subscribe(track.clone(), vec![]) else {
            // The session already has a subscription to the track in this
            // role, which is one that PUBLISH_DONE ended and that still
            // drains. The track is over for now.
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        let own = OwnRequestKey(self.next_own_request);
        self.next_own_request = self.next_own_request.saturating_add(1);
        state.own.insert(
            own,
            OwnRequest {
                id,
                track: track.clone(),
                awaits_publish: false,
            },
        );
        self.pending.insert(
            track.clone(),
            PendingTrack {
                session: publisher,
                request: own,
                waiters: vec![waiter],
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
        Ok(waiting)
    }

    pub(crate) fn on_own_request_message(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(own) = state.own.get(&request) else {
            return;
        };
        if own.awaits_publish {
            return;
        }
        let track_name = own.track.clone();
        let event = match state.moqt.on_request_message(own.id, message) {
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
            RequestEvent::SubscribeEstablished {
                track_alias,
                parameters,
                track_properties,
            } => {
                let Some(pending) = self.pending.remove(&track_name) else {
                    return;
                };
                // The answer came, and the request goes on.
                actions.push(Action::StopUpstreamTimer { session, request });
                // §10.2.11: the relay's LARGEST_OBJECT starts from the one
                // the publisher gave. §9.6: its Track Properties are passed
                // on.
                let largest = parameters.iter().find_map(|parameter| match parameter {
                    Parameter::LargestObject(location) => Some(*location),
                    _ => None,
                });
                self.tracks.insert(
                    track_name.clone(),
                    Track {
                        source: TrackSource::Subscription {
                            session,
                            request,
                            track_alias,
                        },
                        properties: track_properties,
                        largest,
                        subscribers: Vec::new(),
                        in_streams: HashMap::new(),
                        ending: false,
                    },
                );
                for waiter in &pending.waiters {
                    self.establish(&track_name, waiter, actions);
                }
                // The publisher may have opened streams before its
                // SUBSCRIBE_OK was read.
                self.replay_early_streams(session, track_alias, actions);
            }
            RequestEvent::UnsupportedTrack { track_alias, .. } => {
                // §2.5.1: the subscription is not established, and the stream
                // is cancelled. So are the streams that came ahead of the
                // answer under its alias.
                state.own.remove(&request);
                state.drop_early_streams(session, track_alias, actions);
                actions.push(Action::CancelOwnRequest {
                    session,
                    request,
                    code: stream_reset_code::CANCELLED,
                });
                self.refuse_waiters(
                    &track_name,
                    &Refusal::code(request_error_code::UNSUPPORTED_EXTENSION),
                    actions,
                );
            }
            RequestEvent::Rejected(error) => {
                self.on_upstream_refusal(session, request, &track_name, &error, actions);
            }
            RequestEvent::SubscriptionDone(_) => {
                // The same wait for the streams the publisher counted as for a
                // track that came with PUBLISH (§10.11).
                let Some(track) = self.tracks.get_mut(&track_name) else {
                    return;
                };
                track.ending = true;
                let (_, track_alias) = track.source.session_and_alias();
                if !self.end_track_if_drained(&track_name, actions) {
                    actions.push(Action::StartDrainTimer {
                        session,
                        track_alias,
                    });
                }
            }
            // Nothing else answers a SUBSCRIBE; the session has dealt with it.
            _ => {}
        }
    }

    /// The publisher answered the relay's SUBSCRIBE with `REQUEST_ERROR`.
    fn on_upstream_refusal(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        track_name: &FullTrackName,
        error: &RequestError,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        actions.push(Action::FinishOwnRequest { session, request });
        // §5.1: "If a publisher receives a SUBSCRIBE request for a
        // Track with an existing subscription in Pending (publisher)
        // state, it MUST fail that request with a
        // DUPLICATE_SUBSCRIPTION error." In that case the publisher's
        // PUBLISH for the track is on its way; it travels on another
        // stream and may be read after this answer.
        //
        // WHY wait and not refuse: the subscribers would be turned
        // away a moment before the track they asked for is there. The
        // wait is bounded by the same time a publisher gets to answer,
        // counted anew.
        //
        // FIXME: ask again where no PUBLISH is coming. The code has a
        // second meaning (§5.1: "a second subscription to a Track
        // with the same role"): the publisher still holds a
        // subscription of the relay, such as one the relay cancelled
        // and the publisher has not let go of yet. No PUBLISH follows
        // then, and the subscribers wait in vain and get TIMEOUT.
        // WHY not ask again now: §10.6.2 leaves that to the
        // publisher's Retry Interval ("If the value is 0, the request
        // SHOULD NOT be retried"), which needs a timer of its own,
        // and the own session always sends 0.
        //
        // WHY not for NOT_SUPPORTED as well, which the own clients
        // send in this place: it says nothing about a PUBLISH, and a
        // track that is not coming would cost every subscriber the
        // whole wait.
        let asked = self
            .pending
            .get(track_name)
            .is_some_and(|pending| pending.request == request);
        if asked && error.error_code == request_error_code::DUPLICATE_SUBSCRIPTION {
            if let Some(own) = state.own.get_mut(&request) {
                own.awaits_publish = true;
            }
            actions.push(Action::StartUpstreamTimer { session, request });
            return;
        }
        state.own.remove(&request);
        self.refuse_waiters(track_name, &Refusal::passed_on(error), actions);
    }

    /// Something ended on a request stream the relay opened.
    ///
    /// WHY a `STOP_SENDING` changes nothing: it closes only the direction the
    /// relay sends in, and a SUBSCRIBE says nothing more on it anyway. The
    /// publisher's answer, its `PUBLISH_DONE` and the Objects travel the other
    /// way, and the request ends when that direction does (§3.3.2 cancels by
    /// ending every direction that is still open). The clients read it the
    /// same way.
    ///
    /// The other three end the request. Before an answer: no publisher has the
    /// track, whether the request never left, or the publisher ended its
    /// direction without answering. After `SUBSCRIBE_OK` and without
    /// `PUBLISH_DONE`: the publisher is gone from the track, since
    /// `PUBLISH_DONE` travels on that direction and can no longer come.
    pub(crate) fn on_own_request_closed(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        how: OwnRequestEnd,
        actions: &mut Vec<Action>,
    ) {
        if how == OwnRequestEnd::Stopped {
            return;
        }
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(own) = state.own.get(&request) else {
            return;
        };
        // The publisher closes the stream after its REQUEST_ERROR (§3.3.2).
        if own.awaits_publish {
            return;
        }
        let track_name = own.track.clone();
        let asked = self
            .pending
            .get(&track_name)
            .is_some_and(|pending| pending.request == request);
        if asked {
            state.moqt.cancel(own.id);
            state.own.remove(&request);
            actions.push(Action::CancelOwnRequest {
                session,
                request,
                code: stream_reset_code::CANCELLED,
            });
            self.refuse_waiters(
                &track_name,
                &Refusal::code(request_error_code::DOES_NOT_EXIST),
                actions,
            );
            return;
        }
        let sourced = self
            .tracks
            .get(&track_name)
            .is_some_and(|track| track.source.is_subscription(session, request));
        if !sourced {
            return;
        }
        let ending = self
            .tracks
            .get(&track_name)
            .is_some_and(|track| track.ending);
        // After PUBLISH_DONE the publisher closes the stream (§10.11) and the
        // track goes on draining.
        if !ending {
            self.end_track(&track_name, TrackEnd::SourceGone, actions);
        }
    }

    /// The publisher did not answer the relay's SUBSCRIBE in time, or the
    /// PUBLISH it announced did not come in time.
    pub(crate) fn on_upstream_timeout(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(own) = state.own.get(&request) else {
            return;
        };
        let track_name = own.track.clone();
        let asked = self
            .pending
            .get(&track_name)
            .is_some_and(|pending| pending.request == request);
        if !asked {
            return;
        }
        // The timer has run out, so only the request is left to let go of.
        if let Some(own) = state.own.remove(&request)
            && !own.awaits_publish
        {
            state.moqt.cancel(own.id);
            actions.push(Action::CancelOwnRequest {
                session,
                request,
                code: stream_reset_code::CANCELLED,
            });
        }
        // §10.6.2, TIMEOUT: "a relay could not establish an upstream
        // subscription within the timeout".
        self.refuse_waiters(
            &track_name,
            &Refusal::code(request_error_code::TIMEOUT),
            actions,
        );
    }

    /// Takes a subscriber that waited off its pending track. When it was the
    /// last one, the upstream SUBSCRIBE is cancelled: nobody wants the answer.
    pub(crate) fn waiter_left(
        &mut self,
        track_name: &FullTrackName,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(pending) = self.pending.get_mut(track_name) else {
            return;
        };
        pending
            .waiters
            .retain(|waiter| !(waiter.session == session && waiter.stream == stream));
        if !pending.waiters.is_empty() {
            return;
        }
        let (publisher, request) = (pending.session, pending.request);
        self.pending.remove(track_name);
        self.cancel_own_request(publisher, request, actions);
    }

    /// Whether the relay's SUBSCRIBE for the track went to that session and has
    /// no answer yet.
    pub(crate) fn is_asked_of(&self, track_name: &FullTrackName, session: SessionKey) -> bool {
        self.pending
            .get(track_name)
            .is_some_and(|pending| pending.session == session)
    }

    /// The publisher that was asked for the track sent PUBLISH for it: the
    /// relay's SUBSCRIBE is cancelled (§5.1). Returns the subscribers that
    /// waited for its answer.
    pub(crate) fn give_way_to_publish(
        &mut self,
        track_name: &FullTrackName,
        actions: &mut Vec<Action>,
    ) -> Vec<Waiter> {
        let Some(pending) = self.pending.remove(track_name) else {
            return Vec::new();
        };
        self.cancel_own_request(pending.session, pending.request, actions);
        pending.waiters
    }

    /// Cancels a request the relay opened and forgets it (§3.3.2).
    pub(crate) fn cancel_own_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        match state.own.remove(&request) {
            // The request ended with the publisher's answer; what is left of
            // it is the wait for the PUBLISH.
            Some(own) if own.awaits_publish => {
                actions.push(Action::StopUpstreamTimer { session, request });
            }
            Some(own) => {
                state.moqt.cancel(own.id);
                actions.push(Action::CancelOwnRequest {
                    session,
                    request,
                    code: stream_reset_code::CANCELLED,
                });
            }
            None => {}
        }
    }

    /// What a session that is gone leaves in the routing state of the others.
    pub(crate) fn drop_routing_of(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        self.namespaces.retain(|entry| entry.session != session);
        // Tracks that were asked of the session: it will not answer.
        let asked: BTreeSet<FullTrackName> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.session == session)
            .map(|(track, _)| track.clone())
            .collect();
        for track in asked {
            self.refuse_waiters(
                &track,
                &Refusal::code(request_error_code::DOES_NOT_EXIST),
                actions,
            );
        }
        // Tracks it waited for.
        let waited: BTreeSet<FullTrackName> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.waiters.iter().any(|w| w.session == session))
            .map(|(track, _)| track.clone())
            .collect();
        for track in waited {
            let streams: Vec<StreamKey> = self
                .pending
                .get(&track)
                .into_iter()
                .flat_map(|pending| &pending.waiters)
                .filter(|waiter| waiter.session == session)
                .map(|waiter| waiter.stream)
                .collect();
            for stream in streams {
                self.waiter_left(&track, session, stream, actions);
            }
        }
    }

    /// Answers every subscriber that waited for `track_name` with the same
    /// `REQUEST_ERROR`, and forgets the pending track.
    fn refuse_waiters(
        &mut self,
        track_name: &FullTrackName,
        refusal: &Refusal,
        actions: &mut Vec<Action>,
    ) {
        let Some(pending) = self.pending.remove(track_name) else {
            return;
        };
        for waiter in pending.waiters {
            let Some(state) = self.sessions.get_mut(&waiter.session) else {
                continue;
            };
            let response = state.moqt.reject(
                waiter.request,
                refusal.error_code,
                refusal.retry_interval,
                refusal.reason.clone(),
            );
            state.finish_slot(waiter.stream);
            self.respond(waiter.session, waiter.stream, response, actions);
            actions.push(Action::FinishRequest {
                session: waiter.session,
                stream: waiter.stream,
            });
        }
    }

    /// Answers a subscriber that waited with `SUBSCRIBE_OK`, now that the
    /// track has an Established upstream subscription.
    pub(crate) fn establish(
        &mut self,
        track_name: &FullTrackName,
        waiter: &Waiter,
        actions: &mut Vec<Action>,
    ) {
        let Waiter {
            session,
            stream,
            request,
            forward,
        } = *waiter;
        if let Some(subscriber) =
            self.accept_subscriber(track_name, session, stream, request, forward, actions)
            && let Some(track) = self.tracks.get_mut(track_name)
        {
            track.subscribers.push(subscriber);
        }
    }

    /// Sends `SUBSCRIBE_OK` for a track that has an upstream subscription and
    /// returns the subscriber to add to it.
    pub(crate) fn accept_subscriber(
        &mut self,
        track_name: &FullTrackName,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        forward: bool,
        actions: &mut Vec<Action>,
    ) -> Option<Subscriber> {
        let track = self.tracks.get(track_name)?;
        let state = self.sessions.get_mut(&session)?;
        // §10.2.11: LARGEST_OBJECT is present once Objects were published.
        let parameters = track
            .largest
            .map(Parameter::LargestObject)
            .into_iter()
            .collect();
        match state
            .moqt
            .accept_subscribe(request, parameters, track.properties.clone())
        {
            Ok((track_alias, message)) => {
                state.requests.insert(
                    stream,
                    RequestSlot::Subscribe {
                        request,
                        track: track_name.clone(),
                    },
                );
                actions.push(Action::SendRequest {
                    session,
                    stream,
                    message,
                });
                Some(Subscriber {
                    session,
                    stream,
                    request,
                    track_alias,
                    // §5.1: "The publisher does not send Objects if the Forward
                    // State is 0". REQUEST_UPDATE is refused, so a subscriber
                    // that asks for 0 stays there.
                    forward,
                    streams_started: 0,
                    count_unknown: false,
                    outs: BTreeSet::new(),
                })
            }
            Err(error) => {
                actions.push(internal_error(session, &error));
                None
            }
        }
    }
}
