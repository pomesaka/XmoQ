//! Requests the peer opens (§3.3, §5.1, §6.2, §10.1): the responder side.

use std::collections::{BTreeSet, HashMap};

use super::fetch::ends_before;
use super::{
    MAX_UNANSWERED_UPDATES, Request, RequestEvent, RequestId, Role, Session, check_own_parameters,
    first_request_id,
};
use crate::aliases::SubscribedAlias;
use crate::immutable::{TrackRefusal, track_properties_refusal};
use crate::kvp::KeyValuePair;
use crate::message::{
    ControlMessage, Fetch, FetchOk, FetchRange, GoAway, Publish, PublishDone, PublishNamespace,
    ReasonPhrase, RequestError, RequestOk, RequestUpdate, Subscribe, SubscribeNamespace,
    SubscribeOk, publish_done_status, request_error_code, retry_interval,
};
use crate::name::{FullTrackName, Location, TrackNamespace};
use crate::params::{self, MessageKind, Parameter};
use crate::{Error, RequestFault, UsageError};

/// Requests of the peer that are open at once, unless
/// [`Session::set_most_peer_requests`] says otherwise. One more is answered
/// with `REQUEST_ERROR` `EXCESSIVE_LOAD`.
///
/// Provisional: the draft sets no limit, and `XmoQ` has not decided one.
pub const MAX_PEER_REQUESTS: usize = 256;

/// Request IDs of the peer that are remembered above the lowest one not seen
/// yet, while requests arrive out of order or leave a gap.
///
/// WHY remember IDs above a gap and not treat everything below the largest as
/// used: each request has a stream of its own, so IDs arrive out of order, and
/// a request the peer reset before its bytes arrived leaves a gap for good.
/// Counting the gap as used would turn a late, valid request into a duplicate
/// and close the session. §10.1 asks the receiver for the parity and for
/// duplicates, nothing about order.
/// WHY a limit: the peer chooses how far ahead of a gap it goes. Once the
/// memory is full, each new ID pushes out the lowest remembered one, and a
/// duplicate of an ID that old is no longer detected.
///
/// Provisional, and the same number and method as the TypeScript
/// implementation (`packages/moqt/src/core/peer-requests.ts`), so that the two
/// take the same input the same way.
pub const REQUEST_IDS_AHEAD_LIMIT: usize = 1024;

/// Names one request the peer opened. Only [`Session`] creates these, and a
/// session refuses a handle of another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerRequestId {
    session: u64,
    id: u64,
}

impl PeerRequestId {
    /// The Request ID as it appears on the wire (§10.1).
    #[must_use]
    pub fn value(self) -> u64 {
        self.id
    }
}

/// A request of the peer that this endpoint can accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerRequest {
    /// The peer wants to subscribe to a track.
    Subscribe(Subscribe),
    /// The peer offers a track.
    Publish(Publish),
    /// The peer announces a namespace.
    PublishNamespace(PublishNamespace),
    /// The peer asks for a range of a track. A Joining FETCH names a
    /// SUBSCRIBE of the peer that is open in this session, or a Request ID
    /// whose request is yet to come: which of the two,
    /// [`Session::peer_request_yet_to_come`] says, and a caller that holds
    /// no FETCH for a SUBSCRIBE to come refuses the second.
    Fetch(Fetch),
    /// The peer asks for the namespaces under a prefix (§6.1, §10.18). The
    /// caller accepts with [`Session::accept_subscribe_namespace`] and sends
    /// the NAMESPACE and `NAMESPACE_DONE` messages on the stream itself.
    SubscribeNamespace(SubscribeNamespace),
    /// The peer asks what a SUBSCRIBE for a track would be answered with
    /// (§10.14). The caller answers with [`Session::accept_track_status`] or
    /// [`Session::reject`], and closes the stream with a FIN after it.
    TrackStatus(Subscribe),
}

/// What the first message on a request stream of the peer led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerRequestOpened {
    /// A request to decide on: the caller answers with one of the `accept_*`
    /// methods or with [`Session::reject`], and reports the end of the stream
    /// with [`Session::on_peer_request_closed`].
    Request(PeerRequestId, PeerRequest),
    /// The session refused the request by itself. The caller sends `response`,
    /// a `REQUEST_ERROR`, and ends the stream with a FIN (§3.3.2).
    Refused {
        /// The message to send on the request stream.
        response: ControlMessage,
        /// The request, which the session keeps as one that was rejected:
        /// the caller feeds what still comes on its stream with
        /// [`Session::on_peer_request_message`] and reports the end of the
        /// stream with [`Session::on_peer_request_closed`], as for a request
        /// it rejected itself. `None` where the session keeps nothing, which
        /// is when it holds as many requests of the peer as it may
        /// ([`MAX_PEER_REQUESTS`]); what still comes on that stream is
        /// dropped.
        request: Option<PeerRequestId>,
    },
}

/// What the caller has to do after sending the `REQUEST_ERROR` that refused a
/// `REQUEST_UPDATE` (§10.9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRefusal {
    /// Nothing: the peer is the publisher and ends the subscription itself.
    Nothing,
    /// End the subscription with [`Session::subscribe_done`] and the status
    /// `UPDATE_FAILED`. The session takes no other status.
    PublishDone,
    /// End the stream with a FIN. The request is over.
    CloseStream,
}

/// Whether the `REQUEST_UPDATE` of a subscription with these parameters is
/// one that is accepted: `Some` when it carries nothing but delivery
/// timeouts, with the `SUBGROUP_DELIVERY_TIMEOUT` in milliseconds if it is
/// there (0 for no timeout, §8).
///
/// WHY `OBJECT_DELIVERY_TIMEOUT` is let by, which nothing here acts on: a
/// SUBSCRIBE that sets it is accepted the same way, and a parameter that
/// is taken at first and ends the subscription when it is sent again
/// would be the worse of the two.
///
/// WHY `NEW_GROUP_REQUEST` is let by as well (§10.2.13): it changes nothing
/// of the subscription, so a `REQUEST_OK` for it promises nothing that then
/// does not happen ("The Original Publisher MAY delay the
/// `NEW_GROUP_REQUEST`", and one that "does not support dynamic Groups ...
/// ignores the parameter"). The session does not know whether its track has
/// the `DYNAMIC_GROUPS` property; the caller, which set it, decides what
/// the value is for.
pub(super) fn only_the_subgroup_timeout(parameters: &[Parameter]) -> Option<AppliedUpdate> {
    let mut applied = AppliedUpdate {
        subgroup_delivery_timeout: None,
        new_group_request: None,
    };
    for parameter in parameters {
        match parameter {
            Parameter::SubgroupDeliveryTimeout(milliseconds) => {
                applied.subgroup_delivery_timeout = Some(*milliseconds);
            }
            Parameter::NewGroupRequest(group) => applied.new_group_request = Some(*group),
            Parameter::ObjectDeliveryTimeout(_) => {}
            _ => return None,
        }
    }
    Some(applied)
}

/// The `SUBSCRIBER_PRIORITY` among the parameters of a `REQUEST_UPDATE`.
fn subscriber_priority(parameters: &[Parameter]) -> Option<u8> {
    parameters.iter().find_map(|parameter| match parameter {
        Parameter::SubscriberPriority(priority) => Some(*priority),
        _ => None,
    })
}

/// What an update that is applied carries of the two parameters a caller
/// acts on.
pub(super) struct AppliedUpdate {
    pub(super) subgroup_delivery_timeout: Option<u64>,
    pub(super) new_group_request: Option<u64>,
}

/// What a later message on a request stream of the peer means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerRequestEvent {
    /// `PUBLISH_DONE` from the publisher of a track this endpoint was offered.
    /// The request is over; the Track Alias keeps draining, see
    /// [`SubscribedAlias::Draining`].
    PublishDone {
        /// The alias the PUBLISH assigned.
        track_alias: u64,
        /// The message.
        done: PublishDone,
    },
    /// `REQUEST_UPDATE` for an Established subscription of the peer's
    /// SUBSCRIBE that carries no parameter but delivery timeouts
    /// (`SUBGROUP_DELIVERY_TIMEOUT`, `OBJECT_DELIVERY_TIMEOUT`). The caller sends `response`, a `REQUEST_OK`, and acts on the
    /// timeout if it acts on that of a SUBSCRIBE.
    ///
    /// WHY one without a parameter is accepted: §10.9 has "If a parameter
    /// previously set on the request is not present in `REQUEST_UPDATE`, its
    /// value remains unchanged", so there is nothing in it to refuse, and
    /// §13.6.1 names "`REQUEST_UPDATE` with no modified Message Parameters"
    /// as a way to keep a session from idling out.
    SubscriptionUpdated {
        /// The message to send on the request stream.
        response: ControlMessage,
        /// The `SUBGROUP_DELIVERY_TIMEOUT` of the update in milliseconds
        /// (§10.2.3), 0 for no timeout (§8); `None` when the update had
        /// none, and the one before stands (§10.9).
        subgroup_delivery_timeout: Option<u64>,
        /// The `NEW_GROUP_REQUEST` of the update (§10.2.13): the largest
        /// Group ID the subscriber knows plus 1, 0 where it knows none;
        /// `None` when the update had none.
        new_group_request: Option<u64>,
    },
    /// `REQUEST_UPDATE` that this crate does not apply: one that carries any
    /// other parameter, or one for a request that is not an Established
    /// subscription of a SUBSCRIBE. The caller sends `response`, a
    /// `REQUEST_ERROR` with `NOT_SUPPORTED` (§10.9), and then does what
    /// `then` says.
    UpdateRefused {
        /// The message to send on the request stream.
        response: ControlMessage,
        /// What §10.9.1 requires next.
        then: AfterRefusal,
    },
    /// The publisher accepted a `REQUEST_UPDATE` this endpoint sent for a
    /// subscription its PUBLISH made ([`Session::update_published`]), with
    /// the parameters of its `REQUEST_OK`.
    OwnUpdateAccepted {
        /// `EXPIRES` and `LARGEST_OBJECT`, where the publisher sent them.
        parameters: Vec<Parameter>,
    },
    /// The publisher refused such a `REQUEST_UPDATE`. The subscription is
    /// still there: its `PUBLISH_DONE` with `UPDATE_FAILED` follows
    /// (§10.9.1) and ends it as any other.
    OwnUpdateFailed(RequestError),
    /// Another `REQUEST_UPDATE` after one was refused. Nothing is sent (§10.9.1).
    UpdateAlreadyRefused,
    /// `REQUEST_UPDATE` for a FETCH that was answered. The caller sends
    /// `response`, a `REQUEST_OK`, and may give the fetch stream the new
    /// `subscriber_priority`; nothing else of the FETCH changes.
    ///
    /// WHY accepted, where an update of a subscription is refused: a refusal
    /// has a price here that it does not have there. §10.9.1: "When a
    /// `REQUEST_UPDATE` fails for a FETCH, the publisher MUST reset the FETCH
    /// data stream", so a subscriber that only meant to change its priority
    /// would lose the fetch. What an update of a FETCH may carry is
    /// `SUBSCRIBER_PRIORITY` (§10.2.7), `AUTHORIZATION_TOKEN` (§10.2.2) and
    /// the two delivery timeouts (§10.2.3, §10.2.4); any other parameter
    /// closes the session (§10.2.1).
    UpdateAccepted {
        /// The message to send on the request stream.
        response: ControlMessage,
        /// The `SUBSCRIBER_PRIORITY` of the update (§10.2.7); `None` when it
        /// had none, and the one before stands (§10.9).
        subscriber_priority: Option<u8>,
    },
    /// `REQUEST_UPDATE` for a FETCH that has no answer yet. Nothing is sent
    /// now: responses carry no Request ID, so a `REQUEST_OK` sent first
    /// would be read as the answer to the FETCH. [`Session::accept_fetch`]
    /// says how many are owed.
    UpdateHeld {
        /// As in [`Self::UpdateAccepted`].
        subscriber_priority: Option<u8>,
    },
    /// A message for a request this endpoint has said its last on: a
    /// subscription it ended with `PUBLISH_DONE`, a request that was
    /// rejected, a FETCH whose answer it ended
    /// ([`Session::on_fetch_answer_ended`]). The peer sent it before it saw
    /// that; there is nothing to do. A `REQUEST_UPDATE` among them has used
    /// up its Request ID.
    LateMessage,
    /// A GOAWAY on the stream of the peer's request (§10.4): the peer means to
    /// send the request again elsewhere ("re-issue that specific request")
    /// and end this one. The request is as it was.
    GoAway(GoAway),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubscribePhase {
    Pending,
    Established {
        track_alias: u64,
    },
    /// See `PublishState::UpdateRefused` of the requester side.
    UpdateRefused {
        track_alias: u64,
    },
    /// This endpoint sent `PUBLISH_DONE`; the stream is not yet reported gone.
    Done,
}

#[derive(Debug)]
enum PeerRequestState {
    Subscribe {
        full_track_name: FullTrackName,
        phase: SubscribePhase,
    },
    Publish {
        full_track_name: FullTrackName,
        track_alias: u64,
        established: bool,
        /// `REQUEST_UPDATE`s this endpoint sent for it that have no answer
        /// yet ([`Session::update_published`]).
        updates: u32,
    },
    PublishNamespace {
        established: bool,
    },
    SubscribeNamespace {
        prefix: TrackNamespace,
        established: bool,
    },
    Fetch(FetchPhase),
    /// A `TRACK_STATUS`; `answered` once its `REQUEST_OK` was made.
    TrackStatus {
        answered: bool,
    },
    /// `REQUEST_ERROR` was sent, by the caller or by the session itself; the
    /// stream is not yet reported gone. What the peer sent before it saw
    /// the answer still arrives, and gets none. WHY kept: a `REQUEST_UPDATE`
    /// among it uses up a Request ID of the peer (§10.1), whoever refused
    /// the request and whatever kind it was; the peer counts on from there.
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchPhase {
    Pending {
        /// The Start Location, where the FETCH gives it: not for a Relative
        /// Joining FETCH.
        start: Option<Location>,
        /// The Joining Request ID of a Joining FETCH.
        joins: Option<u64>,
        /// `REQUEST_UPDATE`s that came before the answer and wait for theirs.
        held_updates: u64,
    },
    /// `FETCH_OK` was sent.
    Answered,
    /// `FETCH_OK` was sent, and this endpoint has ended its direction of
    /// the request stream since. What the peer sent before it saw that
    /// still arrives, and gets no answer, as after a rejection
    /// ([`PeerRequestState::Rejected`]).
    Over,
}

/// The requests the peer has open, and the Request IDs it has used.
#[derive(Debug)]
pub(super) struct PeerRequests {
    /// 0 or 1: the parity of the peer's Request IDs (§10.1).
    parity: u64,
    /// The lowest ID of the peer's parity that it has not used yet. Every ID
    /// below it was used.
    lowest_unseen: u64,
    /// IDs the peer used above `lowest_unseen`, while lower ones are still to
    /// come.
    seen_ahead: BTreeSet<u64>,
    live: HashMap<u64, PeerRequestState>,
    /// [`MAX_PEER_REQUESTS`], or what [`Session::set_most_peer_requests`]
    /// set.
    pub(super) most: usize,
}

impl PeerRequests {
    /// Whether the request of the peer with this ID is still there.
    pub(super) fn is_live(&self, id: u64) -> bool {
        self.live.contains_key(&id)
    }

    pub(super) fn new(own_role: Role) -> Self {
        let peer_role = match own_role {
            Role::Client => Role::Server,
            Role::Server => Role::Client,
        };
        let parity = first_request_id(peer_role);
        Self {
            parity,
            lowest_unseen: parity,
            seen_ahead: BTreeSet::new(),
            live: HashMap::new(),
            most: MAX_PEER_REQUESTS,
        }
    }

    /// §10.1: an ID of the wrong parity or a repeated one closes the session
    /// with `INVALID_REQUEST_ID`.
    /// The smallest Request ID of the peer not seen yet.
    pub(super) fn lowest_unseen(&self) -> u64 {
        self.lowest_unseen
    }

    pub(super) fn take_id(&mut self, id: u64) -> Result<(), Error> {
        let duplicate = id < self.lowest_unseen
            || self.seen_ahead.contains(&id)
            // A request that is still open was certainly seen, also when its ID
            // was pushed out of the memory above.
            || self.live.contains_key(&id);
        if id % 2 != self.parity || duplicate {
            return Err(Error::InvalidRequestId(id));
        }
        self.seen_ahead.insert(id);
        while self.seen_ahead.remove(&self.lowest_unseen) {
            // At the top of the range no further ID exists, and the comparison
            // above then refuses everything, which is right.
            self.lowest_unseen = self.lowest_unseen.saturating_add(2);
        }
        if self.seen_ahead.len() > REQUEST_IDS_AHEAD_LIMIT {
            self.seen_ahead.pop_first();
        }
        Ok(())
    }

    /// Whether the peer is the publisher of a live subscription to the track.
    pub(super) fn publishes(&self, name: &FullTrackName) -> bool {
        self.live.values().any(|state| {
            matches!(state, PeerRequestState::Publish { full_track_name, .. } if full_track_name == name)
        })
    }

    /// Whether the peer has yet to send the request with the ID `id`, as far
    /// as the request with the ID `before` shows: `id` is one of the peer's,
    /// smaller than `before`, and has not been used.
    ///
    /// WHY smaller: an endpoint gives its requests IDs that only grow
    /// (§10.1), so a request that names another was made after it. WHY this
    /// can be asked at all: requests travel on streams of their own and are
    /// read in no order, so a request can be read before one it names.
    ///
    /// An ID the peer used so long ago that this endpoint forgot it
    /// ([`REQUEST_IDS_AHEAD_LIMIT`] newer ones while a lower one was still
    /// to come) reads as one that is yet to come.
    fn yet_to_come(&self, id: u64, before: u64) -> bool {
        id % 2 == self.parity
            && id < before
            && id >= self.lowest_unseen
            && !self.seen_ahead.contains(&id)
            && !self.live.contains_key(&id)
    }

    /// The Start Location of the FETCH of the peer with the ID `request_id`,
    /// where the FETCH gives one, or the code to refuse the FETCH with.
    fn start_of(&self, request_id: u64, range: &FetchRange) -> Result<Option<Location>, u64> {
        match range {
            // §10.12.3: "End Location MUST specify the same or a
            // larger Location than Start Location". WHY a
            // REQUEST_ERROR and not the end of the session, which
            // the draft does not ask for either: the range is this
            // request's alone, and `INVALID_RANGE` is the code for
            // it (§10.6.2).
            FetchRange::Standalone { start, end, .. } => {
                if ends_before(*start, *end) {
                    return Err(request_error_code::INVALID_RANGE);
                }
                Ok(Some(*start))
            }
            // §10.12.2: "a subscription in the same session in the
            // Established or Pending (subscriber) states". The
            // definition of the code has "not an Established
            // Subscription" (§10.6.2); the section on the request is
            // followed.
            //
            // FIXME: take the Request ID of a PUBLISH of this
            // endpoint as well (§5.1 gives such a subscription a
            // Joining Location).
            FetchRange::RelativeJoining {
                joining_request_id,
                joining_start,
            }
            | FetchRange::AbsoluteJoining {
                joining_request_id,
                joining_start,
            } => {
                // WHY a SUBSCRIBE that is yet to come is taken as well:
                // the two travel on streams of their own, and a peer that
                // sends them together cannot tell which is read first. The
                // caller holds such a FETCH until the SUBSCRIBE comes or a
                // time runs out, and rejects it then
                // (`Session::peer_request_yet_to_come`).
                if !self.is_open_subscribe(*joining_request_id)
                    && !self.yet_to_come(*joining_request_id, request_id)
                {
                    return Err(request_error_code::INVALID_JOINING_REQUEST_ID);
                }
                // §10.12.2.1: {Joining Start, 0} for an Absolute
                // Joining FETCH; counted from the subscription for a
                // Relative one.
                Ok(
                    matches!(range, FetchRange::AbsoluteJoining { .. }).then_some(Location {
                        group: *joining_start,
                        object: 0,
                    }),
                )
            }
        }
    }

    /// Whether `request_id` is a SUBSCRIBE of the peer that is Pending or
    /// Established.
    fn is_open_subscribe(&self, request_id: u64) -> bool {
        matches!(
            self.live.get(&request_id),
            Some(PeerRequestState::Subscribe { phase, .. }) if *phase != SubscribePhase::Done
        )
    }

    /// Whether `request_id` is a SUBSCRIBE of the peer that was accepted and
    /// is not over.
    fn is_established_subscribe(&self, request_id: u64) -> bool {
        matches!(
            self.live.get(&request_id),
            Some(PeerRequestState::Subscribe {
                phase: SubscribePhase::Established { .. } | SubscribePhase::UpdateRefused { .. },
                ..
            })
        )
    }

    /// Whether the peer is the subscriber of a live subscription to the track.
    pub(super) fn subscribes_to(&self, name: &FullTrackName) -> bool {
        self.live.values().any(|state| {
            matches!(
                state,
                PeerRequestState::Subscribe { full_track_name, phase }
                    if full_track_name == name && *phase != SubscribePhase::Done
            )
        })
    }
}

fn request_error(error_code: u64) -> ControlMessage {
    ControlMessage::RequestError(RequestError {
        error_code,
        retry_interval: retry_interval::made_up(error_code),
        error_reason: ReasonPhrase::default(),
        redirect: None,
    })
}

/// The Request ID of the first message of a request stream.
///
/// # Errors
///
/// A protocol error for a message that is not a request (§3.3).
fn request_id_of(first: &ControlMessage) -> Result<u64, Error> {
    match first {
        ControlMessage::Subscribe(message) | ControlMessage::TrackStatus(message) => {
            Ok(message.request_id)
        }
        ControlMessage::Publish(message) => Ok(message.request_id),
        ControlMessage::PublishNamespace(message) => Ok(message.request_id),
        ControlMessage::Fetch(message) => Ok(message.request_id),
        ControlMessage::SubscribeNamespace(message) => Ok(message.request_id),
        ControlMessage::UnsupportedRequest { request_id, .. } => Ok(*request_id),
        _ => Err(Error::ProtocolViolation(
            "request stream did not begin with a request",
        )),
    }
}

const NOT_PENDING: UsageError = UsageError("the request is not waiting for a response");

impl Session {
    /// Refuses a request of the peer whose Request ID was taken, and keeps it
    /// as rejected until its stream is reported gone.
    fn refuse_opened(&mut self, request: PeerRequestId, error_code: u64) -> PeerRequestOpened {
        let kept = self.peer.live.len() < self.peer.most;
        if kept {
            self.peer
                .live
                .insert(request.id, PeerRequestState::Rejected);
        }
        PeerRequestOpened::Refused {
            response: request_error(error_code),
            request: kept.then_some(request),
        }
    }

    /// Turns a request of the peer into one that was rejected: what it held
    /// is let go of, and the request stays until its stream is reported
    /// gone.
    fn reject_peer_request(&mut self, request_id: u64) {
        if let Some(state) = self.peer.live.get_mut(&request_id) {
            let was = std::mem::replace(state, PeerRequestState::Rejected);
            self.forget_peer_request(&was);
        }
    }

    /// Feeds the first message of a bidirectional stream the peer opened.
    ///
    /// # Errors
    ///
    /// A protocol error when the message is not a request (§3.3), when its
    /// Request ID is invalid (§10.1), or when a PUBLISH reuses the Track Alias
    /// of another track (§11.1). The session then has to be closed with
    /// [`Error::termination_code`].
    pub fn on_peer_request_opened(
        &mut self,
        first: ControlMessage,
    ) -> Result<PeerRequestOpened, Error> {
        let request_id = request_id_of(&first)?;
        self.peer.take_id(request_id)?;
        let id = PeerRequestId {
            session: self.tag,
            id: request_id,
        };
        // §10.4: "Requests with a Request ID equal to or greater than the
        // indicated value, as well as any requests that arrive after the
        // GOAWAY, MUST be rejected with REQUEST_ERROR using error code
        // GOING_AWAY." Every request read now came after it.
        if self.go_aways.sent {
            return Ok(self.refuse_opened(id, request_error_code::GOING_AWAY));
        }

        let (state, request) = match first {
            ControlMessage::Subscribe(message) => {
                // §5.1: at most one subscription to a track in each role.
                if self.is_publishing(&message.full_track_name) {
                    return Ok(self.refuse_opened(id, request_error_code::DUPLICATE_SUBSCRIPTION));
                }
                let state = PeerRequestState::Subscribe {
                    full_track_name: message.full_track_name.clone(),
                    phase: SubscribePhase::Pending,
                };
                (state, PeerRequest::Subscribe(message))
            }
            ControlMessage::Publish(message) => {
                // §11.1, checked first: an error that ends the session outranks
                // one that ends the request.
                if let SubscribedAlias::Established(name) =
                    self.subscribed_aliases.state(message.track_alias)
                    && *name != message.full_track_name
                {
                    return Err(Error::DuplicateTrackAlias(message.track_alias));
                }
                // §5.1: at most one subscription to a track as the subscriber.
                // WHY a SUBSCRIBE of this endpoint that has no answer yet does
                // not count: §5.1 has it give way instead, "before sending
                // PUBLISH_OK"; see `accept_publish`. Refusing here would turn
                // away a publisher whose PUBLISH crosses a SUBSCRIBE for the
                // same track.
                if self.is_subscribed_to(&message.full_track_name)
                    && self
                        .pending_subscribe_to(&message.full_track_name)
                        .is_none()
                {
                    return Ok(self.refuse_opened(id, request_error_code::DUPLICATE_SUBSCRIPTION));
                }
                // §2.5.1: this crate understands no Mandatory Track Property,
                // outside or inside Immutable Properties (§12.7).
                //
                // A malformed Track (§12.7) is refused with MALFORMED_TRACK,
                // which §10.6 names "In response to a FETCH" of a relay
                // publisher alone. WHY it all the same (decided on
                // 2026-10-24): the publisher is told why, and §2.4.2 has the
                // subscriber of a malformed Track "cancel any corresponding
                // subscription", which a refusal of the PUBLISH comes
                // closest to. WHY NOT INTERNAL_ERROR: it does not say why.
                // WHY NOT UNINTERESTED, the one §10.6 lists for a subscriber:
                // what it says is not what happened.
                match track_properties_refusal(&message.track_properties) {
                    Some(TrackRefusal::Malformed) => {
                        return Ok(self.refuse_opened(id, request_error_code::MALFORMED_TRACK));
                    }
                    Some(TrackRefusal::Unsupported(_)) => {
                        return Ok(
                            self.refuse_opened(id, request_error_code::UNSUPPORTED_EXTENSION)
                        );
                    }
                    None => {}
                }
                // The alias means the track before the answer: §10.10 lets the
                // publisher send Objects "possibly before PUBLISH_OK".
                self.subscribed_aliases
                    .offer(message.track_alias, message.full_track_name.clone());
                let state = PeerRequestState::Publish {
                    full_track_name: message.full_track_name.clone(),
                    track_alias: message.track_alias,
                    established: false,
                    updates: 0,
                };
                (state, PeerRequest::Publish(message))
            }
            ControlMessage::PublishNamespace(message) => (
                PeerRequestState::PublishNamespace { established: false },
                PeerRequest::PublishNamespace(message),
            ),
            ControlMessage::SubscribeNamespace(message) => match self.namespace_state(&message) {
                Ok(state) => (state, PeerRequest::SubscribeNamespace(message)),
                Err(error_code) => return Ok(self.refuse_opened(id, error_code)),
            },
            ControlMessage::Fetch(message) => {
                let start = match self.peer.start_of(request_id, &message.range) {
                    Ok(start) => start,
                    Err(error_code) => return Ok(self.refuse_opened(id, error_code)),
                };
                let joins = match &message.range {
                    FetchRange::Standalone { .. } => None,
                    FetchRange::RelativeJoining {
                        joining_request_id, ..
                    }
                    | FetchRange::AbsoluteJoining {
                        joining_request_id, ..
                    } => Some(*joining_request_id),
                };
                let state = PeerRequestState::Fetch(FetchPhase::Pending {
                    start,
                    joins,
                    held_updates: 0,
                });
                (state, PeerRequest::Fetch(message))
            }
            ControlMessage::TrackStatus(message) => (
                PeerRequestState::TrackStatus { answered: false },
                PeerRequest::TrackStatus(message),
            ),
            // The types `request_id` was read from above, less the ones handled.
            _ => return Ok(self.refuse_opened(id, request_error_code::NOT_SUPPORTED)),
        };
        // The one refusal that keeps nothing: the table is full.
        if self.peer.live.len() >= self.peer.most {
            self.forget_peer_request(&state);
            return Ok(PeerRequestOpened::Refused {
                response: request_error(request_error_code::EXCESSIVE_LOAD),
                request: None,
            });
        }
        self.peer.live.insert(request_id, state);
        Ok(PeerRequestOpened::Request(id, request))
    }

    /// Accepts a SUBSCRIBE of the peer. Returns the Track Alias this endpoint
    /// will use for the track on its data streams, and the `SUBSCRIBE_OK` to
    /// send.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a SUBSCRIBE waiting for its response, and when
    /// a parameter may not appear in `SUBSCRIBE_OK`.
    pub fn accept_subscribe(
        &mut self,
        request: PeerRequestId,
        parameters: Vec<Parameter>,
        track_properties: Vec<KeyValuePair>,
    ) -> Result<(u64, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::SubscribeOk)?;
        // Before an alias is taken: a call that fails uses none up.
        let pending = matches!(
            self.peer_request_mut(request)?,
            Some(PeerRequestState::Subscribe {
                phase: SubscribePhase::Pending,
                ..
            })
        );
        if !pending {
            return Err(NOT_PENDING);
        }
        let track_alias = self.next_free_alias()?;
        if let Some(PeerRequestState::Subscribe { phase, .. }) = self.peer_request_mut(request)? {
            *phase = SubscribePhase::Established { track_alias };
        }
        self.published_aliases.insert(track_alias);
        let message = ControlMessage::SubscribeOk(SubscribeOk {
            track_alias,
            parameters,
            track_properties,
        });
        Ok((track_alias, message))
    }

    /// Accepts a PUBLISH of the peer. Returns the `PUBLISH_OK` to send, and
    /// the SUBSCRIBE of this endpoint for the same track that had no answer
    /// yet, if there was one.
    ///
    /// That SUBSCRIBE is over when this returns (§5.1: "If a subscriber
    /// receives a PUBLISH for a Track with a subscription in the Pending
    /// (Subscriber) state, it MUST ensure the subscription it initiated
    /// transitions to the Terminated state before sending `PUBLISH_OK`"). The
    /// caller cancels its stream as after [`Session::cancel`], and then sends
    /// the `PUBLISH_OK`.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a PUBLISH waiting for its response, when a
    /// parameter may not appear in `PUBLISH_OK`, and when a SUBSCRIBE of this
    /// endpoint for the track was accepted since the PUBLISH came: the PUBLISH
    /// is then to be rejected with `DUPLICATE_SUBSCRIPTION` (§5.1).
    pub fn accept_publish(
        &mut self,
        request: PeerRequestId,
        parameters: Vec<Parameter>,
    ) -> Result<(ControlMessage, Option<RequestId>), UsageError> {
        check_own_parameters(&parameters, MessageKind::PublishOk)?;
        let Some(PeerRequestState::Publish {
            established: false,
            full_track_name,
            ..
        }) = self.peer_request_mut(request)?
        else {
            return Err(NOT_PENDING);
        };
        let full_track_name = full_track_name.clone();
        let gives_way = self.pending_subscribe_to(&full_track_name);
        let subscribed = self.requests.values().any(|own| {
            matches!(own, Request::Subscribe { full_track_name: name, .. } if *name == full_track_name)
        });
        if subscribed && gives_way.is_none() {
            return Err(UsageError("already a subscriber of this track"));
        }
        if let Some(own) = gives_way {
            self.cancel(own);
        }
        if let Some(PeerRequestState::Publish {
            established,
            track_alias,
            ..
        }) = self.peer_request_mut(request)?
        {
            *established = true;
            let track_alias = *track_alias;
            self.subscribed_aliases.accept_offer(track_alias);
        }
        let ok = ControlMessage::RequestOk(RequestOk {
            parameters,
            track_properties: Vec::new(),
        });
        Ok((ok, gives_way))
    }

    /// Accepts a `PUBLISH_NAMESPACE` of the peer. Returns the
    /// `PUBLISH_NAMESPACE_OK` to send.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a `PUBLISH_NAMESPACE` waiting for its response.
    pub fn accept_publish_namespace(
        &mut self,
        request: PeerRequestId,
    ) -> Result<ControlMessage, UsageError> {
        let Some(PeerRequestState::PublishNamespace { established }) =
            self.peer_request_mut(request)?
        else {
            return Err(NOT_PENDING);
        };
        if *established {
            return Err(NOT_PENDING);
        }
        *established = true;
        Ok(ControlMessage::RequestOk(RequestOk::default()))
    }

    /// Answers a `TRACK_STATUS` of the peer with the `REQUEST_OK` to send
    /// (`TRACK_STATUS_OK`): "the same parameters and Track Properties it would
    /// have set in a `SUBSCRIBE_OK`" (§10.14), of which `LARGEST_OBJECT` alone
    /// may appear in it (§10.2.11). The caller closes the stream with a FIN
    /// after it: "The bidi stream is closed with a FIN after `TRACK_STATUS_OK`
    /// or `REQUEST_ERROR` are sent."
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a `TRACK_STATUS` without an answer, and for
    /// a parameter other than `LARGEST_OBJECT`.
    pub fn accept_track_status(
        &mut self,
        request: PeerRequestId,
        parameters: Vec<Parameter>,
        track_properties: Vec<KeyValuePair>,
    ) -> Result<ControlMessage, UsageError> {
        if !params::all_allowed(&parameters, MessageKind::TrackStatusOk) {
            return Err(UsageError("parameter not allowed in this message"));
        }
        let Some(PeerRequestState::TrackStatus { answered }) = self.peer_request_mut(request)?
        else {
            return Err(NOT_PENDING);
        };
        if *answered {
            return Err(NOT_PENDING);
        }
        *answered = true;
        Ok(ControlMessage::RequestOk(RequestOk {
            parameters,
            track_properties,
        }))
    }

    /// Accepts a FETCH of the peer. Returns the `FETCH_OK` to send, and the
    /// number of `REQUEST_UPDATE`s that came before it: the caller sends
    /// that many `REQUEST_OK` messages (`RequestOk::default()`) after the
    /// `FETCH_OK`. The fetch stream may be opened before or after (§10.13).
    ///
    /// `end_location` is in the encoding of a FETCH: the last Object plus 1,
    /// or Object 0 for the whole Group. No parameter is taken: none of §10.2
    /// may appear in `FETCH_OK`.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a FETCH waiting for its response, and for an
    /// End Location before the Start Location of the FETCH, where the FETCH
    /// gives one: the receiver of such a `FETCH_OK` closes the session
    /// (§10.13). Fails for a Joining FETCH whose SUBSCRIBE is not accepted:
    /// while that is Pending the caller holds the FETCH (§10.12.2: "buffers
    /// the pending Joining Fetch until ... the Subscription is established"),
    /// and when it is over the caller rejects the FETCH.
    pub fn accept_fetch(
        &mut self,
        request: PeerRequestId,
        end_of_track: bool,
        end_location: Location,
        track_properties: Vec<KeyValuePair>,
    ) -> Result<(ControlMessage, u64), UsageError> {
        let Some(PeerRequestState::Fetch(phase)) = self.peer_request_mut(request)? else {
            return Err(NOT_PENDING);
        };
        let FetchPhase::Pending {
            start,
            joins,
            held_updates: owed,
        } = *phase
        else {
            return Err(NOT_PENDING);
        };
        if start.is_some_and(|start| ends_before(start, end_location)) {
            return Err(UsageError("the FETCH_OK ends before the FETCH starts"));
        }
        if joins.is_some_and(|joined| !self.peer.is_established_subscribe(joined)) {
            return Err(UsageError(
                "the SUBSCRIBE of the Joining FETCH is not established",
            ));
        }
        if let Some(PeerRequestState::Fetch(phase)) = self.peer_request_mut(request)? {
            *phase = FetchPhase::Answered;
        }
        let ok = ControlMessage::FetchOk(FetchOk {
            end_of_track,
            end_location,
            parameters: Vec::new(),
            track_properties,
        });
        Ok((ok, owed))
    }

    /// Rejects a request of the peer that is waiting for its response. Returns
    /// the `REQUEST_ERROR` to send; the caller then ends the stream with a FIN
    /// (§3.3.2). The request is over. For a FETCH the caller also resets the
    /// fetch stream if it opened one, and `REQUEST_UPDATE`s that came before
    /// get no answer: their request is gone. The ones that still arrive
    /// yield [`PeerRequestEvent::LateMessage`] until
    /// [`Self::on_peer_request_closed`] reports the stream gone.
    ///
    /// `error_code` is one of [`request_error_code`], other than REDIRECT.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is waiting for its response, and for the code
    /// REDIRECT, which needs a Redirect this method does not take.
    pub fn reject(
        &mut self,
        request: PeerRequestId,
        error_code: u64,
        retry_interval: u64,
        error_reason: ReasonPhrase,
    ) -> Result<ControlMessage, UsageError> {
        if error_code == request_error_code::REDIRECT {
            return Err(UsageError("a REDIRECT needs a Redirect"));
        }
        let pending = match self.peer_request_mut(request)? {
            Some(PeerRequestState::Subscribe { phase, .. }) => *phase == SubscribePhase::Pending,
            Some(
                PeerRequestState::Publish { established, .. }
                | PeerRequestState::PublishNamespace { established }
                | PeerRequestState::SubscribeNamespace { established, .. },
            ) => !*established,
            Some(PeerRequestState::Fetch(phase)) => {
                matches!(phase, FetchPhase::Pending { .. })
            }
            Some(PeerRequestState::TrackStatus { answered }) => !*answered,
            Some(PeerRequestState::Rejected) | None => false,
        };
        if !pending {
            return Err(NOT_PENDING);
        }
        // WHY the request stays: what the peer sent before it saw the
        // answer still arrives, and a REQUEST_UPDATE among it uses up a
        // Request ID (§10.1). One for a request that is gone would close
        // the session, and one that is not seen would leave this endpoint
        // behind the peer's count.
        self.reject_peer_request(request.id);
        Ok(ControlMessage::RequestError(RequestError {
            error_code,
            retry_interval,
            error_reason,
            redirect: None,
        }))
    }

    /// The state a `SUBSCRIBE_NAMESPACE` of the peer begins in, or the code it
    /// is refused with. §10.18: "if a publisher receives a
    /// `SUBSCRIBE_NAMESPACE` with a Track Namespace Prefix that shares a common
    /// prefix with an established `SUBSCRIBE_NAMESPACE`, it MUST respond with
    /// `REQUEST_ERROR` with error code `PREFIX_OVERLAP`". WHY one that waits
    /// for its answer counts as well: two that came at once would otherwise
    /// both be accepted, and overlap.
    fn namespace_state(&self, message: &SubscribeNamespace) -> Result<PeerRequestState, u64> {
        let prefix = &message.track_namespace_prefix;
        let overlaps = self.peer.live.values().any(|state| {
            matches!(
                state,
                PeerRequestState::SubscribeNamespace { prefix: other, .. }
                    if other.is_prefix_of(prefix) || prefix.is_prefix_of(other)
            )
        });
        if overlaps {
            return Err(request_error_code::PREFIX_OVERLAP);
        }
        Ok(PeerRequestState::SubscribeNamespace {
            prefix: prefix.clone(),
            established: false,
        })
    }

    /// Accepts a `SUBSCRIBE_NAMESPACE` of the peer and returns its
    /// `REQUEST_OK` (§6.1: "the publisher MUST send a single `REQUEST_OK` or
    /// `REQUEST_ERROR` as the first message on the bidirectional stream"). What
    /// follows on the stream, the NAMESPACE and `NAMESPACE_DONE` messages, is
    /// the caller's: it knows the namespaces.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a `SUBSCRIBE_NAMESPACE` that waits for its
    /// answer.
    pub fn accept_subscribe_namespace(
        &mut self,
        request: PeerRequestId,
    ) -> Result<ControlMessage, UsageError> {
        let Some(PeerRequestState::SubscribeNamespace { established, .. }) =
            self.peer_request_mut(request)?
        else {
            return Err(UsageError("not a SUBSCRIBE_NAMESPACE of the peer"));
        };
        if *established {
            return Err(NOT_PENDING);
        }
        *established = true;
        Ok(ControlMessage::RequestOk(RequestOk::default()))
    }

    /// Takes the first message of a request the peer opened and refuses the
    /// request, for an endpoint that serves none: with `NOT_SUPPORTED` (§10.6.2:
    /// "The endpoint does not support the type of request") where the session
    /// hands it over, or with the session's own refusal. Returns the
    /// `REQUEST_ERROR` to send before a FIN (§3.3.2), and the request where
    /// the session keeps it, which is fed what still comes on its stream
    /// ([`Self::on_peer_request_message`]) and reported gone
    /// ([`Self::on_peer_request_closed`]), as one rejected with [`Self::reject`].
    ///
    /// # Errors
    ///
    /// What [`Self::on_peer_request_opened`] fails with: a Request ID §10.1
    /// does not allow, among others. The session then has to be closed with
    /// [`Error::termination_code`].
    pub fn refuse_peer_request(
        &mut self,
        first: ControlMessage,
    ) -> Result<(ControlMessage, Option<PeerRequestId>), Error> {
        match self.on_peer_request_opened(first)? {
            PeerRequestOpened::Refused { response, request } => Ok((response, request)),
            PeerRequestOpened::Request(request, _) => {
                // A request the session has just handed over waits for its
                // answer, which is all `reject` asks.
                self.reject_peer_request(request.id);
                let response = ControlMessage::RequestError(RequestError {
                    error_code: request_error_code::NOT_SUPPORTED,
                    retry_interval: retry_interval::made_up(request_error_code::NOT_SUPPORTED),
                    error_reason: ReasonPhrase::default(),
                    redirect: None,
                });
                Ok((response, Some(request)))
            }
        }
    }

    /// Asks the publisher to change parameters of an Established
    /// subscription its PUBLISH made (§10.9: "A subscriber can also send
    /// `REQUEST_UPDATE` to modify parameters of a subscription established
    /// with PUBLISH."). Returns the
    /// `REQUEST_UPDATE` to send on the stream of that PUBLISH. The answer
    /// comes as [`PeerRequestEvent::OwnUpdateAccepted`] or
    /// [`PeerRequestEvent::OwnUpdateFailed`], in the order the updates were
    /// sent.
    ///
    /// # Errors
    ///
    /// Fails for a request that is not a PUBLISH this endpoint accepted,
    /// when a parameter may not appear in `REQUEST_UPDATE`, when
    /// [`MAX_UNANSWERED_UPDATES`] updates wait for their answer, and when
    /// the Request IDs are used up (§10.1: an update consumes one).
    pub fn update_published(
        &mut self,
        request: PeerRequestId,
        parameters: Vec<Parameter>,
    ) -> Result<ControlMessage, UsageError> {
        check_own_parameters(&parameters, MessageKind::RequestUpdate)?;
        let request_id = self.next_request_id;
        let Some(PeerRequestState::Publish {
            established: true,
            updates,
            ..
        }) = self.peer_request_mut(request)?
        else {
            return Err(UsageError("not an established subscription of a PUBLISH"));
        };
        if *updates >= MAX_UNANSWERED_UPDATES {
            return Err(UsageError("too many updates wait for their answer"));
        }
        let next = request_id
            .checked_add(2)
            .ok_or(UsageError("request ids are exhausted"))?;
        *updates += 1;
        self.next_request_id = next;
        Ok(ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters,
        }))
    }

    /// Feeds a message read from a request stream of the peer, after the first.
    ///
    /// # Errors
    ///
    /// [`RequestFault::Peer`] means the peer broke the protocol and the session
    /// has to be closed with [`Error::termination_code`].
    /// [`RequestFault::Usage`] means `request` belongs to another session.
    pub fn on_peer_request_message(
        &mut self,
        request: PeerRequestId,
        message: ControlMessage,
    ) -> Result<PeerRequestEvent, RequestFault> {
        if let ControlMessage::GoAway(go_away) = message {
            if self.peer_request_mut(request)?.is_none() {
                return Err(Error::ProtocolViolation("message after the request ended").into());
            }
            self.check_request_go_away(request.id, &go_away, false)?;
            return Ok(PeerRequestEvent::GoAway(go_away));
        }
        let Some(state) = self.peer_request_mut(request)? else {
            return Err(Error::ProtocolViolation("message after the request ended").into());
        };
        let not_allowed = Error::ProtocolViolation("message not allowed in this request state");
        // §5.1: the publisher may end a subscription while the subscriber's
        // messages are still on their way.
        let ended = matches!(
            state,
            PeerRequestState::Subscribe {
                phase: SubscribePhase::Done,
                ..
            } | PeerRequestState::Rejected
                | PeerRequestState::Fetch(FetchPhase::Over)
        );
        // §10.2.1, with what §10.2 lets the update of this request carry: the
        // decoder let through what the update of any request may carry.
        let update_kind = match state {
            PeerRequestState::Fetch(_) => MessageKind::FetchUpdate,
            PeerRequestState::SubscribeNamespace { .. } => MessageKind::NamespaceUpdate,
            _ => MessageKind::RequestUpdate,
        };
        // §10.1: a REQUEST_UPDATE consumes a Request ID of its sender, also
        // when the message itself is ignored below.
        if let ControlMessage::RequestUpdate(update) = &message {
            self.peer.take_id(update.request_id)?;
        }
        if ended {
            return Ok(PeerRequestEvent::LateMessage);
        }
        // §10.14: "the subscriber cannot send REQUEST_UPDATE". WHY the
        // session is closed, as for any message a request of its kind does
        // not take: the draft names no consequence, and an update of a
        // request that creates no state has nothing to apply to.
        if matches!(message, ControlMessage::RequestUpdate(_))
            && matches!(
                self.peer_request_mut(request)?,
                Some(PeerRequestState::TrackStatus { .. })
            )
        {
            return Err(not_allowed.into());
        }
        match message {
            ControlMessage::RequestUpdate(update)
                if !params::all_allowed(&update.parameters, update_kind) =>
            {
                Err(Error::ProtocolViolation("parameter not allowed in this message").into())
            }
            ControlMessage::RequestUpdate(update) => {
                Ok(self.answer_update(request.id, &update.parameters))
            }
            // §10.9: "The receiver of a REQUEST_UPDATE MUST respond with
            // exactly one REQUEST_OK or REQUEST_ERROR", on the stream of
            // the PUBLISH the update was sent on.
            answer @ (ControlMessage::RequestOk(_) | ControlMessage::RequestError(_)) => {
                let Some(PeerRequestState::Publish {
                    established: true,
                    updates,
                    ..
                }) = self.peer.live.get_mut(&request.id)
                else {
                    return Err(not_allowed.into());
                };
                if *updates == 0 {
                    return Err(not_allowed.into());
                }
                match super::update_answered(updates, answer)? {
                    RequestEvent::UpdateFailed(error) => {
                        Ok(PeerRequestEvent::OwnUpdateFailed(error))
                    }
                    RequestEvent::UpdateAccepted { parameters } => {
                        Ok(PeerRequestEvent::OwnUpdateAccepted { parameters })
                    }
                    _ => Err(not_allowed.into()),
                }
            }
            ControlMessage::PublishDone(done) => {
                let Some(PeerRequestState::Publish {
                    track_alias,
                    established,
                    ..
                }) = self.peer.live.get(&request.id)
                else {
                    return Err(not_allowed.into());
                };
                let (track_alias, established) = (*track_alias, *established);
                self.peer.live.remove(&request.id);
                if established {
                    self.subscribed_aliases
                        .drain(track_alias, done.stream_count);
                } else {
                    self.subscribed_aliases
                        .offer_withdrawn(track_alias, done.stream_count);
                }
                Ok(PeerRequestEvent::PublishDone { track_alias, done })
            }
            _ => Err(not_allowed.into()),
        }
    }

    /// The `PUBLISH_DONE` that ends a subscription the peer opened with
    /// SUBSCRIBE. The caller sends it after closing every data stream of the
    /// subscription, then ends the request stream with a FIN (§10.11).
    ///
    /// Messages that still arrive for the request yield
    /// [`PeerRequestEvent::LateMessage`] until
    /// [`Self::on_peer_request_closed`] reports the stream gone.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is an accepted SUBSCRIBE, and for a status other
    /// than `UPDATE_FAILED` once a `REQUEST_UPDATE` was refused (§10.9.1).
    pub fn subscribe_done(
        &mut self,
        request: PeerRequestId,
        status_code: u64,
        stream_count: u64,
        error_reason: ReasonPhrase,
    ) -> Result<ControlMessage, UsageError> {
        let Some(PeerRequestState::Subscribe { phase, .. }) = self.peer_request_mut(request)?
        else {
            return Err(UsageError("PUBLISH_DONE ends an accepted SUBSCRIBE"));
        };
        let track_alias = match *phase {
            SubscribePhase::Established { track_alias } => track_alias,
            SubscribePhase::UpdateRefused { track_alias } => {
                if status_code != publish_done_status::UPDATE_FAILED {
                    return Err(UsageError(
                        "a refused REQUEST_UPDATE ends the subscription with UPDATE_FAILED",
                    ));
                }
                track_alias
            }
            SubscribePhase::Pending | SubscribePhase::Done => {
                return Err(UsageError("PUBLISH_DONE ends an accepted SUBSCRIBE"));
            }
        };
        *phase = SubscribePhase::Done;
        self.published_aliases.remove(&track_alias);
        Ok(ControlMessage::PublishDone(PublishDone {
            status_code,
            stream_count,
            error_reason,
        }))
    }

    /// Whether the peer has yet to send the request with the Request ID
    /// `request_id`, which a request it sent later names: `request` is that
    /// later request, a Joining FETCH whose Joining Request ID this is.
    ///
    /// `true` for an ID of the peer's that is smaller than the one of
    /// `request` and has not been used. The caller holds the FETCH then, for
    /// as long as it gives a stream of the peer to say what it is, and
    /// rejects it with `INVALID_JOINING_REQUEST_ID` when the ID has been
    /// used by something that is no SUBSCRIBE, or not at all in that time.
    /// The contract is the table `fixtures/joining-request-id/`.
    #[must_use]
    pub fn peer_request_yet_to_come(&self, request_id: u64, request: PeerRequestId) -> bool {
        request.session == self.tag && self.peer.yet_to_come(request_id, request.id)
    }

    /// Records that the answer to a FETCH this endpoint accepted with
    /// `FETCH_OK` has ended: its fetch stream was finished or reset, or none
    /// comes. From here on a message of the peer on the request stream gets
    /// no answer, and a `REQUEST_UPDATE` among them uses up its Request ID
    /// (§10.1). Does nothing for any other request.
    ///
    /// The endpoint may keep its direction of the request stream open for a
    /// while after that, for the peer to end its own first; nothing is
    /// answered on it all the same.
    ///
    /// WHY the caller has to say so: the streams are the caller's, and a
    /// `REQUEST_UPDATE` for a FETCH that is answered gets a `REQUEST_OK`
    /// only while the FETCH is being answered. WHY no parameter of
    /// such a late `REQUEST_UPDATE` closes the session, where one does
    /// while the FETCH is served: the peer sent it before it saw the end,
    /// and after a rejection the same message is not looked at either.
    pub fn on_fetch_answer_ended(&mut self, request: PeerRequestId) {
        if request.session != self.tag {
            return;
        }
        if let Some(PeerRequestState::Fetch(phase @ FetchPhase::Answered)) =
            self.peer.live.get_mut(&request.id)
        {
            *phase = FetchPhase::Over;
        }
    }

    /// Records that the peer's direction of a request stream is gone:
    /// finished, reset, stopped by this endpoint, or lost with the
    /// connection (§3.3.2). Call it for every stream that led to
    /// [`PeerRequestOpened::Request`], and for every one whose
    /// [`PeerRequestOpened::Refused`] names a request, or the request stays
    /// held in this session and counts against what the peer may have open:
    /// a request that was rejected, ended with `PUBLISH_DONE` or answered is
    /// kept until then, for what the peer still sends on its stream. Does
    /// nothing for a request that is already forgotten, or for a handle of
    /// another session.
    pub fn on_peer_request_closed(&mut self, request: PeerRequestId) {
        if request.session == self.tag {
            self.end_peer_request(request.id);
        }
    }

    /// Requests of the peer this session holds state for: open ones, and ones
    /// this endpoint ended whose stream is not yet reported gone.
    #[must_use]
    pub fn peer_request_count(&self) -> usize {
        self.peer.live.len()
    }

    /// The Track Alias of an accepted SUBSCRIBE of the peer, while the
    /// subscription is live.
    #[must_use]
    pub fn peer_subscription_alias(&self, request: PeerRequestId) -> Option<u64> {
        if request.session != self.tag {
            return None;
        }
        match self.peer.live.get(&request.id) {
            Some(PeerRequestState::Subscribe {
                phase:
                    SubscribePhase::Established { track_alias }
                    | SubscribePhase::UpdateRefused { track_alias },
                ..
            }) => Some(*track_alias),
            _ => None,
        }
    }

    fn answer_update(&mut self, request_id: u64, parameters: &[Parameter]) -> PeerRequestEvent {
        let refused = |then| PeerRequestEvent::UpdateRefused {
            response: request_error(request_error_code::NOT_SUPPORTED),
            then,
        };
        match self.peer.live.get_mut(&request_id) {
            Some(PeerRequestState::Fetch(FetchPhase::Answered)) => {
                PeerRequestEvent::UpdateAccepted {
                    response: ControlMessage::RequestOk(RequestOk::default()),
                    subscriber_priority: subscriber_priority(parameters),
                }
            }
            Some(PeerRequestState::Fetch(FetchPhase::Pending { held_updates, .. })) => {
                // Saturating: every update used up a Request ID, of which
                // there are fewer than this counts.
                *held_updates = held_updates.saturating_add(1);
                PeerRequestEvent::UpdateHeld {
                    subscriber_priority: subscriber_priority(parameters),
                }
            }
            // Sorted out before this is called.
            Some(
                PeerRequestState::Rejected
                | PeerRequestState::Fetch(FetchPhase::Over)
                | PeerRequestState::TrackStatus { .. },
            ) => PeerRequestEvent::LateMessage,
            Some(PeerRequestState::Subscribe { phase, .. }) => match *phase {
                // The one update that is applied. WHY the others are
                // refused and not accepted without effect: a REQUEST_OK for
                // a filter or a Forward State that then does not change
                // would have the subscriber wait for Objects that do not
                // come, or pay for ones it asked to stop.
                SubscribePhase::Established { track_alias } => {
                    if let Some(applied) = only_the_subgroup_timeout(parameters) {
                        PeerRequestEvent::SubscriptionUpdated {
                            response: ControlMessage::RequestOk(RequestOk::default()),
                            subgroup_delivery_timeout: applied.subgroup_delivery_timeout,
                            new_group_request: applied.new_group_request,
                        }
                    } else {
                        *phase = SubscribePhase::UpdateRefused { track_alias };
                        refused(AfterRefusal::PublishDone)
                    }
                }
                SubscribePhase::UpdateRefused { .. } => PeerRequestEvent::UpdateAlreadyRefused,
                // WHY end the request: responses carry no Request ID, so a
                // REQUEST_ERROR sent before the response to the request itself
                // can only be read as that response.
                SubscribePhase::Pending | SubscribePhase::Done => {
                    self.reject_peer_request(request_id);
                    refused(AfterRefusal::CloseStream)
                }
            },
            // The peer is the publisher: §10.9.1 has it end the subscription.
            Some(PeerRequestState::Publish {
                established: true, ..
            }) => refused(AfterRefusal::Nothing),
            // §10.9.1: "When a REQUEST_UPDATE fails for a ... PUBLISH_NAMESPACE,
            // the responder MUST close the bidi stream." A request that is not
            // answered yet ends the same way, for the reason given above.
            Some(
                PeerRequestState::Publish { .. }
                | PeerRequestState::PublishNamespace { .. }
                | PeerRequestState::SubscribeNamespace { .. },
            )
            | None => {
                self.reject_peer_request(request_id);
                refused(AfterRefusal::CloseStream)
            }
        }
    }

    fn peer_request_mut(
        &mut self,
        request: PeerRequestId,
    ) -> Result<Option<&mut PeerRequestState>, UsageError> {
        if request.session != self.tag {
            return Err(UsageError("the request belongs to another session"));
        }
        Ok(self.peer.live.get_mut(&request.id))
    }

    fn next_free_alias(&mut self) -> Result<u64, UsageError> {
        let exhausted = UsageError("track aliases are exhausted");
        while self.published_aliases.contains(&self.next_published_alias) {
            self.next_published_alias =
                self.next_published_alias.checked_add(1).ok_or(exhausted)?;
        }
        let alias = self.next_published_alias;
        self.next_published_alias = alias.checked_add(1).ok_or(exhausted)?;
        Ok(alias)
    }

    fn end_peer_request(&mut self, request_id: u64) {
        if let Some(state) = self.peer.live.remove(&request_id) {
            self.forget_peer_request(&state);
        }
    }

    /// Frees what a request of the peer held outside the request table.
    fn forget_peer_request(&mut self, state: &PeerRequestState) {
        match state {
            PeerRequestState::Subscribe {
                phase:
                    SubscribePhase::Established { track_alias }
                    | SubscribePhase::UpdateRefused { track_alias },
                ..
            } => {
                self.published_aliases.remove(track_alias);
            }
            // The subscription is over on this side, so late Objects under the
            // alias are unwanted (§11.1).
            PeerRequestState::Publish {
                track_alias,
                established: true,
                ..
            } => {
                self.subscribed_aliases.retire(*track_alias);
            }
            // The PUBLISH was never accepted, so it ends nothing but itself.
            PeerRequestState::Publish {
                track_alias,
                established: false,
                ..
            } => {
                self.subscribed_aliases.refuse_offer(*track_alias);
            }
            PeerRequestState::Subscribe { .. }
            | PeerRequestState::PublishNamespace { .. }
            | PeerRequestState::SubscribeNamespace { .. }
            | PeerRequestState::Fetch(_)
            | PeerRequestState::TrackStatus { .. }
            | PeerRequestState::Rejected => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_for_too_many_requests_says_on_the_wire_to_ask_again_after_a_second() {
        // REQUEST_ERROR (0x5), 4 bytes: EXCESSIVE_LOAD (0x9), the Retry
        // Interval 1,001 (0x3e9, in two bytes), and an empty reason.
        let mut bytes = Vec::new();
        request_error(request_error_code::EXCESSIVE_LOAD)
            .encode(&mut bytes)
            .unwrap();
        assert_eq!(bytes, [0x05, 0x00, 0x04, 0x09, 0x83, 0xe9, 0x00]);
        // And 0 with a code after which the same request gets the same
        // answer: NOT_SUPPORTED (0x3).
        let mut bytes = Vec::new();
        request_error(request_error_code::NOT_SUPPORTED)
            .encode(&mut bytes)
            .unwrap();
        assert_eq!(bytes, [0x05, 0x00, 0x03, 0x03, 0x00, 0x00]);
    }
    use crate::message::RequestUpdate;
    use crate::name::TrackNamespace;
    use crate::session::RequestState;
    use crate::session::Transport;
    use crate::test_util::{REQUEST_UPDATES_BY_HAND, fixture, fixture_by_hand};

    fn server() -> Session {
        Session::new(Role::Server, Transport::WebTransport)
    }

    fn namespace() -> TrackNamespace {
        TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap()
    }

    fn track(name: &[u8]) -> FullTrackName {
        FullTrackName::new(namespace(), name.to_vec()).unwrap()
    }

    fn subscribe(request_id: u64, name: &[u8]) -> ControlMessage {
        ControlMessage::Subscribe(Subscribe {
            request_id,
            full_track_name: track(name),
            parameters: vec![],
        })
    }

    fn publish(request_id: u64, name: &[u8], track_alias: u64) -> ControlMessage {
        ControlMessage::Publish(Publish {
            request_id,
            full_track_name: track(name),
            track_alias,
            parameters: vec![],
            track_properties: vec![],
        })
    }

    fn publish_namespace(request_id: u64) -> ControlMessage {
        ControlMessage::PublishNamespace(PublishNamespace {
            request_id,
            track_namespace: namespace(),
            parameters: vec![],
        })
    }

    fn update(request_id: u64) -> ControlMessage {
        ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters: vec![],
        })
    }

    fn done(stream_count: u64) -> PublishDone {
        PublishDone {
            status_code: 0x2,
            stream_count,
            error_reason: ReasonPhrase::default(),
        }
    }

    fn subscribe_ok(track_alias: u64) -> ControlMessage {
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias,
            parameters: vec![],
            track_properties: vec![],
        })
    }

    fn opened(session: &mut Session, first: ControlMessage) -> PeerRequestId {
        match session.on_peer_request_opened(first) {
            Ok(PeerRequestOpened::Request(id, _)) => id,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// What the session refused a request of the peer with by itself: the
    /// error code, and the Request ID of the request it keeps as rejected,
    /// `None` where it keeps nothing.
    fn refusal(opened: Result<PeerRequestOpened, Error>) -> Result<(u64, Option<u64>), String> {
        match opened {
            Ok(PeerRequestOpened::Refused {
                response: ControlMessage::RequestError(error),
                request,
            }) => Ok((error.error_code, request.map(PeerRequestId::value))),
            other => Err(format!("{other:?}")),
        }
    }

    const NOT_ALLOWED: RequestFault = RequestFault::Peer(Error::ProtocolViolation(
        "message not allowed in this request state",
    ));

    /// §10.2.14: `TRACK_NAMESPACE_PREFIX` "MAY appear in `REQUEST_UPDATE` for
    /// a `SUBSCRIBE_NAMESPACE`", which is then refused and its stream closed
    /// (§10.9.1); in the update of a SUBSCRIBE it is not allowed (§10.2.1).
    #[test]
    fn a_new_prefix_is_refused_for_a_subscribe_namespace_and_not_allowed_for_a_subscribe() {
        let ns = |fields: &[&[u8]]| {
            TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
        };
        let update = ControlMessage::RequestUpdate(RequestUpdate {
            request_id: 2,
            parameters: vec![Parameter::TrackNamespacePrefix(ns(&[b"xmoq", b"other"]))],
        });
        let subscribe_namespace = ControlMessage::SubscribeNamespace(SubscribeNamespace {
            request_id: 0,
            track_namespace_prefix: ns(&[b"xmoq"]),
            parameters: vec![],
        });
        // (the request, whether it is answered first, what the update does)
        let cases = [
            (subscribe_namespace.clone(), true, Ok(true)),
            (subscribe_namespace, false, Ok(true)),
            (
                subscribe(0, b"a"),
                false,
                Err(RequestFault::Peer(Error::ProtocolViolation(
                    "parameter not allowed in this message",
                ))),
            ),
        ];
        for (first, answered, expected) in cases {
            let mut session = server();
            let PeerRequestOpened::Request(request, _) =
                session.on_peer_request_opened(first.clone()).unwrap()
            else {
                panic!("{first:?}");
            };
            if answered {
                session.accept_subscribe_namespace(request).unwrap();
            }
            let event = session.on_peer_request_message(request, update.clone());
            let refused_and_closed = event.map(|event| {
                matches!(
                    event,
                    PeerRequestEvent::UpdateRefused {
                        then: AfterRefusal::CloseStream,
                        ..
                    }
                )
            });
            assert_eq!(refused_and_closed, expected, "{first:?}");
        }
    }

    /// §10.18: a `SUBSCRIBE_NAMESPACE` whose prefix shares a common prefix
    /// with another gets `PREFIX_OVERLAP`. The first prefix is (`xmoq`,
    /// `chat`).
    #[test]
    fn a_subscribe_namespace_whose_prefix_overlaps_is_refused() {
        let ns = |fields: &[&[u8]]| {
            TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
        };
        let subscribe_namespace = |request_id: u64, prefix: TrackNamespace| {
            ControlMessage::SubscribeNamespace(SubscribeNamespace {
                request_id,
                track_namespace_prefix: prefix,
                parameters: vec![],
            })
        };
        // (the second prefix, whether it is refused)
        let cases = [
            (ns(&[b"xmoq", b"chat"]), true),
            (ns(&[b"xmoq"]), true),
            (ns(&[b"xmoq", b"chat", b"room"]), true),
            (ns(&[]), true),
            (ns(&[b"xmoq", b"video"]), false),
        ];
        for (second, refused) in cases {
            let mut session = server();
            let opened = session
                .on_peer_request_opened(subscribe_namespace(0, ns(&[b"xmoq", b"chat"])))
                .unwrap();
            let PeerRequestOpened::Request(first, _) = opened else {
                panic!("{opened:?}");
            };
            assert_eq!(
                session.accept_subscribe_namespace(first),
                Ok(ControlMessage::RequestOk(RequestOk::default()))
            );
            let again = session
                .on_peer_request_opened(subscribe_namespace(2, second.clone()))
                .unwrap();
            let overlap = matches!(
                again,
                PeerRequestOpened::Refused {
                    response: ControlMessage::RequestError(RequestError {
                        error_code: request_error_code::PREFIX_OVERLAP,
                        ..
                    }),
                    ..
                }
            );
            assert_eq!(overlap, refused, "{second:?}");
        }
    }

    /// `refuse_peer_request`, at a client, whose peer's Request IDs are odd
    /// (§10.1): the refusal, and what a `REQUEST_UPDATE` on the refused stream
    /// then says of its Request ID.
    #[test]
    fn a_request_refused_by_the_session_has_its_ids_checked() {
        let not_supported = ControlMessage::RequestError(RequestError {
            error_code: request_error_code::NOT_SUPPORTED,
            retry_interval: retry_interval::made_up(request_error_code::NOT_SUPPORTED),
            error_reason: ReasonPhrase::default(),
            redirect: None,
        });
        // (the SUBSCRIBE's Request ID, the update's, what the update leads to)
        let cases = [
            (1, Some(3), Ok(Ok(()))),
            (1, Some(1), Ok(Err(Error::InvalidRequestId(1)))),
            (0, None, Err(Error::InvalidRequestId(0))),
        ];
        for (subscribe_id, update_id, expected) in cases {
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let outcome = session
                .refuse_peer_request(subscribe(subscribe_id, b"a"))
                .map(|(response, request)| {
                    assert_eq!(response, not_supported);
                    let request = request.unwrap();
                    let Some(update_id) = update_id else {
                        return Ok(());
                    };
                    session
                        .on_peer_request_message(
                            request,
                            ControlMessage::RequestUpdate(RequestUpdate {
                                request_id: update_id,
                                parameters: vec![],
                            }),
                        )
                        .map(|_| ())
                        .map_err(|fault| match fault {
                            RequestFault::Peer(error) => error,
                            RequestFault::Usage(error) => panic!("{error}"),
                        })
                });
            assert_eq!(outcome, expected, "{subscribe_id} {update_id:?}");
        }
    }

    #[test]
    fn request_ids_of_the_peer_are_checked() {
        // (our role, IDs in the order they arrive, result of the last one)
        let cases: &[(Role, &[u64], Result<(), Error>)] = &[
            (Role::Server, &[0, 2, 4], Ok(())),
            (Role::Client, &[1, 3], Ok(())),
            // Wrong parity for the sender.
            (Role::Server, &[1], Err(Error::InvalidRequestId(1))),
            (Role::Client, &[0], Err(Error::InvalidRequestId(0))),
            // Requests travel on streams of their own: they may overtake each
            // other and leave gaps of any size.
            (Role::Server, &[4, 0, 2], Ok(())),
            (Role::Server, &[1_000_000, 0], Ok(())),
            // Duplicates, of the largest and of an overtaken one.
            (Role::Server, &[0, 0], Err(Error::InvalidRequestId(0))),
            (Role::Server, &[4, 2, 2], Err(Error::InvalidRequestId(2))),
        ];
        for (role, ids, expected) in cases {
            let mut session = Session::new(*role, Transport::WebTransport);
            let mut last = Ok(());
            for id in *ids {
                last = session
                    .on_peer_request_opened(publish_namespace(*id))
                    .map(|_| ());
            }
            assert_eq!(last, *expected, "{role:?} {ids:?}");
        }
    }

    #[test]
    fn request_ids_ahead_of_a_gap_are_remembered_up_to_a_limit() {
        // The expectations below are spelled out for a limit of 1024.
        assert_eq!(REQUEST_IDS_AHEAD_LIMIT, 1024);
        let mut peer = PeerRequests::new(Role::Server);
        // ID 0 is left out; IDs 2, 4, ..., 2050 are 1025 IDs ahead of that gap.
        for index in 1..=1025 {
            peer.take_id(index * 2).unwrap();
        }
        // The lowest remembered one, 2, was pushed out: its duplicate goes
        // undetected. The next one, 4, is still known.
        assert_eq!(peer.take_id(2), Ok(()));
        assert_eq!(peer.take_id(4), Err(Error::InvalidRequestId(4)));
        // The request in the gap is late, not a duplicate.
        assert_eq!(peer.take_id(0), Ok(()));
        assert_eq!(peer.take_id(0), Err(Error::InvalidRequestId(0)));
    }

    #[test]
    fn without_a_gap_every_duplicate_is_detected() {
        let mut peer = PeerRequests::new(Role::Server);
        for index in 0..5000 {
            peer.take_id(index * 2).unwrap();
        }
        for id in [0, 2, 4998, 9998] {
            assert_eq!(peer.take_id(id), Err(Error::InvalidRequestId(id)), "{id}");
        }
    }

    #[test]
    fn a_fetch_whose_answer_this_endpoint_ended_takes_only_request_ids() {
        let asked = |session: &mut Session| {
            let request = opened(session, standalone(0, at(3, 0), at(5, 0)));
            session
                .accept_fetch(request, false, at(5, 0), vec![])
                .unwrap();
            request
        };
        let with = |request_id, parameters| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters,
            })
        };
        let forward = vec![Parameter::Forward(true)];
        // While the FETCH is served: REQUEST_OK, and FORWARD, which the
        // update of a FETCH may not carry (§10.2.12), closes the session.
        let mut session = server();
        let request = asked(&mut session);
        assert!(matches!(
            session.on_peer_request_message(request, with(2, vec![])),
            Ok(PeerRequestEvent::UpdateAccepted { .. })
        ));
        assert_eq!(
            session.on_peer_request_message(request, with(4, forward.clone())),
            Err(RequestFault::Peer(Error::ProtocolViolation(
                "parameter not allowed in this message"
            )))
        );
        // After this endpoint ended its direction: no answer to either,
        // and each used up its Request ID.
        let mut session = server();
        let request = asked(&mut session);
        session.on_fetch_answer_ended(request);
        for (request_id, parameters) in [(2, vec![]), (4, forward)] {
            assert_eq!(
                session.on_peer_request_message(request, with(request_id, parameters)),
                Ok(PeerRequestEvent::LateMessage),
                "{request_id}"
            );
            assert_eq!(
                session.on_peer_request_opened(subscribe(request_id, b"b")),
                Err(Error::InvalidRequestId(request_id))
            );
        }
        // An ID that cannot be the peer's next still closes the session.
        assert_eq!(
            session.on_peer_request_message(request, with(3, vec![])),
            Err(RequestFault::Peer(Error::InvalidRequestId(3)))
        );
        // Held until its stream is gone, and no longer.
        assert_eq!(session.peer_request_count(), 1);
        session.on_peer_request_closed(request);
        assert_eq!(session.peer_request_count(), 0);

        // It changes nothing for a FETCH that has no answer, nor for any
        // other request.
        let mut session = server();
        let waiting = opened(&mut session, standalone(0, at(3, 0), at(5, 0)));
        let subscription = opened(&mut session, subscribe(2, b"a"));
        session.on_fetch_answer_ended(waiting);
        session.on_fetch_answer_ended(subscription);
        assert_eq!(
            session.on_peer_request_message(waiting, with(4, vec![])),
            Ok(PeerRequestEvent::UpdateHeld {
                subscriber_priority: None
            })
        );
        assert!(
            session
                .accept_subscribe(subscription, vec![], vec![])
                .is_ok()
        );
    }

    #[test]
    fn the_id_of_a_request_that_is_still_held_is_never_taken_twice() {
        // The request with the ID 2 is held while 1025 later IDs push it
        // out of the memory of IDs (0 was not used, so the gap is below
        // it), whatever it has come to: open, ended with PUBLISH_DONE, or
        // rejected.
        type Ends = fn(&mut Session, PeerRequestId);
        let cases: [(&str, Ends); 3] = [
            ("open", |_, _| {}),
            ("ended with PUBLISH_DONE", |session, request| {
                session.accept_subscribe(request, vec![], vec![]).unwrap();
                session
                    .subscribe_done(request, 0x2, 0, ReasonPhrase::default())
                    .unwrap();
            }),
            ("rejected", |session, request| {
                session
                    .reject(request, 0x10, 0, ReasonPhrase::default())
                    .unwrap();
            }),
        ];
        for (name, ends) in cases {
            let mut session = server();
            let request = opened(&mut session, subscribe(2, b"a"));
            ends(&mut session, request);
            for index in 2..=1026 {
                session.peer.take_id(index * 2).unwrap();
            }
            assert_eq!(
                session.on_peer_request_opened(subscribe(2, b"b")),
                Err(Error::InvalidRequestId(2)),
                "{name}"
            );
            // Once its stream is gone, the ID reads as one that is yet to
            // come (`REQUEST_IDS_AHEAD_LIMIT`): the price of the limit.
            session.on_peer_request_closed(request);
            assert!(
                session.on_peer_request_opened(subscribe(2, b"b")).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn the_id_of_an_open_request_is_never_taken_twice() {
        let mut session = server();
        // Request 0 stays open while 1025 later IDs push it out of the memory:
        // ID 0 is used, so the gap is at 2.
        opened(&mut session, subscribe(0, b"a"));
        for index in 2..=1027 {
            session.peer.take_id(index * 2).unwrap();
        }
        assert_eq!(
            session.on_peer_request_opened(subscribe(0, b"b")),
            Err(Error::InvalidRequestId(0))
        );
        assert!(session.peer.subscribes_to(&track(b"a")));
    }

    #[test]
    fn a_request_stream_begins_with_a_request() {
        let cases = [
            update(0),
            ControlMessage::RequestOk(RequestOk::default()),
            ControlMessage::PublishDone(done(0)),
        ];
        for first in cases {
            assert_eq!(
                server().on_peer_request_opened(first.clone()),
                Err(Error::ProtocolViolation(
                    "request stream did not begin with a request"
                )),
                "{first:?}"
            );
        }
    }

    /// One row of `fixtures/peer-subscribe/opened.tsv`; see its README.
    #[derive(Debug)]
    struct OpenedRow {
        name: String,
        before: String,
        /// One SUBSCRIBE, whole, as the server sends it.
        sent: Vec<u8>,
        /// `handed`, `refused 0x..` or `closed 0x..`.
        outcome: String,
    }

    /// The bytes a field of a shared table has in lower-case hexadecimal.
    fn hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
        if !hex.len().is_multiple_of(2)
            || hex
                .bytes()
                .any(|digit| !b"0123456789abcdef".contains(&digit))
        {
            return Err(format!("not lower-case hexadecimal bytes: {hex:?}"));
        }
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|error| format!("{hex:?}: {error}"))
    }

    fn opened_rows(table: &str) -> Result<Vec<OpenedRow>, String> {
        let Some(body) = table.strip_suffix('\n') else {
            return Err("the last row does not end with a line feed".to_string());
        };
        let mut rows = Vec::new();
        for line in body.split('\n') {
            let columns: Vec<&str> = line.split('\t').collect();
            let [name, before, hex, outcome] = columns.as_slice() else {
                return Err(format!("not four fields: {line:?}"));
            };
            let sent = hex_bytes(hex)?;
            if rows.iter().any(|row: &OpenedRow| row.name == *name) {
                return Err(format!("twice: {name:?}"));
            }
            rows.push(OpenedRow {
                name: (*name).to_string(),
                before: (*before).to_string(),
                sent,
                outcome: (*outcome).to_string(),
            });
        }
        Ok(rows)
    }

    /// The one message `bytes` are.
    fn whole_message(bytes: &[u8]) -> ControlMessage {
        match ControlMessage::decode(bytes) {
            Ok(Some((message, used))) if used == bytes.len() => message,
            other => panic!("not one whole message: {other:?}"),
        }
    }

    /// A client's session in the state the column `before` of
    /// `fixtures/peer-subscribe/opened.tsv` names. `subscribe` is the
    /// SUBSCRIBE of the server with the Request ID 1 for the track "object".
    fn session_before(before: &str, subscribe: &ControlMessage) -> Session {
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        session.setup(vec![]).unwrap();
        let object = FullTrackName::new(
            TrackNamespace::new(vec![b"xmoq".to_vec(), b"phase0".to_vec()]).unwrap(),
            b"object".to_vec(),
        )
        .unwrap();
        // The request of the server that came before, handed to the caller.
        let handed = |session: &mut Session| match session.on_peer_request_opened(subscribe.clone())
        {
            Ok(PeerRequestOpened::Request(request, PeerRequest::Subscribe(_))) => request,
            other => panic!("{before}: {other:?}"),
        };
        match before {
            "nothing" => {}
            "publish" => {
                session.publish(object, 1, vec![], vec![]).unwrap();
            }
            "subscribe" => {
                handed(&mut session);
            }
            "subscribe-accepted" => {
                let request = handed(&mut session);
                session.accept_subscribe(request, vec![], vec![]).unwrap();
            }
            "subscribe-done" => {
                let request = handed(&mut session);
                session.accept_subscribe(request, vec![], vec![]).unwrap();
                session
                    .subscribe_done(request, 0x2, 0, ReasonPhrase::default())
                    .unwrap();
            }
            "subscribe-rejected" => {
                let request = handed(&mut session);
                session
                    .reject(request, 0x10, 0, ReasonPhrase::default())
                    .unwrap();
            }
            "subscribe-rejected-then-update" => {
                let request = handed(&mut session);
                session
                    .reject(request, 0x10, 0, ReasonPhrase::default())
                    .unwrap();
                // REQUEST_UPDATE with the Request ID 3, on that stream.
                let update = whole_message(&fixture_by_hand("request-update-request-id-3"));
                session.on_peer_request_message(request, update).unwrap();
            }
            "subscribe-refused-then-update" => {
                handed(&mut session);
                // A second SUBSCRIBE for the track, with the Request ID 3.
                let ControlMessage::Subscribe(mut second) = subscribe.clone() else {
                    panic!("{before}: no SUBSCRIBE: {subscribe:?}");
                };
                second.request_id = 3;
                let refused = session.on_peer_request_opened(ControlMessage::Subscribe(second));
                let Ok(PeerRequestOpened::Refused {
                    request: Some(request),
                    ..
                }) = refused
                else {
                    panic!("{before}: {refused:?}");
                };
                // REQUEST_UPDATE with the Request ID 5, on that stream.
                let update = whole_message(&fixture_by_hand("request-update-request-id-5"));
                session.on_peer_request_message(request, update).unwrap();
            }
            other => panic!("a state the test does not know: {other}"),
        }
        session
    }

    /// The contract `fixtures/peer-subscribe/` pins for both implementations:
    /// what the session makes of a SUBSCRIBE the peer sends, before the
    /// caller sees it.
    #[test]
    fn a_subscribe_of_the_peer_is_handed_on_refused_or_fatal_where_the_shared_table_says() {
        let path = format!(
            "{}/../../fixtures/peer-subscribe/opened.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table = std::fs::read_to_string(&path).expect("the fixture is there");
        let rows = opened_rows(&table).unwrap_or_else(|error| panic!("{path}: {error}"));
        // As many as the table has: a row that is added there and not
        // read here is counted.
        assert_eq!(rows.len(), 12, "{path}");
        // The README has the request that came before be the SUBSCRIBE of
        // the first row: Request ID 1, for the track "object".
        let first = rows.first().expect("rows");
        assert_eq!(
            (first.name.as_str(), first.before.as_str()),
            ("first-request", "nothing")
        );
        let subscribe = whole_message(&first.sent);
        for row in &rows {
            let mut session = session_before(&row.before, &subscribe);
            let outcome = match session.on_peer_request_opened(whole_message(&row.sent)) {
                Ok(PeerRequestOpened::Request(_, PeerRequest::Subscribe(_))) => {
                    "handed".to_string()
                }
                Ok(PeerRequestOpened::Refused {
                    response: ControlMessage::RequestError(error),
                    ..
                }) => format!("refused {:#x}", error.error_code),
                Err(error) => match error.termination_code() {
                    Some(code) => format!("closed {:#x}", code.code()),
                    None => panic!("{row:?}: an error that closes no session: {error:?}"),
                },
                other => panic!("{row:?}: {other:?}"),
            };
            assert_eq!(outcome, row.outcome, "{row:?}");
        }
    }

    /// The rows of a shared table: its lines, each split at the tabs into
    /// `N` fields. The first field is the name of the row.
    fn table_rows<const N: usize>(path: &str) -> Vec<[String; N]> {
        let path = format!("{}/../../fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
        let table = std::fs::read_to_string(&path).expect("the fixture is there");
        let body = table
            .strip_suffix('\n')
            .unwrap_or_else(|| panic!("{path}: the last row does not end with a line feed"));
        let mut rows: Vec<[String; N]> = Vec::new();
        for line in body.split('\n') {
            let fields: Vec<String> = line.split('\t').map(str::to_string).collect();
            let row: [String; N] = fields
                .try_into()
                .unwrap_or_else(|_| panic!("{path}: not {N} fields: {line:?}"));
            assert!(
                rows.iter().all(|other| other[0] != row[0]),
                "{path}: twice: {:?}",
                row[0]
            );
            rows.push(row);
        }
        rows
    }

    /// What the session does for an error that closes it, as the shared
    /// tables write it.
    fn closed(error: &Error) -> String {
        match error.termination_code() {
            Some(code) => format!("closed {:#x}", code.code()),
            None => panic!("an error that closes no session: {error:?}"),
        }
    }

    /// The SUBSCRIBE that `fixtures/peer-subscribe/opened.tsv` begins with,
    /// which the column `before` of both tables means.
    fn first_subscribe_of_the_shared_table() -> ControlMessage {
        let rows = table_rows::<4>("peer-subscribe/opened.tsv");
        let [name, before, sent, _] = rows.first().expect("rows");
        assert_eq!(
            (name.as_str(), before.as_str()),
            ("first-request", "nothing")
        );
        whole_message(&hex_bytes(sent).unwrap())
    }

    /// The contract `fixtures/peer-fetch/opened.tsv` pins for both
    /// implementations: what the session makes of a FETCH the peer sends,
    /// before the caller sees it.
    #[test]
    fn a_fetch_of_the_peer_is_handed_on_refused_or_fatal_where_the_shared_table_says() {
        let rows = table_rows::<5>("peer-fetch/opened.tsv");
        // As many as the README has.
        assert_eq!(rows.len(), 22);
        let subscribe = first_subscribe_of_the_shared_table();
        for [name, before, kind, sent, outcome] in &rows {
            let mut session = session_before(before, &subscribe);
            let sent = hex_bytes(sent).unwrap_or_else(|error| panic!("{name}: {error}"));
            let (is, becomes) = match ControlMessage::decode(&sent) {
                // Bytes that are no message close the session before it
                // sees a request, and what kind of FETCH they were meant to
                // be is not known here.
                Err(error) => (kind.as_str(), closed(&error)),
                Ok(Some((message, used))) if used == sent.len() => {
                    let is = match &message {
                        ControlMessage::Fetch(Fetch {
                            range: FetchRange::Standalone { .. },
                            ..
                        }) => "standalone",
                        ControlMessage::Fetch(_) => "joining",
                        other => panic!("{name}: no FETCH: {other:?}"),
                    };
                    let becomes = match session.on_peer_request_opened(message) {
                        // WHERE the two implementations part, by design: a
                        // Joining FETCH that this session hands on is one
                        // `@xmoq/moqt` refuses itself (the README).
                        Ok(PeerRequestOpened::Request(_, PeerRequest::Fetch(_))) => {
                            "handed".to_string()
                        }
                        Ok(PeerRequestOpened::Refused {
                            response: ControlMessage::RequestError(error),
                            ..
                        }) => format!("refused {:#x}", error.error_code),
                        Err(error) => closed(&error),
                        other => panic!("{name}: {other:?}"),
                    };
                    (is, becomes)
                }
                other => panic!("{name}: not one whole message: {other:?}"),
            };
            assert_eq!((is, &becomes), (kind.as_str(), outcome), "{name}");
        }
    }

    /// The contract `fixtures/peer-fetch/updates.tsv` pins for both
    /// implementations: what is sent for each thing that happens to a
    /// Standalone FETCH of the peer.
    #[test]
    fn a_fetch_of_the_peer_is_answered_step_by_step_as_the_shared_table_says() {
        let rows = table_rows::<3>("peer-fetch/updates.tsv");
        // As many as the README has.
        assert_eq!(rows.len(), 13);
        // The FETCH of the README: the capture, with the Request ID 1, the
        // Start Location {3, 4} and the End Location {5, 1}.
        let ControlMessage::Fetch(mut fetch) =
            whole_message(&fixture("fetch-standalone-moqtail-rs"))
        else {
            panic!("the capture is a FETCH");
        };
        fetch.request_id = 1;
        let FetchRange::Standalone { start, end, .. } = &mut fetch.range else {
            panic!("the capture is a Standalone FETCH");
        };
        (*start, *end) = (at(3, 4), at(5, 1));
        let named = |message: &ControlMessage| match message {
            ControlMessage::FetchOk(_) => "FETCH_OK",
            ControlMessage::RequestOk(_) => "REQUEST_OK",
            ControlMessage::RequestError(_) => "REQUEST_ERROR",
            other => panic!("not an answer: {other:?}"),
        };
        for [name, steps, sent] in &rows {
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            session.setup(vec![]).unwrap();
            let request = match session.on_peer_request_opened(ControlMessage::Fetch(fetch.clone()))
            {
                Ok(PeerRequestOpened::Request(request, PeerRequest::Fetch(_))) => request,
                other => panic!("{name}: {other:?}"),
            };
            // The peer's next Request ID: 3, then 5.
            let mut next = 3;
            let mut done = Vec::new();
            for step in steps.split(' ') {
                let mut updated = |session: &mut Session, parameters| {
                    // Through the codec, as an endpoint reads it.
                    let mut bytes = Vec::new();
                    ControlMessage::RequestUpdate(RequestUpdate {
                        request_id: next,
                        parameters,
                    })
                    .encode(&mut bytes)
                    .unwrap();
                    next += 2;
                    match session.on_peer_request_message(request, whole_message(&bytes)) {
                        Ok(PeerRequestEvent::UpdateAccepted { response, .. }) => {
                            named(&response).to_string()
                        }
                        Ok(
                            PeerRequestEvent::UpdateHeld { .. }
                            | PeerRequestEvent::LateMessage
                            | PeerRequestEvent::UpdateAlreadyRefused,
                        ) => "-".to_string(),
                        Err(RequestFault::Peer(error)) => closed(&error),
                        other => panic!("{name}: {step}: {other:?}"),
                    }
                };
                let accepted = |session: &mut Session, end| match session.accept_fetch(
                    request,
                    false,
                    end,
                    vec![],
                ) {
                    Ok((ok, owed)) => {
                        let mut all = vec![named(&ok)];
                        all.extend((0..owed).map(|_| "REQUEST_OK"));
                        all.join(" ")
                    }
                    Err(_) => "usage-error".to_string(),
                };
                done.push(match step {
                    "update" => updated(&mut session, vec![]),
                    "update-priority" => {
                        updated(&mut session, vec![Parameter::SubscriberPriority(200)])
                    }
                    "update-forward" => updated(&mut session, vec![Parameter::Forward(true)]),
                    "accept" => accepted(&mut session, at(5, 1)),
                    "accept-before-start" => accepted(&mut session, at(3, 3)),
                    "accept-beyond-end" => accepted(&mut session, at(9, 0)),
                    "reject" => match session.reject(request, 0x10, 0, ReasonPhrase::default()) {
                        Ok(response) => named(&response).to_string(),
                        Err(_) => "usage-error".to_string(),
                    },
                    other => panic!("{name}: a step the test does not know: {other}"),
                });
            }
            assert_eq!(&done.join(" | "), sent, "{name}");
        }
    }

    /// One row of `fixtures/joining-request-id/held.tsv`; see its README.
    #[derive(Debug)]
    struct JoiningRow {
        name: String,
        /// The role of the peer: the client uses even Request IDs.
        peer: Role,
        /// The Request IDs the peer used before, in the order they came, and
        /// whether the request of each was gone again before the FETCH.
        used: Vec<(u64, bool)>,
        fetch: u64,
        joining: u64,
        /// `held`, and not `refused`.
        held: bool,
    }

    fn joining_rows(table: &str) -> Result<Vec<JoiningRow>, String> {
        let Some(body) = table.strip_suffix('\n') else {
            return Err("the last row does not end with a line feed".to_string());
        };
        let number = |text: &str| {
            text.parse::<u64>()
                .map_err(|error| format!("{text:?}: {error}"))
        };
        let mut rows = Vec::new();
        for line in body.split('\n') {
            let columns: Vec<&str> = line.split('\t').collect();
            let [name, peer, used, fetch, joining, held] = columns.as_slice() else {
                return Err(format!("not six columns: {line:?}"));
            };
            let peer = match *peer {
                "client" => Role::Client,
                "server" => Role::Server,
                other => return Err(format!("neither client nor server: {other:?}")),
            };
            let used = match *used {
                "-" => Vec::new(),
                ids => ids
                    .split(',')
                    .map(|id| match id.strip_suffix(":gone") {
                        Some(id) => number(id).map(|id| (id, true)),
                        None => number(id).map(|id| (id, false)),
                    })
                    .collect::<Result<_, _>>()?,
            };
            let held = match *held {
                "held" => true,
                "refused" => false,
                other => return Err(format!("neither held nor refused: {other:?}")),
            };
            if rows.iter().any(|row: &JoiningRow| row.name == *name) {
                return Err(format!("twice: {name:?}"));
            }
            rows.push(JoiningRow {
                name: (*name).to_string(),
                peer,
                used,
                fetch: number(fetch)?,
                joining: number(joining)?,
                held,
            });
        }
        Ok(rows)
    }

    /// The contract `fixtures/joining-request-id/` pins for both
    /// implementations: which Joining Request ID is one whose SUBSCRIBE is
    /// yet to come.
    #[test]
    fn a_joining_request_id_is_yet_to_come_where_the_shared_table_says() {
        let path = format!(
            "{}/../../fixtures/joining-request-id/held.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table = std::fs::read_to_string(&path).expect("the fixture is there");
        let rows = joining_rows(&table).unwrap_or_else(|error| panic!("{path}: {error}"));
        // As many as the table has: a row that is added there and not
        // read here is counted.
        assert_eq!(rows.len(), 30, "{path}");
        for row in &rows {
            let own = match row.peer {
                Role::Client => Role::Server,
                Role::Server => Role::Client,
            };
            let mut session = Session::new(own, Transport::WebTransport);
            // What the peer used before is no subscription: here it is a
            // PUBLISH_NAMESPACE, which this session hands to its caller and
            // keeps open. One that is gone is one whose stream the peer
            // reset before it was answered.
            for (id, gone) in &row.used {
                let opened = session
                    .on_peer_request_opened(publish_namespace(*id))
                    .unwrap_or_else(|error| panic!("{row:?}: {id}: {error:?}"));
                let PeerRequestOpened::Request(request, _) = opened else {
                    panic!("{row:?}: {id}: {opened:?}");
                };
                if *gone {
                    session.on_peer_request_closed(request);
                }
            }
            let opened = session.on_peer_request_opened(joining(row.fetch, row.joining));
            match opened {
                Ok(PeerRequestOpened::Request(request, PeerRequest::Fetch(_))) => {
                    assert!(row.held, "{row:?}: taken");
                    assert!(
                        session.peer_request_yet_to_come(row.joining, request),
                        "{row:?}"
                    );
                }
                // INVALID_JOINING_REQUEST_ID (0x32), and the session stays.
                other => {
                    assert_eq!(refusal(other), Ok((0x32, Some(row.fetch))), "{row:?}");
                    assert!(!row.held, "{row:?}: refused");
                }
            }
        }
    }

    /// A server's session in which the request of the client with the Request
    /// ID 0 was rejected, by the caller or by the session itself, and the
    /// handle of that request.
    fn rejected(how: &str) -> (Session, PeerRequestId) {
        let mut session = server();
        let reason = ReasonPhrase::default;
        // What the caller is handed and rejects with DOES_NOT_EXIST (0x10).
        let by_the_caller = |session: &mut Session, first| {
            let request = opened(session, first);
            session.reject(request, 0x10, 0, reason()).unwrap();
            request
        };
        // What the session refuses by itself.
        let by_the_session =
            |session: &mut Session, first| match session.on_peer_request_opened(first) {
                Ok(PeerRequestOpened::Refused {
                    request: Some(request),
                    ..
                }) => request,
                other => panic!("{how}: {other:?}"),
            };
        let request = match how {
            "a SUBSCRIBE the caller rejects" => by_the_caller(&mut session, subscribe(0, b"a")),
            "a PUBLISH the caller rejects" => by_the_caller(&mut session, publish(0, b"a", 7)),
            "a PUBLISH_NAMESPACE the caller rejects" => {
                by_the_caller(&mut session, publish_namespace(0))
            }
            "a FETCH the caller rejects" => {
                by_the_caller(&mut session, standalone(0, at(0, 0), at(1, 0)))
            }
            // The track this endpoint publishes: DUPLICATE_SUBSCRIPTION.
            "a SUBSCRIBE the session refuses" => {
                session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
                by_the_session(&mut session, subscribe(0, b"a"))
            }
            // SUBSCRIBE_TRACKS: NOT_SUPPORTED.
            "a request the session does not implement" => by_the_session(
                &mut session,
                ControlMessage::UnsupportedRequest {
                    message_type: 0x51,
                    request_id: 0,
                },
            ),
            // A SUBSCRIBE that is refused by the REQUEST_ERROR for a
            // REQUEST_UPDATE that came before its answer.
            "a SUBSCRIBE an early REQUEST_UPDATE ended" => {
                let request = opened(&mut session, subscribe(0, b"a"));
                assert!(matches!(
                    session.on_peer_request_message(request, update(100)),
                    Ok(PeerRequestEvent::UpdateRefused {
                        then: AfterRefusal::CloseStream,
                        ..
                    })
                ));
                request
            }
            other => panic!("a way the test does not know: {other}"),
        };
        (session, request)
    }

    const REJECTED: [&str; 7] = [
        "a SUBSCRIBE the caller rejects",
        "a PUBLISH the caller rejects",
        "a PUBLISH_NAMESPACE the caller rejects",
        "a FETCH the caller rejects",
        "a SUBSCRIBE the session refuses",
        "a request the session does not implement",
        "a SUBSCRIBE an early REQUEST_UPDATE ended",
    ];

    #[test]
    fn a_request_that_was_rejected_is_kept_until_its_stream_is_gone() {
        for how in REJECTED {
            let (mut session, request) = rejected(how);
            assert_eq!(session.peer_request_count(), 1, "{how}");
            // A REQUEST_UPDATE the peer sent before it saw the answer gets
            // none, and uses up its Request ID (§10.1): the next request
            // cannot have it.
            assert_eq!(
                session.on_peer_request_message(request, update(2)),
                Ok(PeerRequestEvent::LateMessage),
                "{how}"
            );
            assert_eq!(
                session.on_peer_request_opened(publish_namespace(2)),
                Err(Error::InvalidRequestId(2)),
                "{how}"
            );
            // So does any other message, a PUBLISH_DONE here.
            assert_eq!(
                session.on_peer_request_message(request, ControlMessage::PublishDone(done(0))),
                Ok(PeerRequestEvent::LateMessage),
                "{how}"
            );
            // The stream is gone: so is the request, and a message for it
            // now is one for a request that ended.
            session.on_peer_request_closed(request);
            assert_eq!(session.peer_request_count(), 0, "{how}");
            assert_eq!(
                session.on_peer_request_message(request, update(4)),
                Err(RequestFault::Peer(Error::ProtocolViolation(
                    "message after the request ended"
                ))),
                "{how}"
            );
        }
    }

    #[test]
    fn a_request_update_for_a_rejected_request_is_held_to_its_request_id() {
        // (the Request ID of the REQUEST_UPDATE; what the session says)
        let cases = [
            // Not one of a client's.
            (3, Err(RequestFault::Peer(Error::InvalidRequestId(3)))),
            // The ID of the request itself, used.
            (0, Err(RequestFault::Peer(Error::InvalidRequestId(0)))),
            // Far ahead, as requests may overtake each other.
            (1000, Ok(PeerRequestEvent::LateMessage)),
        ];
        for how in REJECTED {
            for (request_id, expected) in &cases {
                // 100 is used up in the one way that has a REQUEST_UPDATE
                // before the rejection; none of the cases is 100.
                let (mut session, request) = rejected(how);
                assert_eq!(
                    session.on_peer_request_message(request, update(*request_id)),
                    *expected,
                    "{how}, {request_id}"
                );
            }
        }
    }

    /// The `REQUEST_UPDATE` messages that the rows of `fixtures/peer-subscribe/`
    /// about a rejected request have arrive before their SUBSCRIBE. Written
    /// by hand from §10.9, so read here: each is that message, and is
    /// written the same.
    #[test]
    fn the_request_updates_written_by_hand_say_what_they_were_meant_to() {
        for (name, request_id, new_group) in REQUEST_UPDATES_BY_HAND {
            let bytes = fixture_by_hand(name);
            let message = ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters: new_group
                    .map(Parameter::NewGroupRequest)
                    .into_iter()
                    .collect(),
            });
            assert_eq!(
                ControlMessage::decode(&bytes),
                Ok(Some((message.clone(), bytes.len()))),
                "{name}"
            );
            let mut encoded = Vec::new();
            message.encode(&mut encoded).unwrap();
            assert_eq!(encoded, bytes, "{name}");
        }
    }

    #[test]
    fn accepting_a_subscribe_that_cannot_be_accepted_uses_up_no_track_alias() {
        let mut session = server();
        let first = opened(&mut session, subscribe(0, b"a"));
        let (alias, _) = session.accept_subscribe(first, vec![], vec![]).unwrap();
        // Accepted already, rejected, and no SUBSCRIBE: each fails.
        let rejected = opened(&mut session, subscribe(2, b"b"));
        session
            .reject(rejected, 0x10, 0, ReasonPhrase::default())
            .unwrap();
        let namespace = opened(&mut session, publish_namespace(4));
        for request in [first, rejected, namespace] {
            assert_eq!(
                session.accept_subscribe(request, vec![], vec![]),
                Err(UsageError("the request is not waiting for a response")),
                "{request:?}"
            );
        }
        // The next SUBSCRIBE gets the alias after the first one's.
        let next = opened(&mut session, subscribe(6, b"c"));
        let (next_alias, _) = session.accept_subscribe(next, vec![], vec![]).unwrap();
        assert_eq!((alias, next_alias), (0, 1));
    }

    /// What a client makes of the Joining FETCH `fetch` of its server that
    /// names `names`, after the server used the odd Request IDs `used`
    /// and left `open`, a Standalone FETCH, open among them: `held` for one
    /// it hands on as naming a request that is yet to come, or the code it
    /// refuses the FETCH with.
    fn at_the_limit_of_remembered_ids(
        used: std::ops::RangeInclusive<u64>,
        open: Option<u64>,
        fetch: u64,
        names: u64,
    ) -> String {
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        if let Some(open) = open {
            opened(&mut session, standalone(open, at(0, 0), at(1, 0)));
        }
        for id in used.step_by(2) {
            session.peer.take_id(id).unwrap();
        }
        match session.on_peer_request_opened(joining(fetch, names)) {
            Ok(PeerRequestOpened::Request(request, PeerRequest::Fetch(_))) => {
                assert!(session.peer_request_yet_to_come(names, request));
                "held".to_string()
            }
            Ok(PeerRequestOpened::Refused {
                response: ControlMessage::RequestError(error),
                ..
            }) => format!("refused {:#x}", error.error_code),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_fetch_itself_can_push_the_id_it_names_out_of_the_memory() {
        // The expectations below are spelled out for a limit of 1024.
        assert_eq!(REQUEST_IDS_AHEAD_LIMIT, 1024);
        // The Request ID 1 has not come. The ID of the FETCH is taken
        // before the one it names is looked up, and is the 1025th ahead of
        // the gap: the lowest one remembered is forgotten for it, and reads
        // as yet to come. `@xmoq/moqt` has the same two cases in a test.
        //
        // 3 to 2049 are 1024 IDs; the FETCH 2051 pushes out 3.
        assert_eq!(
            at_the_limit_of_remembered_ids(3..=2049, None, 2051, 3),
            "held"
        );
        // 3 to 2051 are 1025 IDs, so 3 was pushed out before; the FETCH
        // 2053 pushes out 5.
        assert_eq!(
            at_the_limit_of_remembered_ids(3..=2051, None, 2053, 5),
            "held"
        );
        // One that is still remembered: INVALID_JOINING_REQUEST_ID (0x32).
        assert_eq!(
            at_the_limit_of_remembered_ids(3..=2049, None, 2051, 5),
            "refused 0x32"
        );
        // And one that was pushed out while its request is open, the
        // Standalone FETCH 3, is not yet to come: 0x32, as it is no
        // SUBSCRIBE.
        assert_eq!(
            at_the_limit_of_remembered_ids(5..=2053, Some(3), 2055, 3),
            "refused 0x32"
        );
    }

    #[test]
    fn a_request_of_another_session_names_nothing_that_is_yet_to_come() {
        let mut session = server();
        let mut other = server();
        let held = |session: &mut Session| {
            let PeerRequestOpened::Request(request, _) =
                session.on_peer_request_opened(joining(2, 0)).unwrap()
            else {
                panic!("the FETCH is taken");
            };
            request
        };
        let (own, others) = (held(&mut session), held(&mut other));
        assert!(session.peer_request_yet_to_come(0, own));
        assert!(!session.peer_request_yet_to_come(0, others));
    }

    #[test]
    fn requests_this_crate_does_not_implement_are_refused_not_fatal() {
        let mut session = server();
        // SUBSCRIBE_TRACKS, with Request ID 0.
        assert_eq!(
            refusal(
                session.on_peer_request_opened(ControlMessage::UnsupportedRequest {
                    message_type: 0x51,
                    request_id: 0,
                })
            ),
            Ok((0x3, Some(0)))
        );
        // Its Request ID is used up all the same.
        assert_eq!(
            session.on_peer_request_opened(subscribe(0, b"a")),
            Err(Error::InvalidRequestId(0))
        );
    }

    fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    fn standalone(request_id: u64, start: Location, end: Location) -> ControlMessage {
        ControlMessage::Fetch(Fetch {
            request_id,
            range: FetchRange::Standalone {
                full_track_name: track(b"a"),
                start,
                end,
            },
            parameters: vec![],
        })
    }

    fn joining(request_id: u64, joining_request_id: u64) -> ControlMessage {
        ControlMessage::Fetch(Fetch {
            request_id,
            range: FetchRange::RelativeJoining {
                joining_request_id,
                joining_start: 0,
            },
            parameters: vec![],
        })
    }

    fn fetch_ok(end_location: Location) -> ControlMessage {
        ControlMessage::FetchOk(FetchOk {
            end_of_track: false,
            end_location,
            parameters: vec![],
            track_properties: vec![],
        })
    }

    #[test]
    fn a_fetch_is_handed_on_or_refused_by_its_range() {
        // A server with five requests of its client open: SUBSCRIBEs 0
        // (Pending), 2 (Established) and 4 (ended with PUBLISH_DONE), the
        // PUBLISH 6 and the PUBLISH_NAMESPACE 8; and its own PUBLISH, 1, which
        // makes the client a subscriber as well.
        let prepared = || {
            let mut session = server();
            opened(&mut session, subscribe(0, b"a"));
            let established = opened(&mut session, subscribe(2, b"b"));
            session
                .accept_subscribe(established, vec![], vec![])
                .unwrap();
            let ended = opened(&mut session, subscribe(4, b"c"));
            session.accept_subscribe(ended, vec![], vec![]).unwrap();
            session
                .subscribe_done(ended, 0x2, 0, ReasonPhrase::default())
                .unwrap();
            opened(&mut session, publish(6, b"d", 0));
            opened(&mut session, publish_namespace(8));
            session.publish(track(b"e"), 9, vec![], vec![]).unwrap();
            session
        };
        // (the FETCH, with Request ID 10; the code it is refused with).
        let cases = [
            (standalone(10, at(1, 0), at(1, 0)), None),
            (standalone(10, at(1, 5), at(1, 5)), None),
            // INVALID_RANGE (0x11).
            (standalone(10, at(1, 5), at(1, 4)), Some(0x11)),
            (standalone(10, at(2, 0), at(1, 0)), Some(0x11)),
            (joining(10, 0), None),
            (joining(10, 2), None),
            // INVALID_JOINING_REQUEST_ID (0x32): the subscription that is
            // over, the PUBLISH, the PUBLISH_NAMESPACE, an ID the client has
            // not used, and the PUBLISH of the server.
            (joining(10, 4), Some(0x32)),
            (joining(10, 6), Some(0x32)),
            (joining(10, 8), Some(0x32)),
            (joining(10, 12), Some(0x32)),
            (joining(10, 1), Some(0x32)),
        ];
        for (fetch, refused_with) in cases {
            let mut session = prepared();
            let result = session.on_peer_request_opened(fetch.clone());
            match refused_with {
                None => {
                    let ControlMessage::Fetch(message) = &fetch else {
                        panic!("a FETCH");
                    };
                    assert!(
                        matches!(
                            &result,
                            Ok(PeerRequestOpened::Request(id, PeerRequest::Fetch(handed)))
                                if id.value() == 10 && handed == message
                        ),
                        "{fetch:?}: {result:?}"
                    );
                }
                Some(code) => assert_eq!(refusal(result), Ok((code, Some(10))), "{fetch:?}"),
            }
            // Its Request ID is used up either way.
            assert_eq!(
                session.on_peer_request_opened(subscribe(10, b"z")),
                Err(Error::InvalidRequestId(10)),
                "{fetch:?}"
            );
        }
    }

    #[test]
    fn a_fetch_ok_is_no_request() {
        assert_eq!(
            server().on_peer_request_opened(fetch_ok(at(0, 0))),
            Err(Error::ProtocolViolation(
                "request stream did not begin with a request"
            ))
        );
    }

    #[test]
    fn a_fetch_gets_one_answer() {
        let mut session = server();
        let accepted = opened(&mut session, standalone(0, at(0, 0), at(3, 0)));
        let properties = vec![KeyValuePair::varint(0x0e, 5).unwrap()];
        assert_eq!(
            session.accept_fetch(accepted, true, at(2, 4), properties.clone()),
            Ok((
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: true,
                    end_location: at(2, 4),
                    parameters: vec![],
                    track_properties: properties,
                }),
                0
            ))
        );
        assert_eq!(
            session.accept_fetch(accepted, false, at(2, 4), vec![]),
            Err(NOT_PENDING)
        );
        assert_eq!(
            session.reject(accepted, 0x0, 0, ReasonPhrase::default()),
            Err(NOT_PENDING)
        );
        // It stays until its stream is reported gone.
        assert_eq!(session.peer_request_count(), 1);
        session.on_peer_request_closed(accepted);
        assert_eq!(session.peer_request_count(), 0);

        let rejected = opened(&mut session, standalone(2, at(0, 0), at(3, 0)));
        assert_eq!(
            session.reject(rejected, 0x11, 0, ReasonPhrase::default()),
            Ok(request_error(0x11))
        );
        session.on_peer_request_closed(rejected);
        assert_eq!(session.peer_request_count(), 0);
        assert_eq!(
            session.accept_fetch(rejected, false, at(2, 4), vec![]),
            Err(NOT_PENDING)
        );
        // A FETCH_OK that ends before the FETCH starts is not made: {1, 4}
        // for a FETCH from {1, 5}, and {3, 7} for an Absolute Joining FETCH
        // from Group 4. The FETCH stays to be answered.
        let from_1_5 = opened(&mut session, standalone(6, at(1, 5), at(9, 0)));
        let subscription = opened(&mut session, subscribe(8, b"b"));
        session
            .accept_subscribe(subscription, vec![], vec![])
            .unwrap();
        let absolute = opened(
            &mut session,
            ControlMessage::Fetch(Fetch {
                request_id: 10,
                range: FetchRange::AbsoluteJoining {
                    joining_request_id: subscription.value(),
                    joining_start: 4,
                },
                parameters: vec![],
            }),
        );
        let too_early = Err(UsageError("the FETCH_OK ends before the FETCH starts"));
        assert_eq!(
            session.accept_fetch(from_1_5, false, at(1, 4), vec![]),
            too_early
        );
        assert_eq!(
            session.accept_fetch(absolute, false, at(3, 7), vec![]),
            too_early
        );
        assert!(
            session
                .accept_fetch(from_1_5, false, at(1, 5), vec![])
                .is_ok()
        );
        assert!(
            session
                .accept_fetch(absolute, false, at(4, 0), vec![])
                .is_ok()
        );
        // For a Relative Joining FETCH the start is the caller's to know.
        let relative = opened(&mut session, joining(12, subscription.value()));
        assert!(
            session
                .accept_fetch(relative, false, at(0, 1), vec![])
                .is_ok()
        );
        // A SUBSCRIBE is not accepted as a FETCH.
        let subscription = opened(&mut session, subscribe(4, b"a"));
        assert_eq!(
            session.accept_fetch(subscription, false, at(2, 4), vec![]),
            Err(NOT_PENDING)
        );
    }

    #[test]
    fn an_update_of_a_fetch_is_answered_after_the_fetch() {
        let request_ok = ControlMessage::RequestOk(RequestOk::default());
        let mut session = server();
        let fetch = opened(&mut session, standalone(0, at(0, 0), at(3, 0)));
        // Two updates before the answer: nothing to send yet.
        for request_id in [2, 4] {
            assert_eq!(
                session.on_peer_request_message(fetch, update(request_id)),
                Ok(PeerRequestEvent::UpdateHeld {
                    subscriber_priority: None
                })
            );
        }
        // Each used up a Request ID (§10.1).
        assert_eq!(
            session.on_peer_request_opened(subscribe(4, b"a")),
            Err(Error::InvalidRequestId(4))
        );
        // The answer says that two REQUEST_OK are owed after it.
        assert_eq!(
            session.accept_fetch(fetch, false, at(3, 0), vec![]),
            Ok((fetch_ok(at(3, 0)), 2))
        );
        // After the answer an update is answered at once, and none is owed
        // twice.
        assert_eq!(
            session.on_peer_request_message(fetch, update(6)),
            Ok(PeerRequestEvent::UpdateAccepted {
                response: request_ok,
                subscriber_priority: None,
            })
        );
        // Anything else on the stream of a FETCH is a violation.
        assert_eq!(
            session.on_peer_request_message(fetch, fetch_ok(at(3, 0))),
            Err(NOT_ALLOWED)
        );
    }

    #[test]
    fn updates_of_a_fetch_that_is_rejected_get_no_answer() {
        let mut session = server();
        let fetch = opened(&mut session, standalone(0, at(0, 0), at(3, 0)));
        assert_eq!(
            session.on_peer_request_message(fetch, update(2)),
            Ok(PeerRequestEvent::UpdateHeld {
                subscriber_priority: None
            })
        );
        // The REQUEST_ERROR is all there is to send: the request is over.
        assert_eq!(
            session.reject(fetch, 0x11, 0, ReasonPhrase::default()),
            Ok(request_error(0x11))
        );
        // An update that was still on its way gets nothing, and uses up its
        // Request ID.
        assert_eq!(
            session.on_peer_request_message(fetch, update(4)),
            Ok(PeerRequestEvent::LateMessage)
        );
        assert_eq!(
            session.on_peer_request_opened(subscribe(4, b"a")),
            Err(Error::InvalidRequestId(4))
        );
        // A rejected FETCH is not answered again.
        assert_eq!(
            session.accept_fetch(fetch, false, at(3, 0), vec![]),
            Err(NOT_PENDING)
        );
        assert_eq!(
            session.reject(fetch, 0x11, 0, ReasonPhrase::default()),
            Err(NOT_PENDING)
        );
        // It is held until its stream is reported gone.
        assert_eq!(session.peer_request_count(), 1);
        session.on_peer_request_closed(fetch);
        assert_eq!(session.peer_request_count(), 0);
    }

    #[test]
    fn an_update_of_a_fetch_may_carry_what_the_draft_lists_for_it() {
        let with = |request_id, parameters| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters,
            })
        };
        // (the parameters of the update; whether the session goes on; the
        //  SUBSCRIBER_PRIORITY the caller is given).
        let cases = [
            (vec![], true, None),
            (vec![Parameter::SubscriberPriority(3)], true, Some(3)),
            (vec![Parameter::AuthorizationToken(vec![1])], true, None),
            (vec![Parameter::ObjectDeliveryTimeout(5)], true, None),
            (vec![Parameter::SubgroupDeliveryTimeout(5)], true, None),
            // "REQUEST_UPDATE (for a subscription)": §10.2.12, §10.2.13.
            (vec![Parameter::Forward(false)], false, None),
            (vec![Parameter::NewGroupRequest(0)], false, None),
            (
                vec![Parameter::SubscriberPriority(3), Parameter::Forward(true)],
                false,
                None,
            ),
        ];
        for (parameters, goes_on, subscriber_priority) in cases {
            // Before the FETCH is answered, and after.
            for answered in [false, true] {
                let mut session = server();
                let fetch = opened(&mut session, standalone(0, at(0, 0), at(3, 0)));
                if answered {
                    session
                        .accept_fetch(fetch, false, at(3, 0), vec![])
                        .unwrap();
                }
                let result = session.on_peer_request_message(fetch, with(2, parameters.clone()));
                let expected = if goes_on {
                    Ok(if answered {
                        PeerRequestEvent::UpdateAccepted {
                            response: ControlMessage::RequestOk(RequestOk::default()),
                            subscriber_priority,
                        }
                    } else {
                        PeerRequestEvent::UpdateHeld {
                            subscriber_priority,
                        }
                    })
                } else {
                    Err(RequestFault::Peer(Error::ProtocolViolation(
                        "parameter not allowed in this message",
                    )))
                };
                assert_eq!(result, expected, "{parameters:?}, answered: {answered}");
            }
        }
        // The same FORWARD in the update of a subscription is no violation.
        let mut session = server();
        let subscription = opened(&mut session, subscribe(0, b"a"));
        session
            .accept_subscribe(subscription, vec![], vec![])
            .unwrap();
        assert!(
            session
                .on_peer_request_message(subscription, with(2, vec![Parameter::Forward(false)]))
                .is_ok()
        );
    }

    #[test]
    fn a_joining_fetch_is_answered_once_its_subscribe_is_accepted() {
        let mut session = server();
        let subscription = opened(&mut session, subscribe(0, b"a"));
        let fetch = opened(&mut session, joining(2, 0));
        let not_established = Err(UsageError(
            "the SUBSCRIBE of the Joining FETCH is not established",
        ));
        // Pending: the caller holds the FETCH.
        assert_eq!(
            session.accept_fetch(fetch, false, at(1, 1), vec![]),
            not_established
        );
        session
            .accept_subscribe(subscription, vec![], vec![])
            .unwrap();
        assert!(session.accept_fetch(fetch, false, at(1, 1), vec![]).is_ok());

        // Over before it was accepted, and ended after: the caller rejects.
        let rejected = opened(&mut session, subscribe(4, b"b"));
        let of_the_rejected = opened(&mut session, joining(6, 4));
        session
            .reject(rejected, 0x10, 0, ReasonPhrase::default())
            .unwrap();
        let ended = opened(&mut session, subscribe(8, b"c"));
        let of_the_ended = opened(&mut session, joining(10, 8));
        session.accept_subscribe(ended, vec![], vec![]).unwrap();
        session
            .subscribe_done(ended, 0x2, 0, ReasonPhrase::default())
            .unwrap();
        for fetch in [of_the_rejected, of_the_ended] {
            assert_eq!(
                session.accept_fetch(fetch, false, at(1, 1), vec![]),
                not_established
            );
            assert!(
                session
                    .reject(fetch, 0x32, 0, ReasonPhrase::default())
                    .is_ok()
            );
        }
    }

    #[test]
    fn a_subscribe_is_accepted_and_ended() {
        let mut session = server();
        let request = opened(&mut session, subscribe(0, b"a"));
        assert_eq!(session.peer_subscription_alias(request), None);
        // Nothing to end before the SUBSCRIBE is answered.
        assert_eq!(
            session.subscribe_done(request, 0x2, 0, ReasonPhrase::default()),
            Err(UsageError("PUBLISH_DONE ends an accepted SUBSCRIBE"))
        );

        let (track_alias, ok) = session.accept_subscribe(request, vec![], vec![]).unwrap();
        assert_eq!(track_alias, 0);
        assert_eq!(
            ok,
            ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 0,
                parameters: vec![],
                track_properties: vec![],
            })
        );
        assert_eq!(session.peer_subscription_alias(request), Some(0));
        assert_eq!(
            session.accept_subscribe(request, vec![], vec![]),
            Err(NOT_PENDING)
        );

        assert_eq!(
            session.subscribe_done(request, 0x2, 1, ReasonPhrase::default()),
            Ok(ControlMessage::PublishDone(done(1)))
        );
        assert_eq!(session.peer_subscription_alias(request), None);
        // What the subscriber sent before it saw the PUBLISH_DONE is ignored.
        assert_eq!(
            session.on_peer_request_message(request, update(2)),
            Ok(PeerRequestEvent::LateMessage)
        );
        // An ignored REQUEST_UPDATE used up its Request ID all the same.
        assert_eq!(
            session.on_peer_request_message(request, update(2)),
            Err(RequestFault::Peer(Error::InvalidRequestId(2)))
        );
        session.on_peer_request_closed(request);
        assert_eq!(
            session.on_peer_request_message(request, update(4)),
            Err(RequestFault::Peer(Error::ProtocolViolation(
                "message after the request ended"
            )))
        );
    }

    #[test]
    fn each_accepted_subscribe_gets_an_alias_of_its_own() {
        let mut session = server();
        // An alias this endpoint chose for a PUBLISH of its own is skipped.
        session.publish(track(b"own"), 1, vec![], vec![]).unwrap();
        let aliases: Vec<u64> = [(0, b"a"), (2, b"b"), (4, b"c")]
            .into_iter()
            .map(|(id, name)| {
                let request = opened(&mut session, subscribe(id, name));
                session.accept_subscribe(request, vec![], vec![]).unwrap().0
            })
            .collect();
        assert_eq!(aliases, [0, 2, 3]);
    }

    #[test]
    fn a_second_subscription_in_the_same_role_is_refused() {
        let mut session = server();
        opened(&mut session, subscribe(0, b"a"));
        assert_eq!(
            refusal(session.on_peer_request_opened(subscribe(2, b"a"))),
            Ok((0x19, Some(2)))
        );
        opened(&mut session, publish(4, b"b", 7));
        assert_eq!(
            refusal(session.on_peer_request_opened(publish(6, b"b", 8))),
            Ok((0x19, Some(6)))
        );
        // The same track in the other role is a different subscription.
        opened(&mut session, publish(8, b"a", 9));

        // This endpoint may not double a subscription either.
        assert_eq!(
            session.subscribe(track(b"b"), vec![]).map(|_| ()),
            Err(UsageError("already a subscriber of this track"))
        );
        assert_eq!(
            session.publish(track(b"a"), 5, vec![], vec![]).map(|_| ()),
            Err(UsageError("already a publisher of this track"))
        );
    }

    #[test]
    fn a_subscribe_without_an_answer_gives_way_to_the_peers_publish() {
        // §5.1. (whether the SUBSCRIBE was answered before the PUBLISH came,
        //  what the PUBLISH leads to)
        let mut session = server();
        let (pending, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        let request = opened(&mut session, publish(0, b"a", 7));
        assert_eq!(session.request_state(pending), Some(RequestState::Pending));
        let (_, gave_way) = session.accept_publish(request, vec![]).unwrap();
        assert_eq!(gave_way, Some(pending));
        assert_eq!(
            session.request_state(pending),
            Some(RequestState::Terminated)
        );

        // A SUBSCRIBE that was accepted is a subscription in the same role.
        let mut session = server();
        let (own, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        session.on_request_message(own, subscribe_ok(3)).unwrap();
        assert_eq!(
            refusal(session.on_peer_request_opened(publish(0, b"a", 7))),
            Ok((0x19, Some(0)))
        );

        // Accepted between the PUBLISH and its answer: the PUBLISH cannot be
        // accepted any more.
        let mut session = server();
        let (own, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        let request = opened(&mut session, publish(0, b"a", 7));
        session.on_request_message(own, subscribe_ok(3)).unwrap();
        assert_eq!(
            session.accept_publish(request, vec![]),
            Err(UsageError("already a subscriber of this track"))
        );
    }

    #[test]
    fn a_publish_is_accepted_and_its_alias_drains() {
        let mut session = server();
        let request = opened(&mut session, publish(0, b"a", 7));
        // Objects may arrive before PUBLISH_OK.
        assert_eq!(
            session.subscribed_alias(7),
            SubscribedAlias::Established(&track(b"a"))
        );
        assert_eq!(
            session
                .accept_publish(request, vec![Parameter::Forward(true)])
                .map(|(ok, _)| ok),
            Ok(ControlMessage::RequestOk(RequestOk {
                parameters: vec![Parameter::Forward(true)],
                track_properties: vec![],
            }))
        );
        assert_eq!(
            session.accept_publish(request, vec![]).map(|_| ()),
            Err(NOT_PENDING)
        );

        assert_eq!(
            session.on_peer_request_message(request, ControlMessage::PublishDone(done(1))),
            Ok(PeerRequestEvent::PublishDone {
                track_alias: 7,
                done: done(1),
            })
        );
        assert_eq!(
            session.subscribed_alias(7),
            SubscribedAlias::Draining(&track(b"a"))
        );
        session.on_data_stream_closed(7);
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn a_publish_with_the_alias_of_another_track_closes_the_session() {
        let mut session = server();
        opened(&mut session, publish(0, b"a", 7));
        assert_eq!(
            session.on_peer_request_opened(publish(2, b"b", 7)),
            Err(Error::DuplicateTrackAlias(7))
        );
    }

    fn track_status(request_id: u64) -> ControlMessage {
        ControlMessage::TrackStatus(Subscribe {
            request_id,
            full_track_name: track(b"a"),
            parameters: vec![],
        })
    }

    /// §10.14: a `TRACK_STATUS` gets one answer, `REQUEST_OK` with what a
    /// `SUBSCRIBE_OK` would have had or `REQUEST_ERROR`, and "the subscriber
    /// cannot send `REQUEST_UPDATE`".
    #[test]
    fn a_track_status_of_the_peer_is_answered_once() {
        let largest = Parameter::LargestObject(Location {
            group: 4,
            object: 1,
        });
        let property = KeyValuePair::varint(2, 5).unwrap();
        let mut session = server();
        let request = opened(&mut session, track_status(0));
        assert_eq!(
            session.accept_track_status(request, vec![Parameter::Expires(1)], vec![]),
            Err(UsageError("parameter not allowed in this message"))
        );
        assert_eq!(
            session.accept_track_status(request, vec![largest.clone()], vec![property.clone()]),
            Ok(ControlMessage::RequestOk(RequestOk {
                parameters: vec![largest.clone()],
                track_properties: vec![property],
            }))
        );
        assert_eq!(
            session.accept_track_status(request, vec![largest], vec![]),
            Err(NOT_PENDING)
        );
        assert_eq!(
            session.reject(request, 0x10, 0, ReasonPhrase::default()),
            Err(NOT_PENDING)
        );

        let mut session = server();
        let request = opened(&mut session, track_status(0));
        assert_eq!(
            session.reject(request, 0x10, 0, ReasonPhrase::default()),
            Ok(request_error(0x10))
        );

        let mut session = server();
        let request = opened(&mut session, track_status(0));
        assert_eq!(
            session.on_peer_request_message(
                request,
                ControlMessage::RequestUpdate(RequestUpdate {
                    request_id: 2,
                    parameters: vec![],
                }),
            ),
            Err(RequestFault::Peer(Error::ProtocolViolation(
                "message not allowed in this request state"
            )))
        );
    }

    #[test]
    fn a_publish_this_endpoint_cannot_use_is_refused() {
        // (the Track Properties; the code of the REQUEST_ERROR)
        // UNSUPPORTED_EXTENSION 0x33 for a Mandatory Track Property (0x4000),
        // outside or inside Immutable Properties (0x0B, `c0 40 00 01`);
        // MALFORMED_TRACK 0x12 for Immutable Properties inside Immutable
        // Properties.
        let cases = [
            (vec![KeyValuePair::varint(0x4000, 1).unwrap()], 0x33),
            (
                vec![KeyValuePair::bytes(0x0b, vec![0xc0, 0x40, 0x00, 0x01]).unwrap()],
                0x33,
            ),
            (
                vec![KeyValuePair::bytes(0x0b, vec![0x0b, 0x00]).unwrap()],
                0x12,
            ),
        ];
        for (track_properties, code) in cases {
            let mut session = server();
            let message = Publish {
                request_id: 0,
                full_track_name: track(b"a"),
                track_alias: 7,
                parameters: vec![],
                track_properties: track_properties.clone(),
            };
            assert_eq!(
                refusal(session.on_peer_request_opened(ControlMessage::Publish(message))),
                Ok((code, Some(0))),
                "{track_properties:?}"
            );
            assert_eq!(session.subscribed_alias(7), SubscribedAlias::Unknown);
        }

        let mut session = server();
        let mut message = Publish {
            request_id: 0,
            full_track_name: track(b"a"),
            track_alias: 7,
            parameters: vec![],
            track_properties: vec![KeyValuePair::varint(0x4000, 1).unwrap()],
        };
        assert_eq!(
            refusal(session.on_peer_request_opened(ControlMessage::Publish(message.clone()))),
            Ok((0x33, Some(0)))
        );

        // The same track without the property is fine afterwards.
        message.request_id = 2;
        message.track_properties.clear();
        opened(&mut session, ControlMessage::Publish(message));
    }

    #[test]
    fn rejecting_ends_the_request_and_frees_what_it_held() {
        let mut session = server();
        let request = opened(&mut session, publish(0, b"a", 7));
        assert_eq!(
            session.reject(request, 0x34, 0, ReasonPhrase::default()),
            Err(UsageError("a REDIRECT needs a Redirect"))
        );
        assert_eq!(
            session.reject(request, 0x20, 0, ReasonPhrase::default()),
            Ok(request_error(0x20))
        );
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
        assert_eq!(
            session.reject(request, 0x20, 0, ReasonPhrase::default()),
            Err(NOT_PENDING)
        );

        // An answered request cannot be rejected any more.
        let request = opened(&mut session, publish_namespace(2));
        session.accept_publish_namespace(request).unwrap();
        assert_eq!(
            session.reject(request, 0x20, 0, ReasonPhrase::default()),
            Err(NOT_PENDING)
        );
    }

    #[test]
    fn a_closed_request_stream_frees_what_the_request_held() {
        let mut session = server();
        let subscribed = opened(&mut session, subscribe(0, b"a"));
        session
            .accept_subscribe(subscribed, vec![], vec![])
            .unwrap();
        let published = opened(&mut session, publish(2, b"b", 7));
        session.accept_publish(published, vec![]).unwrap();

        session.on_peer_request_closed(subscribed);
        session.on_peer_request_closed(published);
        // Reporting twice is harmless.
        session.on_peer_request_closed(published);

        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
        // Both tracks can be subscribed to again.
        opened(&mut session, subscribe(4, b"a"));
        opened(&mut session, publish(6, b"b", 7));
    }

    #[test]
    fn a_subscriber_of_a_publish_updates_it_and_reads_the_answers_in_order() {
        let update = |request_id, group| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters: vec![Parameter::NewGroupRequest(group)],
            })
        };
        let ask = |session: &mut Session, request, group| {
            session.update_published(request, vec![Parameter::NewGroupRequest(group)])
        };
        let ok = || ControlMessage::RequestOk(RequestOk::default());
        let mut session = server();
        let request = opened(&mut session, publish(0, b"a", 7));
        // Not before the PUBLISH is accepted.
        assert!(ask(&mut session, request, 0).is_err());
        session.accept_publish(request, vec![]).unwrap();
        // An answer nobody waits for is no message of this stream.
        assert!(session.on_peer_request_message(request, ok()).is_err());

        let mut session = server();
        let request = opened(&mut session, publish(0, b"a", 7));
        session.accept_publish(request, vec![]).unwrap();
        // The server's Request IDs are odd, and each update uses one (§10.1).
        assert_eq!(ask(&mut session, request, 0), Ok(update(1, 0)));
        assert_eq!(ask(&mut session, request, 5), Ok(update(3, 5)));
        assert_eq!(
            session.on_peer_request_message(request, ok()),
            Ok(PeerRequestEvent::OwnUpdateAccepted { parameters: vec![] })
        );
        // A refusal answers all that wait (§10.9.1), and PUBLISH_DONE ends
        // the subscription as any other.
        let refusal = RequestError {
            error_code: 0x3,
            retry_interval: 0,
            error_reason: ReasonPhrase::default(),
            redirect: None,
        };
        assert_eq!(
            session.on_peer_request_message(request, ControlMessage::RequestError(refusal.clone())),
            Ok(PeerRequestEvent::OwnUpdateFailed(refusal))
        );
        assert!(session.on_peer_request_message(request, ok()).is_err());

        // 16 may wait for their answer, and the 17th is not made.
        let mut session = server();
        let request = opened(&mut session, publish(0, b"a", 7));
        session.accept_publish(request, vec![]).unwrap();
        for sent in 0..16 {
            assert!(ask(&mut session, request, 0).is_ok(), "{sent}");
        }
        assert!(ask(&mut session, request, 0).is_err());
    }

    #[test]
    fn an_update_of_an_established_subscription_is_applied_when_it_only_sets_the_subgroup_timeout()
    {
        let with = |request_id, parameters| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters,
            })
        };
        // (the parameters of the update; the timeout and the
        // NEW_GROUP_REQUEST that are reported, or `Err` where the update is
        // refused).
        let cases = [
            (vec![], Ok((None, None))),
            (
                vec![Parameter::SubgroupDeliveryTimeout(250)],
                Ok((Some(250), None)),
            ),
            // §8: 0 is "no timeout", which is a change like another.
            (
                vec![Parameter::SubgroupDeliveryTimeout(0)],
                Ok((Some(0), None)),
            ),
            // Taken as a SUBSCRIBE that sets it is; nothing acts on it.
            (
                vec![Parameter::ObjectDeliveryTimeout(250)],
                Ok((None, None)),
            ),
            (
                vec![
                    Parameter::ObjectDeliveryTimeout(250),
                    Parameter::SubgroupDeliveryTimeout(120),
                ],
                Ok((Some(120), None)),
            ),
            (vec![Parameter::Forward(false)], Err(())),
            (vec![Parameter::SubscriberPriority(3)], Err(())),
            // §10.2.13: 0 is "no Group information", a request like another.
            (vec![Parameter::NewGroupRequest(0)], Ok((None, Some(0)))),
            (
                vec![
                    Parameter::NewGroupRequest(7),
                    Parameter::SubgroupDeliveryTimeout(250),
                ],
                Ok((Some(250), Some(7))),
            ),
            (
                vec![Parameter::NewGroupRequest(7), Parameter::Forward(true)],
                Err(()),
            ),
            (
                vec![
                    Parameter::SubgroupDeliveryTimeout(250),
                    Parameter::Forward(true),
                ],
                Err(()),
            ),
        ];
        for (parameters, expected) in cases {
            let mut session = server();
            let request = opened(&mut session, subscribe(0, b"a"));
            session.accept_subscribe(request, vec![], vec![]).unwrap();
            let event = session
                .on_peer_request_message(request, with(2, parameters.clone()))
                .unwrap();
            let reported = match event {
                PeerRequestEvent::SubscriptionUpdated {
                    response: ControlMessage::RequestOk(_),
                    subgroup_delivery_timeout,
                    new_group_request,
                } => Ok((subgroup_delivery_timeout, new_group_request)),
                PeerRequestEvent::UpdateRefused {
                    then: AfterRefusal::PublishDone,
                    ..
                } => Err(()),
                other => panic!("{parameters:?}: {other:?}"),
            };
            assert_eq!(reported, expected, "{parameters:?}");
            // An update that was applied leaves the subscription as it was:
            // it takes another, and ends with any status.
            if expected.is_ok() {
                assert!(matches!(
                    session.on_peer_request_message(request, with(4, vec![])),
                    Ok(PeerRequestEvent::SubscriptionUpdated { .. })
                ));
                assert!(
                    session
                        .subscribe_done(request, 0x2, 0, ReasonPhrase::default())
                        .is_ok()
                );
            }
        }
    }

    #[test]
    fn a_request_update_is_refused_in_every_state() {
        let refused = |then| {
            Ok(PeerRequestEvent::UpdateRefused {
                response: request_error(0x3),
                then,
            })
        };

        // An accepted SUBSCRIBE: this endpoint is the publisher and has to end
        // the subscription with UPDATE_FAILED (0x8).
        let mut session = server();
        let request = opened(&mut session, subscribe(0, b"a"));
        session.accept_subscribe(request, vec![], vec![]).unwrap();
        assert_eq!(
            session.on_peer_request_message(
                request,
                ControlMessage::RequestUpdate(RequestUpdate {
                    request_id: 2,
                    parameters: vec![Parameter::Forward(false)],
                })
            ),
            refused(AfterRefusal::PublishDone)
        );
        assert_eq!(
            session.on_peer_request_message(request, update(4)),
            Ok(PeerRequestEvent::UpdateAlreadyRefused)
        );
        assert_eq!(
            session.subscribe_done(request, 0x2, 0, ReasonPhrase::default()),
            Err(UsageError(
                "a refused REQUEST_UPDATE ends the subscription with UPDATE_FAILED"
            ))
        );
        assert!(
            session
                .subscribe_done(request, 0x8, 0, ReasonPhrase::default())
                .is_ok()
        );

        // An accepted PUBLISH: the peer is the publisher and ends it itself.
        let request = opened(&mut session, publish(6, b"b", 7));
        session.accept_publish(request, vec![]).unwrap();
        assert_eq!(
            session.on_peer_request_message(request, update(8)),
            refused(AfterRefusal::Nothing)
        );

        // An accepted PUBLISH_NAMESPACE, and a request not answered yet: the
        // stream is closed and the request is over.
        let request = opened(&mut session, publish_namespace(10));
        session.accept_publish_namespace(request).unwrap();
        assert_eq!(
            session.on_peer_request_message(request, update(12)),
            refused(AfterRefusal::CloseStream)
        );
        let request = opened(&mut session, subscribe(14, b"c"));
        assert_eq!(
            session.on_peer_request_message(request, update(16)),
            refused(AfterRefusal::CloseStream)
        );
        assert_eq!(
            session.accept_subscribe(request, vec![], vec![]),
            Err(NOT_PENDING)
        );

        // The Request ID of an update is checked like any other.
        let request = opened(&mut session, publish_namespace(18));
        assert_eq!(
            session.on_peer_request_message(request, update(18)),
            Err(RequestFault::Peer(Error::InvalidRequestId(18)))
        );
    }

    #[test]
    fn messages_that_fit_no_state_are_violations() {
        let mut session = server();
        let subscribed = opened(&mut session, subscribe(0, b"a"));
        // PUBLISH_DONE comes from a publisher, not from a subscriber.
        assert_eq!(
            session.on_peer_request_message(subscribed, ControlMessage::PublishDone(done(0))),
            Err(NOT_ALLOWED)
        );
        let published = opened(&mut session, publish(2, b"b", 7));
        assert_eq!(
            session.on_peer_request_message(published, subscribe(4, b"c")),
            Err(NOT_ALLOWED)
        );
    }

    #[test]
    fn more_streams_than_publish_done_counted_are_let_through() {
        // §10.11 lets a subscriber close the session for it (MAY). This crate
        // does not: the count of a late stream can belong to an earlier use of
        // the alias, and the peer would be cut off for nothing.
        let mut session = server();
        let request = opened(&mut session, publish(0, b"a", 7));
        session.on_data_stream_closed(7);
        session.on_data_stream_closed(7);
        assert_eq!(
            session.on_peer_request_message(request, ControlMessage::PublishDone(done(1))),
            Ok(PeerRequestEvent::PublishDone {
                track_alias: 7,
                done: done(1),
            })
        );
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn a_publish_that_reuses_the_alias_of_a_draining_subscription() {
        #[derive(Debug, Clone, Copy)]
        enum Answer {
            Accepted,
            Refused,
            /// Refused, after a data stream under the alias ended.
            RefusedAfterAStream,
        }
        #[derive(Debug, PartialEq)]
        enum Alias {
            Established,
            Draining,
            Retired,
        }
        let alias_of = |session: &Session| match session.subscribed_alias(7) {
            SubscribedAlias::Established(_) => Alias::Established,
            SubscribedAlias::Draining(_) => Alias::Draining,
            SubscribedAlias::Retired => Alias::Retired,
            SubscribedAlias::Unknown => panic!("the alias is known in every case"),
        };
        // The first PUBLISH, under alias 7, ended with a PUBLISH_DONE that
        // counted 2 data streams, of which none has ended. The publisher
        // publishes the track anew under the same alias.
        // (what the second PUBLISH gets; the alias right after; the alias
        //  after one more data stream under it ended; and after a second)
        let cases = [
            // The alias is the new subscription's, and the old one is over
            // (the same as for a SUBSCRIBE_OK that reuses the alias).
            (
                Answer::Accepted,
                Alias::Established,
                Alias::Established,
                Alias::Established,
            ),
            // A subscription that was never established ends nothing: the
            // first one goes on draining, for the two streams it counted.
            (
                Answer::Refused,
                Alias::Draining,
                Alias::Draining,
                Alias::Retired,
            ),
            // The stream that ended before the refusal was one of the two.
            (
                Answer::RefusedAfterAStream,
                Alias::Draining,
                Alias::Retired,
                Alias::Retired,
            ),
        ];
        for (answer, at_once, after_one, after_two) in cases {
            let mut session = server();
            let first = opened(&mut session, publish(0, b"a", 7));
            session.accept_publish(first, vec![]).unwrap();
            session
                .on_peer_request_message(first, ControlMessage::PublishDone(done(2)))
                .unwrap();
            assert_eq!(alias_of(&session), Alias::Draining, "{answer:?}");

            let second = opened(&mut session, publish(2, b"a", 7));
            // Objects of the second may come before its answer (§10.10).
            assert_eq!(alias_of(&session), Alias::Established, "{answer:?}");
            match answer {
                Answer::Accepted => {
                    session.accept_publish(second, vec![]).unwrap();
                }
                Answer::Refused => {
                    session
                        .reject(
                            second,
                            request_error_code::EXCESSIVE_LOAD,
                            0,
                            ReasonPhrase::default(),
                        )
                        .unwrap();
                }
                Answer::RefusedAfterAStream => {
                    session.on_data_stream_closed(7);
                    session
                        .reject(
                            second,
                            request_error_code::EXCESSIVE_LOAD,
                            0,
                            ReasonPhrase::default(),
                        )
                        .unwrap();
                }
            }
            assert_eq!(alias_of(&session), at_once, "{answer:?}");
            session.on_data_stream_closed(7);
            assert_eq!(alias_of(&session), after_one, "{answer:?}");
            session.on_data_stream_closed(7);
            assert_eq!(alias_of(&session), after_two, "{answer:?}");
        }
    }

    #[test]
    fn a_draining_subscription_whose_alias_a_refused_publish_used_still_times_out() {
        let mut session = server();
        let first = opened(&mut session, publish(0, b"a", 7));
        session.accept_publish(first, vec![]).unwrap();
        session
            .on_peer_request_message(first, ControlMessage::PublishDone(done(2)))
            .unwrap();
        let second = opened(&mut session, publish(2, b"a", 7));
        session
            .reject(
                second,
                request_error_code::EXCESSIVE_LOAD,
                0,
                ReasonPhrase::default(),
            )
            .unwrap();
        assert!(matches!(
            session.subscribed_alias(7),
            SubscribedAlias::Draining(_)
        ));
        // The caller's wait for the two streams runs out.
        session.on_drain_timeout(7);
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn an_offer_that_is_not_accepted_leaves_the_draining_subscription_its_alias() {
        #[derive(Debug, Clone, Copy)]
        enum Step {
            /// A data stream under the alias ends.
            StreamEnds,
            /// The caller's wait for the draining subscription runs out.
            WaitRunsOut,
            /// The PUBLISH is refused by the caller.
            Refused,
            /// Its publisher ends it with `PUBLISH_DONE`, which counts 5.
            PublishDone,
            /// Its stream is closed before an answer.
            StreamClosed,
            /// A `REQUEST_UPDATE` arrives before an answer, which ends it.
            Updated,
        }
        #[derive(Debug, PartialEq)]
        enum Alias {
            Draining,
            Retired,
        }
        // A PUBLISH under alias 7 was accepted and ended with a PUBLISH_DONE
        // that counted `counted` data streams, none of which has ended. A
        // second PUBLISH comes under the same alias and is not accepted.
        // (streams the first counted; what happens after the second came;
        //  the alias then; the alias after one more data stream ended)
        let cases: [(u64, &[Step], Alias, Alias); 8] = [
            (2, &[Step::Refused], Alias::Draining, Alias::Draining),
            // The stream that ended while the second had the alias is
            // counted for the first, which has its one stream then.
            (
                1,
                &[Step::StreamEnds, Step::Refused],
                Alias::Retired,
                Alias::Retired,
            ),
            // The wait ran out while the second had the alias: the first is
            // over as soon as the alias is back.
            (
                2,
                &[Step::WaitRunsOut, Step::Refused],
                Alias::Retired,
                Alias::Retired,
            ),
            // The second ends itself before it has an answer (§5.1 lets a
            // publisher do so). It was never accepted, so the first goes on,
            // with its own count of 2 and not the second's of 5.
            (2, &[Step::PublishDone], Alias::Draining, Alias::Draining),
            (1, &[Step::PublishDone], Alias::Draining, Alias::Retired),
            (
                2,
                &[Step::PublishDone, Step::WaitRunsOut],
                Alias::Retired,
                Alias::Retired,
            ),
            (2, &[Step::StreamClosed], Alias::Draining, Alias::Draining),
            (2, &[Step::Updated], Alias::Draining, Alias::Draining),
        ];
        for (counted, steps, then, after_a_stream) in cases {
            let mut session = server();
            let first = opened(&mut session, publish(0, b"a", 7));
            session.accept_publish(first, vec![]).unwrap();
            session
                .on_peer_request_message(first, ControlMessage::PublishDone(done(counted)))
                .unwrap();
            let second = opened(&mut session, publish(2, b"a", 7));
            for step in steps {
                match step {
                    Step::StreamEnds => session.on_data_stream_closed(7),
                    Step::WaitRunsOut => session.on_drain_timeout(7),
                    Step::Refused => {
                        let code = request_error_code::EXCESSIVE_LOAD;
                        session
                            .reject(second, code, 0, ReasonPhrase::default())
                            .unwrap();
                    }
                    Step::PublishDone => {
                        session
                            .on_peer_request_message(second, ControlMessage::PublishDone(done(5)))
                            .unwrap();
                    }
                    Step::StreamClosed => session.on_peer_request_closed(second),
                    Step::Updated => {
                        let update = ControlMessage::RequestUpdate(RequestUpdate {
                            request_id: 4,
                            parameters: vec![],
                        });
                        session.on_peer_request_message(second, update).unwrap();
                    }
                }
            }
            let alias_of = |session: &Session| match session.subscribed_alias(7) {
                SubscribedAlias::Draining(_) => Alias::Draining,
                SubscribedAlias::Retired => Alias::Retired,
                other => panic!("{counted} {steps:?}: {other:?}"),
            };
            assert_eq!(alias_of(&session), then, "{counted} {steps:?}");
            session.on_data_stream_closed(7);
            assert_eq!(alias_of(&session), after_a_stream, "{counted} {steps:?}");
        }
    }

    #[test]
    fn a_publish_beyond_the_limit_leaves_the_draining_subscription_its_alias() {
        // The expectations below are spelled out for a limit of 256.
        assert_eq!(MAX_PEER_REQUESTS, 256);
        let mut session = server();
        let first = opened(&mut session, publish(0, b"a", 7));
        session.accept_publish(first, vec![]).unwrap();
        session
            .on_peer_request_message(first, ControlMessage::PublishDone(done(2)))
            .unwrap();
        for index in 1..=256 {
            opened(&mut session, publish_namespace(index * 2));
        }
        // The session refuses the 257th request itself, and the alias the
        // refused PUBLISH named is the draining subscription's again.
        assert_eq!(
            refusal(session.on_peer_request_opened(publish(514, b"a", 7))),
            Ok((0x9, None))
        );
        assert!(matches!(
            session.subscribed_alias(7),
            SubscribedAlias::Draining(_)
        ));
        session.on_data_stream_closed(7);
        session.on_data_stream_closed(7);
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn a_refused_publish_leaves_a_draining_subscribe_of_this_endpoint_its_alias() {
        let mut session = server();
        // This endpoint subscribed to "a"; the publisher chose alias 7 and
        // ended the subscription with a PUBLISH_DONE that counted 2 streams.
        let (own, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        let ok = ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 7,
            parameters: vec![],
            track_properties: vec![],
        });
        session.on_request_message(own, ok).unwrap();
        session
            .on_request_message(own, ControlMessage::PublishDone(done(2)))
            .unwrap();
        // A PUBLISH of the peer for another track uses the alias, and is
        // refused.
        let published = opened(&mut session, publish(0, b"b", 7));
        let code = request_error_code::UNINTERESTED;
        session
            .reject(published, code, 0, ReasonPhrase::default())
            .unwrap();
        assert_eq!(
            session.subscribed_alias(7),
            SubscribedAlias::Draining(&track(b"a"))
        );
    }

    #[test]
    fn open_requests_of_the_peer_are_bounded() {
        // The expectations below are spelled out for a limit of 256.
        assert_eq!(MAX_PEER_REQUESTS, 256);
        let mut session = server();
        for index in 0..256 {
            opened(&mut session, publish_namespace(index * 2));
        }
        assert_eq!(
            refusal(session.on_peer_request_opened(publish(512, b"a", 7))),
            Ok((0x9, None))
        );
        // The refused PUBLISH left its alias behind as unwanted.
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn open_requests_of_the_peer_are_bounded_by_what_the_session_was_set_to() {
        // (the limit set, how many requests are taken)
        let cases = [(3, 3), (300, 300), (0, 1)];
        for (most, taken) in cases {
            let mut session = server();
            session.set_most_peer_requests(most);
            for index in 0..taken {
                opened(&mut session, publish_namespace(index * 2));
            }
            assert_eq!(
                refusal(session.on_peer_request_opened(publish_namespace(taken * 2))),
                Ok((0x9, None)),
                "{most}"
            );
        }
    }

    #[test]
    fn a_refused_request_is_kept_while_the_session_has_room_for_it() {
        // The expectations below are spelled out for a limit of 256.
        assert_eq!(MAX_PEER_REQUESTS, 256);
        // (requests held before; whether the request that is refused is
        //  kept; requests held after)
        let cases = [(254, true, 255), (255, true, 256), (256, false, 256)];
        for (held, kept, after) in cases {
            let mut session = server();
            for index in 0..held {
                opened(&mut session, publish_namespace(index * 2));
            }
            // A FETCH that ends before it starts: the session refuses it
            // itself, with INVALID_RANGE (0x11).
            let fetch = standalone(1000, at(3, 4), at(3, 3));
            let (code, request) = refusal(session.on_peer_request_opened(fetch)).unwrap();
            assert_eq!((code, request.is_some()), (0x11, kept), "{held}");
            assert_eq!(session.peer_request_count(), after, "{held}");
        }
    }

    #[test]
    fn a_handle_of_another_session_is_refused() {
        let foreign = opened(&mut server(), subscribe(0, b"a"));
        let mut session = server();
        opened(&mut session, subscribe(0, b"a"));
        let other_session = UsageError("the request belongs to another session");
        assert_eq!(
            session.accept_subscribe(foreign, vec![], vec![]),
            Err(other_session)
        );
        assert_eq!(
            session.on_peer_request_message(foreign, update(2)),
            Err(RequestFault::Usage(other_session))
        );
        assert_eq!(session.peer_subscription_alias(foreign), None);
        session.on_peer_request_closed(foreign);
        // The request with the same number in this session is untouched.
        assert!(session.peer.subscribes_to(&track(b"a")));
    }
}
