//! Routing by namespace (§9.5): a SUBSCRIBE for a track nobody sent a PUBLISH
//! for is passed on to the publishers that announced a matching namespace
//! with `PUBLISH_NAMESPACE`, and answered when the first of them has
//! accepted or the last of them has not.
//!
//! A subscriber's SUBSCRIBE goes to every publisher of a matching namespace
//! that the track has room for, also when the track has a source already
//! ([`Relay::ask_the_publishers`]).
//!
//! A publisher whose `PUBLISH_NAMESPACE` arrives while tracks of the
//! namespace are subscribed to is asked for each of them then.
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

use std::collections::{BTreeSet, VecDeque};

use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{ControlMessage, ReasonPhrase, RequestError, request_error_code};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{Parameter, SubscriptionFilter};
use xmoq_moqt::session::{PeerRequestId, RequestEvent, RequestId};
use xmoq_moqt::{RequestFault, stream_reset_code};

use crate::{
    Action, MAX_SOURCES, OwnRequestEnd, OwnRequestKey, Relay, RequestSlot, SessionKey, Source,
    StreamKey, Subscriber, TrackEnd, TrackSource, internal_error, largest_object,
};

/// Namespaces one session may have announced at once. One more is refused
/// with `EXCESSIVE_LOAD`. Provisional.
pub const MAX_NAMESPACES_PER_SESSION: usize = 64;

/// Subscribers that may wait for one upstream SUBSCRIBE to be answered. One
/// more is refused with `EXCESSIVE_LOAD`. Provisional.
pub const MAX_WAITING_SUBSCRIBERS: usize = 64;

/// Requests the relay may have open on one session at once: its SUBSCRIBEs
/// to a publisher. A publisher that has as many is passed over, and a
/// subscriber for whose SUBSCRIBE nobody else could be asked is refused
/// with `EXCESSIVE_LOAD`. Provisional.
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

/// Publishers a track remembers as having been asked for it without
/// becoming a source. Past that the one remembered longest is forgotten, and
/// is asked again when the next subscriber comes. Provisional.
pub const MAX_DECLINED_PUBLISHERS: usize = 16;

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

/// The first Location a subscription passes (§5.1.2), for a SUBSCRIBE that is
/// accepted while `largest` is the Largest Object of its track. `None`:
/// nothing passes, which is so only where the filter starts after a Location
/// that has none after it.
///
/// Without a filter: {0, 0}, "Subscriptions without a filter pass all
/// Objects published or received via upstream subscriptions". Largest
/// Object: "{Largest Object.Group, Largest Object.Object + 1}"; Next Group
/// Start: "{Largest Object.Group + 1, 0}"; for both, "If no content has been
/// delivered yet, the filter Start Location is {0, 0}". `AbsoluteStart` and
/// `AbsoluteRange`: the Start Location they give, which "MAY be less than the
/// Largest Object observed at the publisher". Where an `AbsoluteRange`
/// filter ends is [`filter_end_group`].
pub(crate) fn filter_start(
    filter: Option<SubscriptionFilter>,
    largest: Option<Location>,
) -> Option<Location> {
    let first = Location {
        group: 0,
        object: 0,
    };
    match (filter, largest) {
        (None, _)
        | (Some(SubscriptionFilter::LargestObject | SubscriptionFilter::NextGroupStart), None) => {
            Some(first)
        }
        (Some(SubscriptionFilter::LargestObject), Some(largest)) => {
            match largest.object.checked_add(1) {
                Some(object) => Some(Location {
                    group: largest.group,
                    object,
                }),
                // The Group has no Object after its last possible one.
                None => next_group(largest),
            }
        }
        (Some(SubscriptionFilter::NextGroupStart), Some(largest)) => next_group(largest),
        (
            Some(
                SubscriptionFilter::AbsoluteStart { start }
                | SubscriptionFilter::AbsoluteRange { start, .. },
            ),
            _,
        ) => Some(start),
    }
}

/// The last Group a subscription passes (§5.1.2), `None` where it is open
/// ended: every filter but `AbsoluteRange`, of which the draft has "There is
/// no End Group - the subscription is open ended".
///
/// The End Group is the Group of the Start Location plus the End Group
/// Delta, and it passes whole: "Only objects ... having Locations greater
/// than or equal to Start Location and strictly less than or equal to the
/// End Group (when present) pass the filter"; "If the specified End Group
/// Delta is zero, the remainder of that Group passes the filter. Otherwise,
/// the last Group ID to be delivered will be the Group ID in Start Location
/// plus the End Group Delta."
pub(crate) fn filter_end_group(filter: Option<SubscriptionFilter>) -> Option<u64> {
    match filter? {
        SubscriptionFilter::AbsoluteRange {
            start,
            end_group_delta,
        } => {
            // A sum over 2^64 - 1 is a PROTOCOL_VIOLATION the decoder has
            // found (§5.1.2); the last Group there is stands for it here.
            Some(start.group.saturating_add(end_group_delta))
        }
        SubscriptionFilter::LargestObject
        | SubscriptionFilter::NextGroupStart
        | SubscriptionFilter::AbsoluteStart { .. } => None,
    }
}

fn next_group(largest: Location) -> Option<Location> {
    Some(Location {
        group: largest.group.checked_add(1)?,
        object: 0,
    })
}

/// A downstream SUBSCRIBE that waits for the upstream one to be answered.
#[derive(Debug)]
pub(crate) struct Waiter {
    session: SessionKey,
    stream: StreamKey,
    request: PeerRequestId,
    forward: bool,
    /// The Subscription Filter of the SUBSCRIBE (§5.1.2). Its Start Location
    /// is worked out when the SUBSCRIBE is accepted, from the Largest Object
    /// of that moment.
    filter: Option<SubscriptionFilter>,
    /// What the SUBSCRIBE asked of the delivery.
    asked: Delivery,
}

/// What a SUBSCRIBE asks of how its Objects are delivered.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Delivery {
    /// [`Subscriber::subgroup_delivery_timeout`].
    pub(crate) subgroup_delivery_timeout: Option<u64>,
    /// [`Subscriber::priority`].
    pub(crate) subscriber_priority: u8,
}

impl Delivery {
    /// Of the parameters of a SUBSCRIBE (§10.2.3, §10.2.7).
    pub(crate) fn of(parameters: &[Parameter]) -> Self {
        Self {
            subgroup_delivery_timeout: xmoq_moqt::params::subgroup_delivery_timeout(parameters),
            subscriber_priority: parameters
                .iter()
                .find_map(|parameter| match parameter {
                    Parameter::SubscriberPriority(priority) => Some(*priority),
                    _ => None,
                })
                .unwrap_or(crate::DEFAULT_PRIORITY),
        }
    }
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
        filter: Option<SubscriptionFilter>,
        asked: Delivery,
    ) -> Self {
        Self {
            session,
            stream,
            request,
            forward,
            filter,
            asked,
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
/// a track can get a source while the publisher that was asked has not
/// answered, from a PUBLISH of another publisher, and with several
/// publishers asked for one track, one can have accepted while another has
/// not answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) publisher: SessionKey,
    pub(crate) request: OwnRequestKey,
}

/// What [`Relay::ask_the_publishers`] did.
pub(crate) struct Asked {
    pub(crate) asks: Vec<Ask>,
    /// The Error Code of the first publisher that could not be asked.
    pub(crate) nobody_because: Option<u64>,
}

/// A track that has no source and that publishers were asked for. §9.4: no
/// `SUBSCRIBE_OK` goes downstream before an upstream subscription is
/// Established, so the subscribers wait.
#[derive(Debug)]
pub(crate) struct PendingTrack {
    /// Never empty: when the last ask is over, the waiters are refused and
    /// the track is forgotten.
    pub(crate) asks: Vec<Ask>,
    pub(crate) waiters: Vec<Waiter>,
    /// As [`Track::declined`](crate::Track), for as long as the track waits;
    /// it goes on to the track with a source.
    pub(crate) declined: Declined,
}

/// The sessions that were asked for a track and did not become a source,
/// the one that is remembered longest first; no more than
/// [`MAX_DECLINED_PUBLISHERS`].
///
/// A session is in at most one of three places of a track: its sources,
/// what is asked, and here. [`Tracks::decline`](crate::Tracks) puts one in,
/// and only one that is no source; [`Tracks::add_source`](crate::Tracks)
/// takes out the one that becomes a source; [`Relay::ask_the_publishers`]
/// asks nobody who is in here.
#[derive(Debug, Default)]
pub(crate) struct Declined(VecDeque<SessionKey>);

impl Declined {
    pub(crate) fn remember(&mut self, session: SessionKey) {
        self.forget(session);
        self.0.push_back(session);
        if self.0.len() > MAX_DECLINED_PUBLISHERS {
            self.0.pop_front();
        }
    }

    pub(crate) fn forget(&mut self, session: SessionKey) {
        self.0.retain(|declined| *declined != session);
    }

    pub(crate) fn sessions(&self) -> impl Iterator<Item = SessionKey> + '_ {
        self.0.iter().copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

/// Which request of the relay a publisher answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnRequestKind {
    Subscribe,
    Fetch,
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
    pub(crate) fn passed_on(error: &RequestError, asked: OwnRequestKind) -> Self {
        use request_error_code as code;
        let error_code = match error.error_code {
            // §10.6.2 defines these two for a range of Locations, and
            // MALFORMED_TRACK "In response to a FETCH". The relay's SUBSCRIBE
            // has no range (last arm).
            passed @ (code::INVALID_RANGE | code::MALFORMED_TRACK) if asked == OwnRequestKind::Fetch => {
                passed
            }
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
            // Codes that do not answer the relay's request. For a SUBSCRIBE
            // those are also MALFORMED_TRACK and INVALID_RANGE (first arm).
            // UNINTERESTED, PREFIX_OVERLAP and the like answer a PUBLISH or a
            // namespace request, and INVALID_JOINING_REQUEST_ID a FETCH that
            // joins a subscription, which the relay's never does. The rest
            // is unknown here.
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
        // A publisher that announces again may have the tracks now that it
        // did not have when it was asked.
        self.tracks
            .forget_declined(session, |track| namespace.is_prefix_of(track.namespace()));
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

    /// Sends the relay's SUBSCRIBE for a track to every publisher that
    /// announced a namespace of it and can become one more source, in the
    /// order they announced (§9.5: "Relays MUST send SUBSCRIBE messages to
    /// all matching publishers"). What was asked is put with the track, and
    /// returned for a name that has no track yet: making one is the caller's.
    ///
    /// Left out are the sessions that are a source of the track, that are
    /// asked for it, and that were asked and did not become a source
    /// ([`MAX_DECLINED_PUBLISHERS`]).
    ///
    /// WHY no more than the track has room for, against that MUST: a
    /// publisher that accepts would be cancelled at once when the track is
    /// full, having started to send. §9.3 lets a relay turn an additional
    /// publisher away "due to implementation constraints". The ones left out
    /// are asked when a subscriber comes, or a namespace of the track is
    /// announced, and there is room.
    ///
    /// `first` is the session that announced a namespace just now, when that
    /// is why the publishers are asked: it comes before the ones that
    /// announced earlier and were left out, since §9.5 has the SUBSCRIBE go
    /// "to the publisher that sent the `PUBLISH_NAMESPACE`".
    pub(crate) fn ask_the_publishers(
        &mut self,
        track: &FullTrackName,
        first: Option<SessionKey>,
        actions: &mut Vec<Action>,
    ) -> Asked {
        // WHY the sources are left out here, though the session state would
        // refuse a second subscription to the track on a session that is a
        // source (§5.1) and the relay would then open nothing: which
        // sessions are asked is this function's to say, and the room of the
        // track is counted from the same list. No test tells the two apart.
        let mut taken: Vec<SessionKey> = self.tracks.live(track).map_or(Vec::new(), |live| {
            live.sources()
                .map(|source| source.session_and_alias().0)
                .collect()
        });
        let sources = taken.len();
        let asks = self.tracks.asks(track);
        taken.extend(asks.iter().map(|ask| ask.publisher));
        taken.extend(self.tracks.declined(track));
        let room = MAX_SOURCES.saturating_sub(sources + asks.len());
        // A session that announced two namespaces of the track is one
        // publisher.
        let mut publishers: Vec<SessionKey> = Vec::new();
        for publisher in first.into_iter().chain(self.matching_publishers(track)) {
            if !taken.contains(&publisher) && !publishers.contains(&publisher) {
                publishers.push(publisher);
            }
        }
        let mut asked = Asked {
            asks: Vec::new(),
            nobody_because: None,
        };
        for publisher in publishers {
            if asked.asks.len() >= room {
                break;
            }
            match self.ask(publisher, track, actions) {
                Ok(ask) => asked.asks.push(ask),
                Err(error_code) => {
                    asked.nobody_because.get_or_insert(error_code);
                }
            }
        }
        if let Some(asks) = self.tracks.asks_mut(track) {
            asks.extend(asked.asks.iter().copied());
        }
        asked
    }

    /// Sends the relay's SUBSCRIBE for a track to a publisher.
    ///
    /// # Errors
    ///
    /// The Error Code for a subscriber that nobody else could be asked for.
    fn ask(
        &mut self,
        publisher: SessionKey,
        track: &FullTrackName,
        actions: &mut Vec<Action>,
    ) -> Result<Ask, u64> {
        let Some(state) = self.sessions.get_mut(&publisher) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        if state.own_requests() >= MAX_OWN_REQUESTS_PER_SESSION {
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
        actions.push(Action::OpenRequest {
            session: publisher,
            request: own,
            message,
        });
        actions.push(Action::StartUpstreamTimer {
            session: publisher,
            request: own,
        });
        Ok(Ask {
            publisher,
            request: own,
        })
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
                let largest = largest_object(&parameters);
                // §10.2.11: the relay's LARGEST_OBJECT starts from the one
                // the publisher gave. §9.6: its Track Properties are passed
                // on.
                self.on_upstream_accept(&own, accepted, track_properties, largest, actions);
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
        accepted: TrackSource,
        properties: Vec<KeyValuePair>,
        largest: Option<Location>,
        actions: &mut Vec<Action>,
    ) {
        // An answer to what was asked, whether the track still waits for its
        // first source or got one from another publisher since.
        if own.phase != Some(UpstreamPhase::Asked) {
            return;
        }
        let Ask { publisher, request } = own.ask;
        // The answer came, and the request goes on. What was asked of other
        // publishers is still asked.
        actions.push(Action::StopUpstreamTimer {
            session: publisher,
            request,
        });
        if let Some(asks) = self.tracks.asks_mut(&own.track) {
            asks.retain(|ask| *ask != own.ask);
        }
        let (_, track_alias) = accepted.session_and_alias();
        let Ok(added) = self
            .tracks
            .add_source(&own.track, accepted, properties, largest)
        else {
            // The track got its sources elsewhere while the relay waited for
            // this answer, and cannot take another. For the publisher the
            // subscription is Established, so it is cancelled (§3.3.2), as
            // one the relay cannot use for another reason. Reached for a
            // track that has all the sources it may have. A source on the
            // same session is the other reason `add_source` has, and does
            // not get here: a publisher that becomes a source with PUBLISH
            // has had what was asked of it cancelled.
            //
            // The session has given the alias to this subscription all the
            // same, so a source that drained under it is over.
            self.end_sources_under(publisher, track_alias, None, actions);
            self.cancel_own_request(publisher, request, actions);
            if let Some(state) = self.sessions.get_mut(&publisher) {
                state.drop_early_streams(publisher, track_alias, actions);
            }
            return;
        };
        self.alias_was_taken_over(&own.track, accepted, added.replaced, actions);
        for waiter in &added.waiters {
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
        // WHY the phase is not looked at: the session reports this for the
        // one answer a SUBSCRIBE gets, so the request is one that was asked
        // and had no answer.
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
        let done = self.tracks.live(&own.track).and_then(|track| {
            track
                .sources()
                .find(|source| source.is_subscription(publisher, request))
        });
        let Some((_, track_alias)) = done.map(|source| source.session_and_alias()) else {
            return;
        };
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
        self.ask_is_over(
            &own.track,
            own.ask,
            &Refusal::passed_on(error, OwnRequestKind::Subscribe),
            actions,
        );
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
        // WHY remembered: the track goes on, with the sources it has or
        // with what is still asked, and without this the publisher would get
        // a SUBSCRIBE for it with every subscriber that comes, in the place
        // of one that was left out for want of room. A track without a
        // source is over when its last ask is, and what it remembered with
        // it: the next subscriber starts anew.
        self.tracks.decline(track_name, ask.publisher);
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
                let closed = |source: &Source| source.is_subscription(session, request);
                self.source_gone(&own.track, closed, TrackEnd::SourceGone, actions);
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

    /// Whether a SUBSCRIBE of the relay for the track went to that session
    /// and has no answer yet, whether or not the track has a source.
    pub(crate) fn is_asked_of(&self, track_name: &FullTrackName, session: SessionKey) -> bool {
        self.tracks
            .asks(track_name)
            .iter()
            .any(|ask| ask.publisher == session)
    }

    /// A publisher that was asked for the track sent PUBLISH for it: the
    /// relay's SUBSCRIBE to it is cancelled (§5.1). What is asked of other
    /// publishers is still asked.
    pub(crate) fn give_way_to_publish(
        &mut self,
        track_name: &FullTrackName,
        publisher: SessionKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(asks) = self.tracks.asks_mut(track_name) else {
            return;
        };
        let (given_up, others): (Vec<Ask>, Vec<Ask>) = std::mem::take(asks)
            .into_iter()
            .partition(|ask| ask.publisher == publisher);
        *asks = others;
        for ask in given_up {
            self.cancel_own_request(ask.publisher, ask.request, actions);
        }
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
        // WHY last: what was asked of the session ended above, which is
        // remembered like any ask that ends without a source.
        self.tracks.forget_declined(session, |_| true);
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
        self.respond(session, stream, response, actions);
        actions.push(Action::FinishRequest { session, stream });
        // A SUBSCRIBE that is refused takes its Joining FETCHes with it.
        self.finish_slot(session, stream, Some(request), actions);
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
            filter,
            asked,
        } = *waiter;
        if let Some((subscriber, slot)) = self.accept_subscriber(
            track_name,
            (session, stream, request),
            forward,
            filter,
            asked,
            actions,
        ) && let Some(track) = self.tracks.live_mut(track_name)
        {
            track.subscribers.push(subscriber);
            // After the subscriber is in the track: a Joining FETCH that
            // waited is let go by the slot, and reads its Joining Location
            // there.
            self.put_slot(session, stream, slot, actions);
        }
    }

    /// Sends `SUBSCRIBE_OK` for a track that has an upstream subscription and
    /// returns the subscriber to add to it.
    fn accept_subscriber(
        &mut self,
        track_name: &FullTrackName,
        (session, stream, request): (SessionKey, StreamKey, PeerRequestId),
        forward: bool,
        filter: Option<SubscriptionFilter>,
        asked: Delivery,
        actions: &mut Vec<Action>,
    ) -> Option<(Subscriber, RequestSlot)> {
        let track = self.tracks.live(track_name)?;
        let state = self.sessions.get_mut(&session)?;
        // §10.2.11: LARGEST_OBJECT is present once Objects were published.
        // WHY one value for the three: the subscriber works out from the
        // LARGEST_OBJECT of its SUBSCRIBE_OK where its subscription starts
        // and where a Joining FETCH ends (§5.1.2: "Largest Object is
        // communicated in SUBSCRIBE_OK"; §10.12.2.1: "contiguous and
        // non-overlapping"). The relay has to count from the same Location.
        let joining = track.largest;
        let start = filter_start(filter, joining);
        let end_group = filter_end_group(filter);
        let parameters = joining.map(Parameter::LargestObject).into_iter().collect();
        match state
            .moqt
            .accept_subscribe(request, parameters, track.properties.clone())
        {
            Ok((track_alias, message)) => {
                // A subscriber that finished its direction while it waited
                // has still finished it.
                let peer_finished = state.peer_finished(stream);
                let slot = RequestSlot::Subscribe {
                    request,
                    track: track_name.clone(),
                    peer_finished,
                };
                actions.push(Action::SendRequest {
                    session,
                    stream,
                    message,
                });
                let subscriber = Subscriber {
                    session,
                    stream,
                    request,
                    track_alias,
                    // §5.1: "The publisher does not send Objects if the Forward
                    // State is 0". REQUEST_UPDATE is refused, so a subscriber
                    // that asks for 0 stays there.
                    forward,
                    joining,
                    start,
                    end_group,
                    subgroup_delivery_timeout: asked.subgroup_delivery_timeout,
                    priority: asked.subscriber_priority,
                    streams_started: 0,
                    count_unknown: false,
                    outs: BTreeSet::new(),
                };
                Some((subscriber, slot))
            }
            // The session is closed for it. A Joining FETCH that waited for
            // this SUBSCRIBE goes when the session does
            // (`Relay::drop_fetches_of`).
            Err(error) => {
                actions.push(internal_error(session, &error));
                None
            }
        }
    }
}
