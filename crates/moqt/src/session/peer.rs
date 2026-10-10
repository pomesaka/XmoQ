//! Requests the peer opens (§3.3, §5.1, §6.2, §10.1): the responder side.

use std::collections::{BTreeSet, HashMap};

use super::fetch::ends_before;
use super::{Request, RequestId, Role, Session, check_own_parameters, first_request_id};
use crate::aliases::SubscribedAlias;
use crate::kvp::KeyValuePair;
use crate::message::{
    ControlMessage, Fetch, FetchOk, FetchRange, Publish, PublishDone, PublishNamespace,
    ReasonPhrase, RequestError, RequestOk, Subscribe, SubscribeOk, publish_done_status,
    request_error_code,
};
use crate::name::{FullTrackName, Location};
use crate::params::{self, MessageKind, Parameter};
use crate::{Error, RequestFault, UsageError};

/// Requests of the peer that are open at once. One more is answered with
/// `REQUEST_ERROR` `EXCESSIVE_LOAD`.
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
    /// SUBSCRIBE of the peer that is open in this session; the session has
    /// checked that.
    Fetch(Fetch),
}

/// What the first message on a request stream of the peer led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerRequestOpened {
    /// A request to decide on: the caller answers with one of the `accept_*`
    /// methods or with [`Session::reject`], and reports the end of the stream
    /// with [`Session::on_peer_request_closed`].
    Request(PeerRequestId, PeerRequest),
    /// The session refused the request by itself. The caller sends `response`,
    /// a `REQUEST_ERROR`, and ends the stream with a FIN (§3.3.2). Nothing is
    /// kept for the request.
    Refused {
        /// The message to send on the request stream.
        response: ControlMessage,
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
    /// `REQUEST_UPDATE`, which this crate never applies. The caller sends
    /// `response`, a `REQUEST_ERROR` with `NOT_SUPPORTED` (§10.9), and then does
    /// what `then` says.
    UpdateRefused {
        /// The message to send on the request stream.
        response: ControlMessage,
        /// What §10.9.1 requires next.
        then: AfterRefusal,
    },
    /// Another `REQUEST_UPDATE` after one was refused. Nothing is sent (§10.9.1).
    UpdateAlreadyRefused,
    /// `REQUEST_UPDATE` for a FETCH that was answered. The caller sends
    /// `response`, a `REQUEST_OK`; nothing of the FETCH changes.
    ///
    /// WHY accepted, where an update of a subscription is refused: a refusal
    /// has a price here that it does not have there. §10.9.1: "When a
    /// `REQUEST_UPDATE` fails for a FETCH, the publisher MUST reset the FETCH
    /// data stream", so a subscriber that only meant to change its priority
    /// would lose the fetch. What an update of a FETCH may carry is
    /// `SUBSCRIBER_PRIORITY` (§10.2.7), `AUTHORIZATION_TOKEN` (§10.2.2) and
    /// the two delivery timeouts (§10.2.3, §10.2.4), none of which anything
    /// here uses; any other parameter closes the session (§10.2.1).
    UpdateAccepted {
        /// The message to send on the request stream.
        response: ControlMessage,
    },
    /// `REQUEST_UPDATE` for a FETCH that has no answer yet. Nothing is sent
    /// now: responses carry no Request ID, so a `REQUEST_OK` sent first
    /// would be read as the answer to the FETCH. [`Session::accept_fetch`]
    /// says how many are owed.
    UpdateHeld,
    /// A message for a subscription this endpoint already ended with
    /// `PUBLISH_DONE`. It crossed the `PUBLISH_DONE` on the way; there is
    /// nothing to do.
    LateMessage,
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
    },
    PublishNamespace {
        established: bool,
    },
    Fetch(FetchPhase),
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
    /// `REQUEST_ERROR` was sent; the stream is not yet reported gone. What
    /// the peer sent before it saw the answer still arrives, and gets none.
    Rejected,
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
}

impl PeerRequests {
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
        }
    }

    /// §10.1: an ID of the wrong parity or a repeated one closes the session
    /// with `INVALID_REQUEST_ID`.
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

    /// The Start Location of a FETCH of the peer, where the FETCH gives one,
    /// or the code to refuse the FETCH with.
    fn start_of(&self, range: &FetchRange) -> Result<Option<Location>, u64> {
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
                if !self.is_open_subscribe(*joining_request_id) {
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
        // FIXME: give a Retry Interval where the request can succeed later.
        // §10.6.2 has the sender of `EXCESSIVE_LOAD` say when to retry; 0 says
        // "do not retry" for every code.
        retry_interval: 0,
        error_reason: ReasonPhrase::default(),
        redirect: None,
    })
}

const NOT_PENDING: UsageError = UsageError("the request is not waiting for a response");

impl Session {
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
        let request_id = match &first {
            ControlMessage::Subscribe(message) => message.request_id,
            ControlMessage::Publish(message) => message.request_id,
            ControlMessage::PublishNamespace(message) => message.request_id,
            ControlMessage::Fetch(message) => message.request_id,
            ControlMessage::UnsupportedRequest { request_id, .. } => *request_id,
            _ => {
                return Err(Error::ProtocolViolation(
                    "request stream did not begin with a request",
                ));
            }
        };
        self.peer.take_id(request_id)?;
        let refused = |error_code| {
            Ok(PeerRequestOpened::Refused {
                response: request_error(error_code),
            })
        };
        let id = PeerRequestId {
            session: self.tag,
            id: request_id,
        };

        let (state, request) = match first {
            ControlMessage::Subscribe(message) => {
                // §5.1: at most one subscription to a track in each role.
                if self.is_publishing(&message.full_track_name) {
                    return refused(request_error_code::DUPLICATE_SUBSCRIPTION);
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
                    return refused(request_error_code::DUPLICATE_SUBSCRIPTION);
                }
                // §2.5.1: this crate understands no Mandatory Track Property.
                if message
                    .track_properties
                    .iter()
                    .any(KeyValuePair::is_mandatory_track_property)
                {
                    return refused(request_error_code::UNSUPPORTED_EXTENSION);
                }
                // The alias means the track before the answer: §10.10 lets the
                // publisher send Objects "possibly before PUBLISH_OK".
                self.subscribed_aliases
                    .offer(message.track_alias, message.full_track_name.clone());
                let state = PeerRequestState::Publish {
                    full_track_name: message.full_track_name.clone(),
                    track_alias: message.track_alias,
                    established: false,
                };
                (state, PeerRequest::Publish(message))
            }
            ControlMessage::PublishNamespace(message) => (
                PeerRequestState::PublishNamespace { established: false },
                PeerRequest::PublishNamespace(message),
            ),
            ControlMessage::Fetch(message) => {
                let start = match self.peer.start_of(&message.range) {
                    Ok(start) => start,
                    Err(error_code) => return refused(error_code),
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
            // The types `request_id` was read from above, less the ones handled.
            _ => return refused(request_error_code::NOT_SUPPORTED),
        };
        if self.peer.live.len() >= MAX_PEER_REQUESTS {
            self.forget_peer_request(&state);
            return refused(request_error_code::EXCESSIVE_LOAD);
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
        let track_alias = self.next_free_alias()?;
        let Some(PeerRequestState::Subscribe { phase, .. }) = self.peer_request_mut(request)?
        else {
            return Err(NOT_PENDING);
        };
        if *phase != SubscribePhase::Pending {
            return Err(NOT_PENDING);
        }
        *phase = SubscribePhase::Established { track_alias };
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
                | PeerRequestState::PublishNamespace { established },
            ) => !*established,
            Some(PeerRequestState::Fetch(phase)) => {
                matches!(phase, FetchPhase::Pending { .. })
            }
            None => false,
        };
        if !pending {
            return Err(NOT_PENDING);
        }
        // WHY a rejected FETCH stays where the other requests go at once: a
        // REQUEST_UPDATE for it can be on its way, as the answer to it is
        // held back until the FETCH is answered. One for a request that is
        // gone would close the session.
        if let Some(PeerRequestState::Fetch(phase)) = self.peer_request_mut(request)? {
            *phase = FetchPhase::Rejected;
        } else {
            self.end_peer_request(request.id);
        }
        Ok(ControlMessage::RequestError(RequestError {
            error_code,
            retry_interval,
            error_reason,
            redirect: None,
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
            } | PeerRequestState::Fetch(FetchPhase::Rejected)
        );
        let is_fetch = matches!(state, PeerRequestState::Fetch(_));
        // §10.1: a REQUEST_UPDATE consumes a Request ID of its sender, also
        // when the message itself is ignored below.
        if let ControlMessage::RequestUpdate(update) = &message {
            self.peer.take_id(update.request_id)?;
        }
        if ended {
            return Ok(PeerRequestEvent::LateMessage);
        }
        match message {
            // §10.2.1, with what §10.2 lets the update of a FETCH carry.
            ControlMessage::RequestUpdate(update)
                if is_fetch
                    && !params::all_allowed(&update.parameters, MessageKind::FetchUpdate) =>
            {
                Err(Error::ProtocolViolation("parameter not allowed in this message").into())
            }
            ControlMessage::RequestUpdate(_) => Ok(self.answer_update(request.id)),
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

    /// Records that a request stream of the peer is gone: finished, reset,
    /// stopped or lost with the connection, by either endpoint (§3.3.2). Call it
    /// for every stream that led to [`PeerRequestOpened::Request`], or the
    /// request stays open in this session. Does nothing for a request that is
    /// already forgotten, or for a handle of another session.
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

    fn answer_update(&mut self, request_id: u64) -> PeerRequestEvent {
        let refused = |then| PeerRequestEvent::UpdateRefused {
            response: request_error(request_error_code::NOT_SUPPORTED),
            then,
        };
        match self.peer.live.get_mut(&request_id) {
            Some(PeerRequestState::Fetch(FetchPhase::Answered)) => {
                PeerRequestEvent::UpdateAccepted {
                    response: ControlMessage::RequestOk(RequestOk::default()),
                }
            }
            Some(PeerRequestState::Fetch(FetchPhase::Pending { held_updates, .. })) => {
                // Saturating: every update used up a Request ID, of which
                // there are fewer than this counts.
                *held_updates = held_updates.saturating_add(1);
                PeerRequestEvent::UpdateHeld
            }
            // Sorted out before this is called.
            Some(PeerRequestState::Fetch(FetchPhase::Rejected)) => PeerRequestEvent::LateMessage,
            Some(PeerRequestState::Subscribe { phase, .. }) => match *phase {
                SubscribePhase::Established { track_alias } => {
                    *phase = SubscribePhase::UpdateRefused { track_alias };
                    refused(AfterRefusal::PublishDone)
                }
                SubscribePhase::UpdateRefused { .. } => PeerRequestEvent::UpdateAlreadyRefused,
                // WHY end the request: responses carry no Request ID, so a
                // REQUEST_ERROR sent before the response to the request itself
                // can only be read as that response.
                SubscribePhase::Pending | SubscribePhase::Done => {
                    self.end_peer_request(request_id);
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
            Some(PeerRequestState::Publish { .. } | PeerRequestState::PublishNamespace { .. })
            | None => {
                self.end_peer_request(request_id);
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
            | PeerRequestState::Fetch(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::RequestUpdate;
    use crate::name::TrackNamespace;
    use crate::session::RequestState;
    use crate::session::Transport;

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

    fn refusal(error_code: u64) -> PeerRequestOpened {
        PeerRequestOpened::Refused {
            response: request_error(error_code),
        }
    }

    const NOT_ALLOWED: RequestFault = RequestFault::Peer(Error::ProtocolViolation(
        "message not allowed in this request state",
    ));

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

    #[test]
    fn requests_this_crate_does_not_implement_are_refused_not_fatal() {
        let mut session = server();
        // TRACK_STATUS, with Request ID 0.
        assert_eq!(
            session.on_peer_request_opened(ControlMessage::UnsupportedRequest {
                message_type: 0x0d,
                request_id: 0,
            }),
            Ok(refusal(0x3))
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
                Some(code) => assert_eq!(result, Ok(refusal(code)), "{fetch:?}"),
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
                Ok(PeerRequestEvent::UpdateHeld)
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
                response: request_ok
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
            Ok(PeerRequestEvent::UpdateHeld)
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
        // (the parameters of the update; whether the session goes on).
        let cases = [
            (vec![], true),
            (vec![Parameter::SubscriberPriority(3)], true),
            (vec![Parameter::AuthorizationToken(vec![1])], true),
            (vec![Parameter::ObjectDeliveryTimeout(5)], true),
            (vec![Parameter::SubgroupDeliveryTimeout(5)], true),
            // "REQUEST_UPDATE (for a subscription)": §10.2.12, §10.2.13.
            (vec![Parameter::Forward(false)], false),
            (vec![Parameter::NewGroupRequest(0)], false),
            (
                vec![Parameter::SubscriberPriority(3), Parameter::Forward(true)],
                false,
            ),
        ];
        for (parameters, goes_on) in cases {
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
                        }
                    } else {
                        PeerRequestEvent::UpdateHeld
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
            session.on_peer_request_opened(subscribe(2, b"a")),
            Ok(refusal(0x19))
        );
        opened(&mut session, publish(4, b"b", 7));
        assert_eq!(
            session.on_peer_request_opened(publish(6, b"b", 8)),
            Ok(refusal(0x19))
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
            session.on_peer_request_opened(publish(0, b"a", 7)),
            Ok(refusal(0x19))
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

    #[test]
    fn a_publish_this_endpoint_cannot_use_is_refused() {
        let mut session = server();
        let mut message = Publish {
            request_id: 0,
            full_track_name: track(b"a"),
            track_alias: 7,
            parameters: vec![],
            track_properties: vec![KeyValuePair::varint(0x4000, 1).unwrap()],
        };
        assert_eq!(
            session.on_peer_request_opened(ControlMessage::Publish(message.clone())),
            Ok(refusal(0x33))
        );
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Unknown);

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
            session.on_peer_request_message(request, update(2)),
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
            session.on_peer_request_opened(publish(514, b"a", 7)),
            Ok(refusal(0x9))
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
            session.on_peer_request_opened(publish(512, b"a", 7)),
            Ok(refusal(0x9))
        );
        // The refused PUBLISH left its alias behind as unwanted.
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
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
