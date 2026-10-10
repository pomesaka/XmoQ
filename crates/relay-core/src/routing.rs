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
//!
//! Two words for the side the Objects come from. A *publisher* is a peer: the
//! session that announced a namespace, or that sent PUBLISH. *Upstream* is the
//! relay's own part towards it: the SUBSCRIBE it sends, the wait for the
//! answer, the subscription it then holds (§9.4 says "upstream subscription").
//! So a function that is about the relay's request says upstream, and a field
//! that says who is on the other end says publisher. In the same way a
//! *waiter* is a downstream SUBSCRIBE that has no answer yet, and a
//! *subscriber* one that has its `SUBSCRIBE_OK`. A *source* of a track is an
//! upstream subscription that is Established: a PUBLISH the relay accepted,
//! or a SUBSCRIBE of the relay that a publisher accepted. An *ask* is a
//! SUBSCRIBE of the relay that is not a source yet.

use std::collections::BTreeSet;

use xmoq_moqt::message::{ControlMessage, ReasonPhrase, RequestError, request_error_code};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::Parameter;
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

impl Waiter {
    /// Whether this is the SUBSCRIBE on that request stream of that session.
    pub(crate) fn is_on(&self, session: SessionKey, stream: StreamKey) -> bool {
        self.session == session && self.stream == stream
    }

    pub(crate) fn new(
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        forward: bool,
    ) -> Self {
        Self {
            session,
            stream,
            request,
            forward,
        }
    }
}

/// How far a track has come with one request of the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamPhase {
    /// The publisher has not answered.
    Asked,
    /// The publisher answered `DUPLICATE_SUBSCRIPTION`: the request is over
    /// on its stream and in the session, and its PUBLISH for the track is
    /// awaited. Nothing that arrives for the request means anything any more.
    AwaitsPublish,
    /// The publisher accepted, and the request is the source of the track.
    Established,
    /// The publisher said `PUBLISH_DONE` on it.
    Draining,
}

/// What the relay knows about one of its requests: the one place the three
/// handlers of what comes back on a request look it up.
struct OwnRequestView {
    id: RequestId,
    track: FullTrackName,
    ask: Ask,
    /// `None`: the track is gone, or has gone on without the request.
    phase: Option<UpstreamPhase>,
}

/// A SUBSCRIBE of the relay to one publisher that is not Established: the
/// publisher has not answered, or has announced a PUBLISH instead
/// (`DUPLICATE_SUBSCRIPTION`).
///
/// WHY a thing of its own, kept by a track whether or not it has a source:
/// with several publishers asked for one track, one can have accepted while
/// another has not answered. Today one publisher is asked, so a track has at
/// most one, and none once it has a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) publisher: SessionKey,
    pub(crate) request: OwnRequestKey,
}

/// A track that has no source and that publishers were asked for. §9.4: no
/// `SUBSCRIBE_OK` goes downstream before an upstream subscription is
/// Established, so the subscribers wait.
#[derive(Debug)]
pub(crate) struct PendingTrack {
    /// Never empty: when the last ask is over, the waiters are refused and
    /// the track is forgotten.
    pub(crate) asks: Vec<Ask>,
    waiters: Vec<Waiter>,
}

/// What a request is refused with: the fields of a `REQUEST_ERROR`.
pub(crate) struct Refusal {
    error_code: u64,
    retry_interval: u64,
    reason: ReasonPhrase,
}

impl Refusal {
    /// With that code and nothing else.
    ///
    /// FIXME: a Retry Interval where a retry can succeed later, as when a
    /// publisher may still appear. §10.6.2 reads 0 as "do not retry".
    pub(crate) fn code(error_code: u64) -> Self {
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
    /// code it would explain something the subscriber was not told.
    ///
    /// The Retry Interval is the publisher's where the subscriber's retry
    /// is the publisher's business: with a code that is passed on, and with
    /// `DOES_NOT_EXIST` in place of another, since a subscriber that asks
    /// again makes the relay ask the same publisher again, and the publisher
    /// said when that may be. Under `INTERNAL_ERROR` in place of another code
    /// it is 0 ("SHOULD NOT be retried", §10.6.2): what the publisher's value
    /// was about is not known, and 0 promises nothing.
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
            // DUPLICATE_SUBSCRIPTION has no arm of its own. `passed_on` is
            // called for the answer to an ask, and that code from the
            // publisher of an ask is no refusal (`on_upstream_refusal`
            // returns before it gets here). For an answer to a request that
            // is no ask any more there are no waiters to pass anything on to.
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

    /// The sessions that announced a namespace the track is in, in the order
    /// they announced it.
    ///
    /// §9.5: Namespace Prefix Matching, field by field. §9.5 also has a
    /// publisher announce a namespace explicitly to get subscriptions routed
    /// to it, so sessions that only sent PUBLISH are not among them.
    pub(crate) fn matching_publishers<'a>(
        &'a self,
        track: &'a FullTrackName,
    ) -> impl Iterator<Item = SessionKey> + 'a {
        self.namespaces
            .iter()
            .filter(|entry| entry.namespace.is_prefix_of(track.namespace()))
            .map(|entry| entry.session)
    }

    /// Lets one more subscriber wait for the answer to an upstream SUBSCRIBE
    /// that is under way.
    ///
    /// §9.4 lets a relay make one upstream subscription for several
    /// subscribers; the ones that come while it is not answered wait for the
    /// same answer.
    ///
    /// # Errors
    ///
    /// The Error Code to refuse the subscriber with.
    pub(crate) fn wait_with_the_others(
        &mut self,
        track: &FullTrackName,
        waiter: Waiter,
    ) -> Result<(), u64> {
        // Never absent: the caller comes here for a track it found pending.
        let Some(pending) = self.tracks.pending_mut(track) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        if pending.waiters.len() >= MAX_WAITING_SUBSCRIBERS {
            return Err(request_error_code::EXCESSIVE_LOAD);
        }
        pending.waiters.push(waiter);
        Ok(())
    }

    /// Sends the relay's SUBSCRIBE for a track to a publisher, with the first
    /// subscriber that waits for the answer.
    ///
    /// # Errors
    ///
    /// The Error Code to refuse the subscriber with.
    pub(crate) fn ask(
        &mut self,
        publisher: SessionKey,
        track: &FullTrackName,
        waiter: Waiter,
        actions: &mut Vec<Action>,
    ) -> Result<(), u64> {
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
        self.tracks.insert_pending(
            track.clone(),
            PendingTrack {
                asks: vec![Ask {
                    publisher,
                    request: own,
                }],
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
        Ok(())
    }

    /// Looks up a request of the relay, and how far its track has come with
    /// it.
    fn own_request_of(
        &self,
        session: SessionKey,
        request: OwnRequestKey,
    ) -> Option<OwnRequestView> {
        let own = self.sessions.get(&session)?.own.get(&request)?;
        let ask = Ask {
            publisher: session,
            request,
        };
        let asked = self.tracks.asks(&own.track).contains(&ask);
        let draining = self
            .tracks
            .live(&own.track)
            .and_then(|track| track.subscription_is_draining(session, request));
        let phase = match (asked, draining) {
            (true, _) if own.awaits_publish => Some(UpstreamPhase::AwaitsPublish),
            (true, _) => Some(UpstreamPhase::Asked),
            (false, Some(true)) => Some(UpstreamPhase::Draining),
            (false, Some(false)) => Some(UpstreamPhase::Established),
            (false, None) => None,
        };
        Some(OwnRequestView {
            id: own.id,
            track: own.track.clone(),
            ask,
            phase,
        })
    }

    pub(crate) fn on_own_request_message(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(own) = self.own_request_of(session, request) else {
            return;
        };
        if own.phase == Some(UpstreamPhase::AwaitsPublish) {
            return;
        }
        // Never absent: the request was just found in this session.
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
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
                let accepted = TrackSource::Subscription {
                    session,
                    request,
                    track_alias,
                };
                let largest = parameters.iter().find_map(|parameter| match parameter {
                    Parameter::LargestObject(location) => Some(*location),
                    _ => None,
                });
                // §10.2.11: the relay's LARGEST_OBJECT starts from the one
                // the publisher gave. §9.6: its Track Properties are passed
                // on.
                let track = Track::established(accepted, track_properties, largest);
                self.on_upstream_accept(&own, track, actions);
            }
            RequestEvent::UnsupportedTrack { track_alias, .. } => {
                self.on_upstream_unsupported(&own, track_alias, actions);
            }
            RequestEvent::Rejected(error) => self.on_upstream_refusal(&own, &error, actions),
            RequestEvent::SubscriptionDone(_) => self.on_upstream_done(&own, actions),
            // Nothing else answers a SUBSCRIBE; the session has dealt with it.
            _ => {}
        }
    }

    /// The publisher accepted the relay's SUBSCRIBE: the track has an
    /// Established upstream subscription (§9.4), and the subscribers that
    /// waited are answered.
    fn on_upstream_accept(
        &mut self,
        own: &OwnRequestView,
        mut track: Track,
        actions: &mut Vec<Action>,
    ) {
        let Some(mut pending) = self.tracks.take_pending(&own.track) else {
            return;
        };
        let Ask { publisher, request } = own.ask;
        // The answer came, and the request goes on. What was asked of other
        // publishers is still asked.
        actions.push(Action::StopUpstreamTimer {
            session: publisher,
            request,
        });
        pending.asks.retain(|ask| *ask != own.ask);
        track.asks = pending.asks;
        let (_, track_alias) = track.source.session_and_alias();
        self.tracks.insert_live(own.track.clone(), track);
        for waiter in &pending.waiters {
            self.establish(&own.track, waiter, actions);
        }
        // The publisher may have opened streams before its SUBSCRIBE_OK was
        // read.
        self.replay_early_streams(publisher, track_alias, actions);
    }

    /// The publisher accepted with a Mandatory Track Property the relay does
    /// not understand. §2.5.1: the subscription is not established, and the
    /// stream is cancelled. So are the streams that came ahead of the answer
    /// under its alias.
    fn on_upstream_unsupported(
        &mut self,
        own: &OwnRequestView,
        track_alias: u64,
        actions: &mut Vec<Action>,
    ) {
        let Ask { publisher, request } = own.ask;
        let Some(state) = self.sessions.get_mut(&publisher) else {
            return;
        };
        state.own.remove(&request);
        state.drop_early_streams(publisher, track_alias, actions);
        actions.push(Action::CancelOwnRequest {
            session: publisher,
            request,
            code: stream_reset_code::CANCELLED,
        });
        let unsupported = Refusal::code(request_error_code::UNSUPPORTED_EXTENSION);
        self.ask_is_over(&own.track, own.ask, &unsupported, actions);
    }

    /// The publisher said `PUBLISH_DONE` on the relay's SUBSCRIBE.
    fn on_upstream_done(&mut self, own: &OwnRequestView, actions: &mut Vec<Action>) {
        let Ask { publisher, request } = own.ask;
        let Some(track) = self.tracks.live(&own.track) else {
            return;
        };
        let (_, track_alias) = track.source.session_and_alias();
        self.source_done(
            &own.track,
            |source| source.is_subscription(publisher, request),
            publisher,
            track_alias,
            actions,
        );
    }

    /// The publisher answered the relay's SUBSCRIBE with `REQUEST_ERROR`.
    fn on_upstream_refusal(
        &mut self,
        own: &OwnRequestView,
        error: &RequestError,
        actions: &mut Vec<Action>,
    ) {
        let Ask {
            publisher: session,
            request,
        } = own.ask;
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
        let asked = own.phase == Some(UpstreamPhase::Asked);
        if asked && error.error_code == request_error_code::DUPLICATE_SUBSCRIPTION {
            if let Some(own) = state.own.get_mut(&request) {
                own.awaits_publish = true;
            }
            actions.push(Action::StartUpstreamTimer { session, request });
            return;
        }
        state.own.remove(&request);
        self.ask_is_over(&own.track, own.ask, &Refusal::passed_on(error), actions);
    }

    /// One ask of a track is over without a source: the publisher refused,
    /// did not answer, or is gone. When it was the last one of a track that
    /// has no source, the subscribers that waited are refused with `refusal`.
    ///
    /// WHY the last one: a track another publisher was asked for may still
    /// come. And a track that has a source loses nothing with an ask.
    fn ask_is_over(
        &mut self,
        track_name: &FullTrackName,
        ask: Ask,
        refusal: &Refusal,
        actions: &mut Vec<Action>,
    ) {
        if let Some(asks) = self.tracks.asks_mut(track_name) {
            asks.retain(|other| *other != ask);
        }
        let hopeless = self
            .tracks
            .pending(track_name)
            .is_some_and(|pending| pending.asks.is_empty());
        if hopeless {
            self.refuse_waiters(track_name, refusal, actions);
        }
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
        let Some(own) = self.own_request_of(session, request) else {
            return;
        };
        match own.phase {
            Some(UpstreamPhase::Asked) => {
                self.cancel_own_request(session, request, actions);
                let nobody = Refusal::code(request_error_code::DOES_NOT_EXIST);
                self.ask_is_over(&own.track, own.ask, &nobody, actions);
            }
            Some(UpstreamPhase::Established) => {
                self.end_track(&own.track, TrackEnd::SourceGone, actions);
            }
            // The publisher closes the stream after its REQUEST_ERROR
            // (§3.3.2). After PUBLISH_DONE it closes it as well (§10.11), and
            // the track goes on draining.
            Some(UpstreamPhase::AwaitsPublish | UpstreamPhase::Draining) | None => {}
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
        let Some(own) = self.own_request_of(session, request) else {
            return;
        };
        // Never absent: the request was just found in this session.
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // The timer has run out, so only the request is left to let go of.
        match own.phase {
            Some(UpstreamPhase::Asked) => {
                state.own.remove(&request);
                state.moqt.cancel(own.id);
                actions.push(Action::CancelOwnRequest {
                    session,
                    request,
                    code: stream_reset_code::CANCELLED,
                });
            }
            Some(UpstreamPhase::AwaitsPublish) => {
                state.own.remove(&request);
            }
            Some(UpstreamPhase::Established | UpstreamPhase::Draining) | None => return,
        }
        // §10.6.2, TIMEOUT: "a relay could not establish an upstream
        // subscription within the timeout".
        let too_late = Refusal::code(request_error_code::TIMEOUT);
        self.ask_is_over(&own.track, own.ask, &too_late, actions);
    }

    /// Takes the subscribers that `gone` picks off a pending track. When
    /// nobody is left, what was asked upstream is cancelled: nobody wants the
    /// answer.
    pub(crate) fn waiters_left(
        &mut self,
        track_name: &FullTrackName,
        gone: impl Fn(&Waiter) -> bool,
        actions: &mut Vec<Action>,
    ) {
        let Some(pending) = self.tracks.pending_mut(track_name) else {
            return;
        };
        pending.waiters.retain(|waiter| !gone(waiter));
        if !pending.waiters.is_empty() {
            return;
        }
        for ask in self
            .tracks
            .take_pending(track_name)
            .map_or(Vec::new(), |pending| pending.asks)
        {
            self.cancel_own_request(ask.publisher, ask.request, actions);
        }
    }

    /// Whether the track has no source and a SUBSCRIBE of the relay for it
    /// went to that session.
    pub(crate) fn is_asked_of(&self, track_name: &FullTrackName, session: SessionKey) -> bool {
        self.tracks
            .pending(track_name)
            .is_some_and(|pending| pending.asks.iter().any(|ask| ask.publisher == session))
    }

    /// A publisher that was asked for the track sent PUBLISH for it: the
    /// relay's SUBSCRIBE to it is cancelled (§5.1). Returns the subscribers
    /// that waited, and what is still asked of other publishers.
    pub(crate) fn give_way_to_publish(
        &mut self,
        track_name: &FullTrackName,
        publisher: SessionKey,
        actions: &mut Vec<Action>,
    ) -> (Vec<Waiter>, Vec<Ask>) {
        let Some(pending) = self.tracks.take_pending(track_name) else {
            return (Vec::new(), Vec::new());
        };
        let (given_up, others): (Vec<Ask>, Vec<Ask>) = pending
            .asks
            .into_iter()
            .partition(|ask| ask.publisher == publisher);
        for ask in given_up {
            self.cancel_own_request(ask.publisher, ask.request, actions);
        }
        (pending.waiters, others)
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
    ///
    /// By track name both times, so that the same events lead to the same
    /// actions.
    pub(crate) fn drop_routing_of(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        self.namespaces.retain(|entry| entry.session != session);
        // What was asked of the session: it will not answer.
        let asked: BTreeSet<(FullTrackName, OwnRequestKey)> = self
            .tracks
            .all_asks()
            .filter(|(_, ask)| ask.publisher == session)
            .map(|(track, ask)| (track.clone(), ask.request))
            .collect();
        let nobody = Refusal::code(request_error_code::DOES_NOT_EXIST);
        for (track, request) in asked {
            let ask = Ask {
                publisher: session,
                request,
            };
            self.ask_is_over(&track, ask, &nobody, actions);
        }
        // Tracks it waited for.
        let waited: BTreeSet<FullTrackName> = self
            .tracks
            .all_pending()
            .filter(|(_, pending)| pending.waiters.iter().any(|w| w.session == session))
            .map(|(track, _)| track.clone())
            .collect();
        for track in waited {
            self.waiters_left(&track, |waiter| waiter.session == session, actions);
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
        let Some(pending) = self.tracks.take_pending(track_name) else {
            return;
        };
        for waiter in pending.waiters {
            self.refuse(
                waiter.session,
                waiter.stream,
                waiter.request,
                refusal,
                actions,
            );
        }
    }

    /// Answers a request with `REQUEST_ERROR` and a FIN (§3.3.2).
    pub(crate) fn refuse(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        refusal: &Refusal,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let response = state.moqt.reject(
            request,
            refusal.error_code,
            refusal.retry_interval,
            refusal.reason.clone(),
        );
        state.finish_slot(stream);
        self.respond(session, stream, response, actions);
        actions.push(Action::FinishRequest { session, stream });
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
            && let Some(track) = self.tracks.live_mut(track_name)
        {
            track.subscribers.push(subscriber);
        }
    }

    /// Sends `SUBSCRIBE_OK` for a track that has an upstream subscription and
    /// returns the subscriber to add to it.
    fn accept_subscriber(
        &mut self,
        track_name: &FullTrackName,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        forward: bool,
        actions: &mut Vec<Action>,
    ) -> Option<Subscriber> {
        let track = self.tracks.live(track_name)?;
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
                // A subscriber that finished its direction while it waited
                // has still finished it.
                let peer_finished = state.peer_finished(stream);
                state.requests.insert(
                    stream,
                    RequestSlot::Subscribe {
                        request,
                        track: track_name.clone(),
                        peer_finished,
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
