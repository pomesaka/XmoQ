//! Session and request state (§3.3, §5.1, §6.2, §10.1, §11.1).
//!
//! This file holds the SETUP exchange and the requests this endpoint opens;
//! [`peer`] holds the requests the peer opens.

mod fetch;
mod peer;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

pub use fetch::{FetchStreamOpened, NO_FETCH_FOR_THE_STREAM};
pub use peer::{
    AfterRefusal, MAX_PEER_REQUESTS, PeerRequest, PeerRequestEvent, PeerRequestId,
    PeerRequestOpened, REQUEST_IDS_AHEAD_LIMIT,
};

pub use crate::aliases::SubscribedAlias;
use crate::aliases::SubscribedAliases;
use crate::immutable::{TrackRefusal, track_properties_refusal};
use crate::kvp::KeyValuePair;
use crate::message::{
    ControlMessage, GoAway, Publish, PublishDone, PublishNamespace, ReasonPhrase, RequestError,
    RequestOk, RequestUpdate, Setup, Subscribe, SubscribeNamespace, SubscribeOk,
    publish_done_status, request_error_code, setup_option,
};
use crate::name::{FullTrackName, Location, TrackNamespace};
use crate::params::{self, MessageKind, Parameter};
use crate::{Error, RequestFault, UsageError};

/// Setup Options this crate knows to be single-instance. §10.3: a sender MUST NOT
/// repeat an option unless its definition allows it, and only AUTHORIZATION TOKEN
/// does (§10.3.1.4). Options this crate does not know are left to the caller.
const SINGLE_SETUP_OPTIONS: [u64; 4] = [
    setup_option::PATH,
    setup_option::MAX_AUTH_TOKEN_CACHE_SIZE,
    setup_option::AUTHORITY,
    setup_option::MOQT_IMPLEMENTATION,
];

/// Which end of the session this endpoint is. It decides the parity of Request
/// IDs (§10.1) and which Setup Options are legal (§10.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The endpoint that opened the connection.
    Client,
    /// The endpoint that accepted it.
    Server,
}

/// What carries the session (§3.1). AUTHORITY and PATH exist only on native QUIC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// WebTransport over HTTP/3.
    WebTransport,
    /// A QUIC connection with the `moqt-18` ALPN.
    NativeQuic,
}

/// Requests this endpoint ended with `PUBLISH_DONE` that are remembered until
/// their stream is reported gone, oldest dropped first. A caller that never
/// reports the stream gone cannot make the session grow without bound; a
/// message for a request that was dropped is then a protocol violation.
const ENDED_BY_PUBLISH_DONE_LIMIT: usize = 64;

/// Gives every [`Session`] of the process a number of its own, which its
/// request handles carry.
static NEXT_SESSION_TAG: AtomicU64 = AtomicU64::new(0);

/// Names one request this endpoint opened. Only [`Session`] creates these.
///
/// A handle belongs to the `Session` that returned it. A relay holds many
/// sessions whose Request IDs repeat, so every handle carries the number of its
/// session, and a session refuses a handle of another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(u64, u64);

impl RequestId {
    /// The Request ID as it appears on the wire (§10.1).
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }
}

/// `REQUEST_UPDATE`s of one subscription that may wait for their answer.
/// **Provisional.** WHY a limit: each is kept until its answer comes, and a
/// publisher that answers none would have the count grow with every call.
pub const MAX_UNANSWERED_UPDATES: u32 = 16;

/// States of a request after it is sent (§5.1 for subscriptions; §6.2 gives
/// `PUBLISH_NAMESPACE` the same single response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    /// Sent, no response yet.
    Pending,
    /// Accepted by the peer.
    Established,
    /// Rejected, done or cancelled. No further message is expected.
    Terminated,
}

/// A request that is Pending or Established. Terminated requests are dropped.
#[derive(Debug)]
enum Request {
    /// `track_alias` is the alias the publisher assigned in `SUBSCRIBE_OK`, so it
    /// is set exactly when the subscription is Established.
    Subscribe {
        full_track_name: FullTrackName,
        track_alias: Option<u64>,
        /// The `REQUEST_UPDATE`s this endpoint sent
        /// ([`Session::update_subscription`]).
        updates: SentUpdates,
    },
    Publish {
        full_track_name: FullTrackName,
        track_alias: u64,
        state: PublishState,
    },
    PublishNamespace {
        established: bool,
    },
    /// The namespaces the peer named on its stream (NAMESPACE) and has not
    /// withdrawn (`NAMESPACE_DONE`), by their suffix. At most
    /// [`MAX_NAMESPACES_OF_A_SUBSCRIPTION`].
    SubscribeNamespace {
        prefix: TrackNamespace,
        established: bool,
        named: HashSet<TrackNamespace>,
    },
    Fetch(fetch::OwnFetch),
    /// A `TRACK_STATUS`, which its one answer ends (§10.14).
    TrackStatus,
}

/// The namespaces a `SUBSCRIBE_NAMESPACE` of this endpoint keeps, that the
/// peer named and has not withdrawn. One more ends the request
/// ([`RequestEvent::TooManyNamespaces`]). Decided on 2026-10-24. WHY a limit: each is kept
/// for its `NAMESPACE_DONE` (§10.18: "If a subscriber receives a
/// `NAMESPACE_DONE` before the corresponding NAMESPACE, it MUST close the
/// session"), and the peer says how many there are. WHY 4,096: as many as the
/// relay tells one request of (`MAX_NAMESPACES_OF_A_WATCH` of
/// `xmoq-relay-core`).
pub const MAX_NAMESPACES_OF_A_SUBSCRIPTION: usize = 4096;

/// Where a PUBLISH this endpoint sent stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishState {
    Pending,
    Established,
    /// A `REQUEST_UPDATE` was refused. §10.9.1 leaves one way on: ending the
    /// subscription with `PUBLISH_DONE` and `UPDATE_FAILED`. Kept as a state so
    /// that the session, not the caller's memory, holds the caller to it.
    UpdateRefused,
}

impl Request {
    fn is_established(&self) -> bool {
        match self {
            Self::Subscribe { track_alias, .. } => track_alias.is_some(),
            Self::Publish { state, .. } => *state != PublishState::Pending,
            Self::PublishNamespace { established }
            | Self::SubscribeNamespace { established, .. } => *established,
            Self::Fetch(fetch) => fetch.answered,
            Self::TrackStatus => false,
        }
    }
}

/// What a message on a request stream means for the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestEvent {
    /// `SUBSCRIBE_OK`: Objects for the track arrive under `track_alias`.
    SubscribeEstablished {
        /// The alias Subgroup Headers of the track carry.
        track_alias: u64,
        /// Message Parameters of the `SUBSCRIBE_OK`.
        parameters: Vec<Parameter>,
        /// Track Properties of the `SUBSCRIBE_OK`.
        track_properties: Vec<KeyValuePair>,
    },
    /// The Track Properties of the `SUBSCRIBE_OK` keep the track from being
    /// processed: a Mandatory Track Property, none of which this crate
    /// understands (§2.5.1), or Immutable Properties that make it malformed
    /// (§12.7). The request is Terminated and the caller has to cancel the
    /// stream, with `MALFORMED_TRACK` for a malformed track (§2.4.2: "it MUST
    /// cancel any corresponding subscription") and `CANCELLED` otherwise.
    TrackNotProcessed {
        /// Why.
        refusal: TrackRefusal,
        /// The alias the `SUBSCRIBE_OK` assigned. The publisher may already have
        /// opened subgroup streams under it; the session reports it as
        /// [`SubscribedAlias::Retired`], so the caller can stop them (§10.11).
        track_alias: u64,
    },
    /// `PUBLISH_OK`: the subscriber accepted the track.
    PublishEstablished {
        /// The Forward State the subscriber set; Objects are sent only when true.
        forward: bool,
        /// Message Parameters of the `PUBLISH_OK`.
        parameters: Vec<Parameter>,
    },
    /// `PUBLISH_NAMESPACE_OK`: the peer accepted the namespace.
    PublishNamespaceEstablished,
    /// The `REQUEST_OK` of a `SUBSCRIBE_NAMESPACE` (§10.18): the namespaces
    /// follow.
    NamespacesSubscribed,
    /// NAMESPACE (§10.16) on the stream of a `SUBSCRIBE_NAMESPACE`: a
    /// namespace under its prefix, whole (the prefix and the suffix).
    Namespace(TrackNamespace),
    /// `NAMESPACE_DONE` (§10.17): the namespace is withdrawn, whole.
    NamespaceDone(TrackNamespace),
    /// A NAMESPACE named one namespace more than a `SUBSCRIBE_NAMESPACE` keeps
    /// ([`MAX_NAMESPACES_OF_A_SUBSCRIPTION`]). The request is over in the
    /// session, and the caller has to cancel its stream (§6.1: "can be
    /// cancelled by closing the stream with either a FIN or `RESET_STREAM`").
    /// WHY the request alone, and not the session: nothing else of the
    /// session depends on what that request was told.
    TooManyNamespaces {
        /// The cap, in namespaces.
        limit: usize,
    },
    /// `FETCH_OK`: the peer serves the FETCH.
    FetchEstablished {
        /// Whether the End Location is the last Object of the track.
        end_of_track: bool,
        /// The end of what the fetch stream covers, in the encoding of a
        /// FETCH: the last Object plus 1, or Object 0 for the whole Group.
        end_location: Location,
        /// Track Properties of the `FETCH_OK`.
        track_properties: Vec<KeyValuePair>,
    },
    /// The Track Properties of the `FETCH_OK` keep the track from being
    /// processed, as for [`RequestEvent::TrackNotProcessed`]. The request is
    /// Terminated and the caller has to cancel its stream and stop the fetch
    /// stream, with the codes said there.
    FetchNotProcessed {
        /// Why.
        refusal: TrackRefusal,
    },
    /// `TRACK_STATUS_OK` (§10.14, a `REQUEST_OK`): what the peer would have
    /// set in a `SUBSCRIBE_OK`. The request is Terminated; the peer closes
    /// its stream with a FIN.
    TrackStatusAnswered {
        /// Message Parameters: `LARGEST_OBJECT` where Objects have been
        /// published (§10.2.11).
        parameters: Vec<Parameter>,
        /// Track Properties (§10.5).
        track_properties: Vec<KeyValuePair>,
    },
    /// `REQUEST_ERROR`: the peer rejected the request, which is now Terminated.
    Rejected(RequestError),
    /// `PUBLISH_DONE` ended a subscription this endpoint asked for. The request
    /// is Terminated; the Track Alias keeps draining, see
    /// [`SubscribedAlias::Draining`].
    SubscriptionDone(PublishDone),
    /// The publisher accepted a `REQUEST_UPDATE` this endpoint sent
    /// ([`Session::update_subscription`]), with the parameters of its
    /// `REQUEST_OK`.
    UpdateAccepted {
        /// `EXPIRES` and `LARGEST_OBJECT`, where the publisher sent them.
        parameters: Vec<Parameter>,
    },
    /// The publisher refused a `REQUEST_UPDATE` this endpoint sent. The
    /// subscription is still there: its `PUBLISH_DONE` with `UPDATE_FAILED`
    /// follows (§10.9.1) and ends it as any other.
    UpdateFailed(RequestError),
    /// The answer to a `REQUEST_UPDATE` this endpoint sent before the
    /// publisher refused an earlier one ([`Self::UpdateFailed`]). There is
    /// nothing to do: the subscription is ending.
    UpdateAnsweredAfterRefusal,
    /// `REQUEST_UPDATE` from the subscriber of a track this endpoint
    /// publishes, with no parameter but delivery timeouts:
    /// as [`PeerRequestEvent::SubscriptionUpdated`], for a subscription
    /// that a PUBLISH made. The caller sends `response`, a `REQUEST_OK`.
    SubscriptionUpdated {
        /// The message to send on the request stream.
        response: ControlMessage,
        /// The `SUBGROUP_DELIVERY_TIMEOUT` of the update in milliseconds,
        /// 0 for no timeout; `None` when the update had none.
        subgroup_delivery_timeout: Option<u64>,
        /// The `NEW_GROUP_REQUEST` of the update (§10.2.13); `None` when
        /// the update had none.
        new_group_request: Option<u64>,
    },
    /// `REQUEST_UPDATE` from the subscriber of a track this endpoint publishes
    /// that carries any other parameter. The caller sends `response`, a
    /// `REQUEST_ERROR` with `NOT_SUPPORTED` (§10.9), and then has to end the
    /// subscription with [`Session::publish_done`] and the status
    /// `UPDATE_FAILED` (§10.9.1). Until it does, the session takes nothing else
    /// for this request: `publish_done` with another status is an error.
    UpdateRefused {
        /// The message to send on the request stream.
        response: ControlMessage,
    },
    /// Another `REQUEST_UPDATE` after one was refused. Nothing is sent: §10.9.1
    /// lets a receiver answer several updates with a single `REQUEST_ERROR`,
    /// and the subscription is ending anyway.
    UpdateAlreadyRefused,
    /// A message for a PUBLISH that this endpoint had already ended with
    /// `PUBLISH_DONE`. It crossed the `PUBLISH_DONE` on the way; there is
    /// nothing to do.
    LateMessage,
    /// A GOAWAY on the stream of the request (§10.4): "the endpoint SHOULD
    /// re-issue that specific request on a session at the specified URI (or
    /// the current session if no URI is provided), and close the old request
    /// stream". The request is as it was; sending it again and ending this
    /// one are the caller's.
    GoAway(GoAway),
}

/// What a message on the peer's control stream was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    /// The peer's SETUP, the first message.
    Setup(Setup),
    /// The peer's GOAWAY (§10.4): it means to close the session. The session
    /// opens no request after it (§10.4: "Upon receiving a GOAWAY on the
    /// control stream, an endpoint SHOULD NOT initiate new requests to the
    /// peer"); what is open stays as it is ("The GOAWAY message does not
    /// impact subscription state"). Where to go next is the caller's.
    GoAway(GoAway),
}

/// The state of one MOQT session, as far as the endpoint that opens requests
/// needs it: the SETUP exchange, Request IDs, Track Aliases and open requests.
#[derive(Debug)]
pub struct Session {
    role: Role,
    transport: Transport,
    setup_sent: bool,
    peer_setup_received: bool,
    /// The GOAWAY messages on the control streams.
    go_aways: GoAways,
    /// The requests the peer sent GOAWAY on the stream of (§10.4), by whether
    /// this endpoint opened them and their Request ID.
    moved: HashSet<(bool, u64)>,
    next_request_id: u64,
    /// Requests that are Pending or Established, by Request ID.
    requests: HashMap<u64, Request>,
    subscribed_aliases: SubscribedAliases,
    /// Track Aliases this endpoint chose in PUBLISH. A separate space: each
    /// publisher numbers its own tracks (§11.1).
    published_aliases: HashSet<u64>,
    /// The alias the next accepted SUBSCRIBE of the peer gets.
    next_published_alias: u64,
    /// PUBLISH requests this endpoint ended with `PUBLISH_DONE` whose streams
    /// are not yet reported gone. What still arrives on them was sent before
    /// the peer saw the `PUBLISH_DONE`.
    ended_by_publish_done: VecDeque<u64>,
    /// The number this session's request handles carry.
    tag: u64,
    peer: peer::PeerRequests,
}

/// The GOAWAY messages on the two control streams of a session (§10.4).
#[derive(Debug, Default)]
struct GoAways {
    /// The peer sent one.
    received: bool,
    /// This endpoint sent one ([`Session::go_away`]).
    sent: bool,
}

impl Session {
    /// A session before any SETUP is exchanged.
    #[must_use]
    pub fn new(role: Role, transport: Transport) -> Self {
        Self {
            role,
            transport,
            setup_sent: false,
            peer_setup_received: false,
            go_aways: GoAways::default(),
            moved: HashSet::new(),
            // §10.1: the client uses even Request IDs from 0, the server odd from 1.
            next_request_id: first_request_id(role),
            requests: HashMap::new(),
            subscribed_aliases: SubscribedAliases::default(),
            published_aliases: HashSet::new(),
            next_published_alias: 0,
            ended_by_publish_done: VecDeque::new(),
            // Relaxed: the numbers only have to differ.
            tag: NEXT_SESSION_TAG.fetch_add(1, Ordering::Relaxed),
            peer: peer::PeerRequests::new(role),
        }
    }

    /// The SETUP to send first on this endpoint's control stream.
    ///
    /// # Errors
    ///
    /// Fails when called twice, when AUTHORITY or PATH is given by anything but
    /// a client on native QUIC, or when a single-instance option is repeated.
    pub fn setup(
        &mut self,
        setup_options: Vec<KeyValuePair>,
    ) -> Result<ControlMessage, UsageError> {
        if self.setup_sent {
            return Err(UsageError("SETUP was already sent"));
        }
        match misplaced_setup_option(&setup_options, self.role, self.transport) {
            Some(MisplacedOption::Authority) => {
                return Err(UsageError("AUTHORITY is for a client on native QUIC"));
            }
            Some(MisplacedOption::Path) => {
                return Err(UsageError("PATH is for a client on native QUIC"));
            }
            None => {}
        }
        let repeated = SINGLE_SETUP_OPTIONS.iter().any(|ty| {
            setup_options
                .iter()
                .filter(|option| option.ty() == *ty)
                .count()
                > 1
        });
        if repeated {
            return Err(UsageError("setup option repeated"));
        }
        self.setup_sent = true;
        Ok(ControlMessage::Setup(Setup { setup_options }))
    }

    /// Which end of the session this endpoint is.
    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Whether both SETUP messages have been exchanged.
    #[must_use]
    pub fn is_established(&self) -> bool {
        self.setup_sent && self.peer_setup_received
    }

    /// How many requests of the peer may be open at once, in place of
    /// [`MAX_PEER_REQUESTS`]; at least 1. For a request that is open already
    /// it changes nothing.
    pub fn set_most_peer_requests(&mut self, most: usize) {
        self.peer.most = most.max(1);
    }

    /// Whether the peer sent GOAWAY on its control stream. Requests are not
    /// opened after it.
    #[must_use]
    pub fn peer_going_away(&self) -> bool {
        self.go_aways.received
    }

    /// Whether this endpoint sent GOAWAY on its control stream. Every request
    /// of the peer is refused with `GOING_AWAY` after it.
    #[must_use]
    pub fn going_away(&self) -> bool {
        self.go_aways.sent
    }

    /// The GOAWAY to send on this endpoint's control stream (§10.4), with
    /// `new_session_uri` (empty for the URI the client used) and `timeout_ms`.
    /// The session refuses every request of the peer that comes after it
    /// with `GOING_AWAY` by itself ([`Session::on_peer_request_opened`]).
    ///
    /// The Request ID is the smallest Request ID of the peer the session has
    /// not seen. WHY that one, though IDs above it may have been seen: §10.4
    /// has "the smallest peer Request ID that was not or might not have been
    /// processed", and a request above it that came is answered on its own
    /// stream ("their status can be determined from the response on each
    /// request stream").
    ///
    /// # Errors
    ///
    /// Before SETUP was sent, for a second GOAWAY (§10.4: the receiver "MUST
    /// close the session with a `PROTOCOL_VIOLATION` ... if it receives more
    /// than one GOAWAY on the control stream"), and for a URI from a client
    /// ("A client MUST send a zero-length New Session URI"). A URI over 8,192
    /// bytes is refused where the message is encoded.
    pub fn go_away(
        &mut self,
        new_session_uri: Vec<u8>,
        timeout_ms: u64,
    ) -> Result<ControlMessage, UsageError> {
        if !self.setup_sent {
            return Err(UsageError("GOAWAY before SETUP"));
        }
        if self.go_aways.sent {
            return Err(UsageError("a second GOAWAY"));
        }
        if self.role == Role::Client && !new_session_uri.is_empty() {
            return Err(UsageError("a GOAWAY of a client with a URI"));
        }
        self.go_aways.sent = true;
        Ok(ControlMessage::GoAway(GoAway {
            new_session_uri,
            timeout_ms,
            request_id: Some(self.peer.lowest_unseen()),
        }))
    }

    /// Feeds a message read from the peer's control stream: its SETUP first,
    /// and a GOAWAY after it.
    ///
    /// # Errors
    ///
    /// A protocol error when the first message is not SETUP, when the SETUP holds
    /// an option its sender may not use, for a GOAWAY that breaks a rule of
    /// §10.4, and for any other message after the SETUP. The session then has
    /// to be closed with [`Error::termination_code`].
    pub fn on_control_message(&mut self, message: ControlMessage) -> Result<ControlEvent, Error> {
        if self.peer_setup_received {
            // After SETUP only GOAWAY belongs on a control stream (Table 5).
            let ControlMessage::GoAway(go_away) = message else {
                return Err(Error::ProtocolViolation(
                    "message not allowed on the control stream",
                ));
            };
            self.on_go_away(&go_away)?;
            return Ok(ControlEvent::GoAway(go_away));
        }
        let ControlMessage::Setup(setup) = message else {
            return Err(Error::ProtocolViolation(
                "control stream did not begin with SETUP",
            ));
        };
        let peer_role = match self.role {
            Role::Client => Role::Server,
            Role::Server => Role::Client,
        };
        match misplaced_setup_option(&setup.setup_options, peer_role, self.transport) {
            Some(MisplacedOption::Authority) => return Err(Error::InvalidAuthority),
            Some(MisplacedOption::Path) => return Err(Error::InvalidPath),
            None => {}
        }
        // WHY NOT reject a repeated option here, as `setup` does for the ones
        // this endpoint sends: §10.3 puts the rule on the sender only ("Senders
        // MUST NOT repeat ..."), and for receivers says just that duplicates of
        // unknown options MUST be allowed. Message Parameters are different:
        // §10.2 tells receivers to check them.
        self.peer_setup_received = true;
        Ok(ControlEvent::Setup(setup))
    }

    /// The rules §10.4 sets for a GOAWAY on the control stream.
    fn on_go_away(&mut self, go_away: &GoAway) -> Result<(), Error> {
        // "The endpoint MUST close the session with a PROTOCOL_VIOLATION if
        // it receives more than one GOAWAY on the control stream".
        if self.go_aways.received {
            return Err(Error::ProtocolViolation("a second GOAWAY"));
        }
        // "If a server receives a GOAWAY with a non-zero New Session URI
        // Length it MUST close the session with a PROTOCOL_VIOLATION."
        if self.role == Role::Server && !go_away.new_session_uri.is_empty() {
            return Err(Error::ProtocolViolation("a GOAWAY of a client with a URI"));
        }
        // "Request ID: Present only when sent on the control stream." WHY a
        // violation where it is missing: without it the receiver cannot tell
        // which of its requests the peer took (§10.4).
        let Some(request_id) = go_away.request_id else {
            return Err(Error::ProtocolViolation("a GOAWAY without a Request ID"));
        };
        // "If the parity of the Request ID does not match the receiver's
        // parity, the endpoint MUST close the session with INVALID_REQUEST_ID."
        if request_id % 2 != first_request_id(self.role) {
            return Err(Error::InvalidRequestId(request_id));
        }
        self.go_aways.received = true;
        Ok(())
    }

    /// Opens a subscription to a track. Returns the handle of the request and the
    /// SUBSCRIBE to send first on a new bidirectional stream.
    ///
    /// # Errors
    ///
    /// Fails when a parameter may not appear in SUBSCRIBE, and when this
    /// endpoint already is a subscriber of the track in this session (§5.1: at
    /// most one subscription to a track in each role).
    pub fn subscribe(
        &mut self,
        full_track_name: FullTrackName,
        parameters: Vec<Parameter>,
    ) -> Result<(RequestId, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::Subscribe)?;
        if self.is_subscribed_to(&full_track_name) {
            return Err(UsageError("already a subscriber of this track"));
        }
        let request_id = self.open(Request::Subscribe {
            full_track_name: full_track_name.clone(),
            track_alias: None,
            updates: SentUpdates::default(),
        })?;
        let message = ControlMessage::Subscribe(Subscribe {
            request_id: request_id.0,
            full_track_name,
            parameters,
        });
        Ok((request_id, message))
    }

    /// Asks the publisher to change parameters of an Established
    /// subscription this endpoint made with SUBSCRIBE (§10.9). Returns the
    /// `REQUEST_UPDATE` to send on the stream of that SUBSCRIBE; a parameter
    /// that is left out stays as it was. The answer comes as
    /// [`RequestEvent::UpdateAccepted`] or [`RequestEvent::UpdateFailed`],
    /// in the order the updates were sent.
    ///
    /// # Errors
    ///
    /// Fails for a request that is not an Established SUBSCRIBE of this
    /// endpoint, when a parameter may not appear in `REQUEST_UPDATE`, when
    /// [`MAX_UNANSWERED_UPDATES`] updates wait for their answer, and when
    /// the Request IDs are used up (§10.1: an update consumes one). **A
    /// publisher that answers none leaves the subscription unable to be
    /// updated after that many**: the session has no clock, and a caller
    /// that stops waiting does not make room.
    pub fn update_subscription(
        &mut self,
        request: RequestId,
        parameters: Vec<Parameter>,
    ) -> Result<ControlMessage, UsageError> {
        check_own_parameters(&parameters, MessageKind::RequestUpdate)?;
        self.check_own(request)?;
        let Some(Request::Subscribe {
            track_alias: Some(_),
            updates,
            ..
        }) = self.requests.get_mut(&request.0)
        else {
            return Err(UsageError("not an established subscription of a SUBSCRIBE"));
        };
        updates.check_room()?;
        let request_id = self.next_request_id;
        self.next_request_id = request_id
            .checked_add(2)
            .ok_or(UsageError("request ids are exhausted"))?;
        updates.sent();
        Ok(ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters,
        }))
    }

    /// Offers a track to the peer. Returns the handle of the request and the
    /// PUBLISH to send first on a new bidirectional stream.
    ///
    /// `track_alias` is this endpoint's choice and must not name another track it
    /// is publishing in this session (§11.1).
    ///
    /// # Errors
    ///
    /// Fails for an alias already in use, when a parameter may not appear in
    /// PUBLISH, and when this endpoint already is a publisher of the track in
    /// this session (§5.1).
    pub fn publish(
        &mut self,
        full_track_name: FullTrackName,
        track_alias: u64,
        parameters: Vec<Parameter>,
        track_properties: Vec<KeyValuePair>,
    ) -> Result<(RequestId, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::Publish)?;
        if self.published_aliases.contains(&track_alias) {
            return Err(UsageError("track alias already in use"));
        }
        if self.is_publishing(&full_track_name) {
            return Err(UsageError("already a publisher of this track"));
        }
        let request_id = self.open(Request::Publish {
            full_track_name: full_track_name.clone(),
            track_alias,
            state: PublishState::Pending,
        })?;
        self.published_aliases.insert(track_alias);
        let message = ControlMessage::Publish(Publish {
            request_id: request_id.0,
            full_track_name,
            track_alias,
            parameters,
            track_properties,
        });
        Ok((request_id, message))
    }

    /// Asks the peer for the namespaces under `prefix`, now and as they come
    /// and go (§6.1, §10.18). Returns the handle of the request and the
    /// `SUBSCRIBE_NAMESPACE` to send first on a new bidirectional stream.
    /// The answer comes as [`RequestEvent::NamespacesSubscribed`] or
    /// [`RequestEvent::Rejected`], and the namespaces as
    /// [`RequestEvent::Namespace`] and [`RequestEvent::NamespaceDone`].
    ///
    /// # Errors
    ///
    /// Fails when a parameter may not appear in `SUBSCRIBE_NAMESPACE`, when
    /// the peer sent GOAWAY, and when the Request IDs are used up.
    pub fn subscribe_namespace(
        &mut self,
        prefix: TrackNamespace,
        parameters: Vec<Parameter>,
    ) -> Result<(RequestId, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::SubscribeNamespace)?;
        let request_id = self.open(Request::SubscribeNamespace {
            prefix: prefix.clone(),
            established: false,
            named: HashSet::new(),
        })?;
        let message = ControlMessage::SubscribeNamespace(SubscribeNamespace {
            request_id: request_id.0,
            track_namespace_prefix: prefix,
            parameters,
        });
        Ok((request_id, message))
    }

    /// Announces a namespace. Returns the handle of the request and the
    /// `PUBLISH_NAMESPACE` to send first on a new bidirectional stream.
    ///
    /// # Errors
    ///
    /// Fails when a parameter may not appear in `PUBLISH_NAMESPACE`.
    pub fn publish_namespace(
        &mut self,
        track_namespace: TrackNamespace,
        parameters: Vec<Parameter>,
    ) -> Result<(RequestId, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::PublishNamespace)?;
        let request_id = self.open(Request::PublishNamespace { established: false })?;
        let message = ControlMessage::PublishNamespace(PublishNamespace {
            request_id: request_id.0,
            track_namespace,
            parameters,
        });
        Ok((request_id, message))
    }

    /// `TRACK_STATUS_OK`: `LARGEST_OBJECT` is the one parameter it may carry
    /// (§10.2.11), and its Track Properties are the track's (§10.5).
    fn on_track_status_ok(
        &mut self,
        request: RequestId,
        ok: RequestOk,
    ) -> Result<RequestEvent, Error> {
        if !params::all_allowed(&ok.parameters, MessageKind::TrackStatusOk) {
            return Err(Error::ProtocolViolation(
                "parameter not allowed in this message",
            ));
        }
        self.end_request(request);
        Ok(RequestEvent::TrackStatusAnswered {
            parameters: ok.parameters,
            track_properties: ok.track_properties,
        })
    }

    /// Opens a `TRACK_STATUS` (§10.14) for `full_track_name`. Returns the
    /// request handle and the message to send on a new request stream; the
    /// answer is [`RequestEvent::TrackStatusAnswered`] or
    /// [`RequestEvent::Rejected`].
    ///
    /// # Errors
    ///
    /// Fails for a parameter that may not appear in `TRACK_STATUS`: any but
    /// `AUTHORIZATION_TOKEN`.
    pub fn track_status(
        &mut self,
        full_track_name: FullTrackName,
        parameters: Vec<Parameter>,
    ) -> Result<(RequestId, ControlMessage), UsageError> {
        check_own_parameters(&parameters, MessageKind::TrackStatus)?;
        let request_id = self.open(Request::TrackStatus)?;
        let message = ControlMessage::TrackStatus(Subscribe {
            request_id: request_id.0,
            full_track_name,
            parameters,
        });
        Ok((request_id, message))
    }

    /// The state of a request, or `None` for a handle this session never issued.
    #[must_use]
    pub fn request_state(&self, request: RequestId) -> Option<RequestState> {
        if request.1 != self.tag {
            return None;
        }
        match self.requests.get(&request.0) {
            Some(live) if live.is_established() => Some(RequestState::Established),
            Some(_) => Some(RequestState::Pending),
            None if self.was_issued(request) => Some(RequestState::Terminated),
            None => None,
        }
    }

    /// Feeds a message read from the stream of `request`.
    ///
    /// # Errors
    ///
    /// [`RequestFault::Peer`] means the peer broke the protocol and the session
    /// has to be closed with [`Error::termination_code`]; the request is left as
    /// it was. A message for a request that is no longer open is such an error.
    /// [`RequestFault::Usage`] means `request` belongs to another session.
    pub fn on_request_message(
        &mut self,
        request: RequestId,
        message: ControlMessage,
    ) -> Result<RequestEvent, RequestFault> {
        self.check_own(request)?;
        if let ControlMessage::GoAway(go_away) = message {
            if !self.requests.contains_key(&request.0) {
                // It crossed this endpoint's end of the request, as any message
                // may (§5.1).
                if self.ended_by_publish_done.contains(&request.0) {
                    return Ok(RequestEvent::LateMessage);
                }
                return Err(Error::ProtocolViolation("message after the request ended").into());
            }
            self.check_request_go_away(request.0, &go_away, true)?;
            return Ok(RequestEvent::GoAway(go_away));
        }
        Ok(self.on_own_request_message(request, message)?)
    }

    /// The rules §10.4 sets for a GOAWAY on the stream of request `id`.
    /// `own` tells the requests this endpoint opened from the peer's, whose
    /// IDs may be the same numbers.
    fn check_request_go_away(&mut self, id: u64, go_away: &GoAway, own: bool) -> Result<(), Error> {
        // "Request ID: Present only when sent on the control stream."
        if go_away.request_id.is_some() {
            return Err(Error::ProtocolViolation(
                "a GOAWAY on a request stream with a Request ID",
            ));
        }
        // "If a server receives a GOAWAY with a non-zero New Session URI
        // Length it MUST close the session with a PROTOCOL_VIOLATION."
        if self.role == Role::Server && !go_away.new_session_uri.is_empty() {
            return Err(Error::ProtocolViolation("a GOAWAY of a client with a URI"));
        }
        // "The endpoint MUST close the session with a PROTOCOL_VIOLATION if
        // it receives more than one GOAWAY on the control stream or on a
        // single request stream." Only requests that are still there are
        // remembered, so the set is no larger than they are.
        let (requests, peer) = (&self.requests, &self.peer);
        self.moved.retain(|&(of_own, id)| {
            if of_own {
                requests.contains_key(&id)
            } else {
                peer.is_live(id)
            }
        });
        if !self.moved.insert((own, id)) {
            return Err(Error::ProtocolViolation(
                "a second GOAWAY on a request stream",
            ));
        }
        Ok(())
    }

    fn on_own_request_message(
        &mut self,
        request: RequestId,
        message: ControlMessage,
    ) -> Result<RequestEvent, Error> {
        let Some(live) = self.requests.get_mut(&request.0) else {
            // §5.1 lets a publisher end a subscription with PUBLISH_DONE while
            // it is Pending or Established, so the subscriber's PUBLISH_OK,
            // REQUEST_ERROR and any REQUEST_UPDATE (§10.9) can cross it.
            //
            // WHY NOT check that what crossed would have been legal: the
            // subscription is over on this side and nothing depends on it.
            // The Request ID of an update is another matter: §10.1 has it
            // consume one, whatever becomes of the update.
            if self.ended_by_publish_done.contains(&request.0) {
                if let ControlMessage::RequestUpdate(update) = &message {
                    self.peer.take_id(update.request_id)?;
                }
                return Ok(RequestEvent::LateMessage);
            }
            return Err(Error::ProtocolViolation("message after the request ended"));
        };
        check_update_of_publish(live, &message)?;
        let established = live.is_established();
        match (live, message) {
            (live, ControlMessage::RequestError(error)) if !established => {
                check_redirect(self.role, live, &error)?;
                self.end_request(request);
                Ok(RequestEvent::Rejected(error))
            }
            (Request::Subscribe { .. }, ControlMessage::SubscribeOk(ok)) if !established => {
                self.on_subscribe_ok(request, ok)
            }
            // The answers to the updates this endpoint sent (§10.9).
            (
                Request::Subscribe { updates, .. },
                answer @ (ControlMessage::RequestOk(_) | ControlMessage::RequestError(_)),
            ) if established && updates.unanswered > 0 => update_answered(updates, answer),
            (
                Request::Subscribe {
                    track_alias: Some(track_alias),
                    ..
                },
                ControlMessage::PublishDone(done),
            ) => {
                // §10.11: PUBLISH_DONE travels on the request stream and
                // overtakes Objects still in flight, so the request ends here
                // and the alias goes on until the counted streams have ended.
                let track_alias = *track_alias;
                self.requests.remove(&request.0);
                self.subscribed_aliases
                    .drain(track_alias, done.stream_count);
                Ok(RequestEvent::SubscriptionDone(done))
            }
            (Request::Publish { state, .. }, ControlMessage::RequestOk(ok))
                if *state == PublishState::Pending =>
            {
                check_request_ok(&ok.parameters, &ok.track_properties, MessageKind::PublishOk)?;
                *state = PublishState::Established;
                Ok(RequestEvent::PublishEstablished {
                    forward: params::forward(&ok.parameters),
                    parameters: ok.parameters,
                })
            }
            // §10.9: the subscriber of a subscription established with PUBLISH
            // may update it, and gets exactly one REQUEST_OK or REQUEST_ERROR.
            // On any other request of this endpoint the peer is not the one
            // who may send an update, which the last arm rejects.
            //
            // §10.1: "Each SUBSCRIBE, PUBLISH, FETCH, SUBSCRIBE_NAMESPACE,
            // SUBSCRIBE_TRACKS, PUBLISH_NAMESPACE, REQUEST_UPDATE, and
            // TRACK_STATUS message consumes a Request ID", so the update's is
            // checked with the IDs of the requests the peer opens.
            (Request::Publish { state, .. }, ControlMessage::RequestUpdate(update))
                if *state == PublishState::Established =>
            {
                self.peer.take_id(update.request_id)?;
                Ok(answer_update_of_publish(state, &update))
            }
            (
                Request::Publish {
                    state: PublishState::UpdateRefused,
                    ..
                },
                ControlMessage::RequestUpdate(update),
            ) => {
                // It gets no answer, and its Request ID is used up all the same.
                self.peer.take_id(update.request_id)?;
                Ok(RequestEvent::UpdateAlreadyRefused)
            }
            (Request::PublishNamespace { established }, ControlMessage::RequestOk(ok))
                if !*established =>
            {
                check_request_ok(
                    &ok.parameters,
                    &ok.track_properties,
                    MessageKind::PublishNamespaceOk,
                )?;
                *established = true;
                Ok(RequestEvent::PublishNamespaceEstablished)
            }
            (Request::SubscribeNamespace { .. }, message) => {
                self.on_subscribe_namespace_message(request, message)
            }
            (Request::Fetch(_), ControlMessage::FetchOk(ok)) if !established => {
                self.on_fetch_ok(request, ok)
            }
            (Request::TrackStatus, ControlMessage::RequestOk(ok)) => {
                self.on_track_status_ok(request, ok)
            }
            // §5.1 and §6.2: exactly one response per request, and every other
            // message decoded here either opens a request or answers a different
            // kind.
            _ => Err(Error::ProtocolViolation(
                "message not allowed in this request state",
            )),
        }
    }

    /// A message on the stream of a `SUBSCRIBE_NAMESPACE` of this endpoint: its
    /// `REQUEST_OK` (a `REQUEST_ERROR` is taken before), and then NAMESPACE
    /// and `NAMESPACE_DONE`.
    fn on_subscribe_namespace_message(
        &mut self,
        request: RequestId,
        message: ControlMessage,
    ) -> Result<RequestEvent, Error> {
        let not_allowed = Error::ProtocolViolation("message not allowed in this request state");
        let Some(Request::SubscribeNamespace {
            prefix,
            established,
            named,
        }) = self.requests.get_mut(&request.0)
        else {
            return Err(not_allowed);
        };
        let event = match message {
            ControlMessage::RequestOk(ok) if !*established => answered_namespaces(established, &ok),
            message if *established => on_namespace(prefix, named, message),
            _ => Err(not_allowed),
        }?;
        if matches!(event, RequestEvent::TooManyNamespaces { .. }) {
            self.end_request(request);
        }
        Ok(event)
    }

    /// The `PUBLISH_DONE` that ends a subscription this endpoint opened with
    /// PUBLISH. The caller sends it after closing every data stream of the
    /// subscription, then closes the request stream (§10.11).
    ///
    /// Messages that still arrive for the request yield
    /// [`RequestEvent::LateMessage`] until [`Self::cancel`] reports the stream
    /// gone, which also lets the session forget the request.
    ///
    /// `status_code` says why the subscription ends (see
    /// [`crate::message::publish_done_status`]). `stream_count` is the number of
    /// data streams opened for it, 0 for none and 2^62 - 1 when unknown.
    ///
    /// # Errors
    ///
    /// Fails unless `request` is a PUBLISH that is Pending or Established, and
    /// for a status other than `UPDATE_FAILED` once a `REQUEST_UPDATE` was
    /// refused (§10.9.1).
    pub fn publish_done(
        &mut self,
        request: RequestId,
        status_code: u64,
        stream_count: u64,
        error_reason: ReasonPhrase,
    ) -> Result<ControlMessage, UsageError> {
        self.check_own(request)?;
        let Some(Request::Publish { state, .. }) = self.requests.get(&request.0) else {
            return Err(UsageError(
                "PUBLISH_DONE ends a live subscription opened with PUBLISH",
            ));
        };
        if *state == PublishState::UpdateRefused
            && status_code != publish_done_status::UPDATE_FAILED
        {
            return Err(UsageError(
                "a refused REQUEST_UPDATE ends the subscription with UPDATE_FAILED",
            ));
        }
        self.ended_by_publish_done.push_back(request.0);
        if self.ended_by_publish_done.len() > ENDED_BY_PUBLISH_DONE_LIMIT {
            self.ended_by_publish_done.pop_front();
        }
        self.end_request(request);
        Ok(ControlMessage::PublishDone(PublishDone {
            status_code,
            stream_count,
            error_reason,
        }))
    }

    /// Records that the stream of `request` was reset, stopped or lost, by either
    /// endpoint (§3.3.2). Call it whenever a request stream goes away without a
    /// terminal event, or the request stays open in this session. Does nothing
    /// for a request that already ended.
    ///
    /// A FIN of the peer is such an end for a request that has no answer
    /// yet. For a FETCH that has its `FETCH_OK` it is not, and this is not
    /// called for it: the request stream has nothing more to carry then, and
    /// the FETCH goes on until its fetch stream ends ([`Self::fetch`]).
    ///
    /// A cancelled subscription has no more use for its Objects, so its Track
    /// Alias is retired at once (§10.11 lets a subscriber that unsubscribed skip
    /// the wait).
    pub fn cancel(&mut self, request: RequestId) {
        if request.1 != self.tag {
            return;
        }
        self.ended_by_publish_done
            .retain(|ended| *ended != request.0);
        self.end_request(request);
    }

    /// What a Track Alias from a Subgroup Header refers to.
    #[must_use]
    pub fn subscribed_alias(&self, track_alias: u64) -> SubscribedAlias<'_> {
        self.subscribed_aliases.state(track_alias)
    }

    /// Whether a `SUBSCRIBE` of this endpoint waits for its `SUBSCRIBE_OK`,
    /// which could still give a Track Alias that is unknown now
    /// ([`crate::early`]).
    #[must_use]
    pub fn has_pending_subscribe(&self) -> bool {
        self.requests.values().any(|request| {
            matches!(
                request,
                Request::Subscribe {
                    track_alias: None,
                    ..
                }
            )
        })
    }

    /// Records that a data stream carrying `track_alias` ended, by FIN or by
    /// reset. `PUBLISH_DONE` counts the streams of a subscription, and once that
    /// many have ended the subscription has nothing left to deliver (§10.11).
    ///
    /// The call may come before the `SUBSCRIBE_OK` that assigns the alias is
    /// fed in, as happens when control and data streams are handled by
    /// different tasks: such streams are held and counted once the subscription
    /// is established. They are held for a bounded number of aliases. Beyond
    /// that the oldest are forgotten, and a subscription whose streams were
    /// forgotten stays draining until [`Self::on_drain_timeout`].
    pub fn on_data_stream_closed(&mut self, track_alias: u64) {
        self.subscribed_aliases.stream_closed(track_alias);
    }

    /// Records that the caller stopped waiting for the last Objects of a
    /// subscription that `PUBLISH_DONE` ended. §10.11 asks for a timer here; this
    /// crate has no clock, so the caller runs it. Does nothing unless the alias
    /// is draining.
    pub fn on_drain_timeout(&mut self, track_alias: u64) {
        self.subscribed_aliases.drain_timeout(track_alias);
    }

    fn on_subscribe_ok(
        &mut self,
        request: RequestId,
        ok: SubscribeOk,
    ) -> Result<RequestEvent, Error> {
        // §11.1: the same alias for a different track with an Established
        // subscription closes the session. Checked before the properties: an
        // error that ends the session outranks one that ends the request.
        //
        // An alias that is Established for the *same* track is refused as well.
        // §11.1 only names "a different Track", but a second subscription to one
        // track should never be accepted in the first place (§5.1,
        // DUPLICATE_SUBSCRIPTION), so a publisher that does it is at fault
        // either way.
        if matches!(
            self.subscribed_aliases.state(ok.track_alias),
            SubscribedAlias::Established(_)
        ) {
            return Err(Error::DuplicateTrackAlias(ok.track_alias));
        }
        if let Some(refusal) = track_properties_refusal(&ok.track_properties) {
            self.end_request(request);
            // The publisher may already be sending under the alias. Remembered
            // as retired, its streams are told apart from ones that are merely
            // ahead of their SUBSCRIBE_OK (§11.1).
            self.subscribed_aliases.retire_if_unknown(ok.track_alias);
            return Ok(RequestEvent::TrackNotProcessed {
                refusal,
                track_alias: ok.track_alias,
            });
        }
        if let Some(Request::Subscribe {
            full_track_name,
            track_alias,
            ..
        }) = self.requests.get_mut(&request.0)
        {
            *track_alias = Some(ok.track_alias);
            self.subscribed_aliases
                .establish(ok.track_alias, full_track_name.clone());
        }
        Ok(RequestEvent::SubscribeEstablished {
            track_alias: ok.track_alias,
            parameters: ok.parameters,
            track_properties: ok.track_properties,
        })
    }

    fn open(&mut self, request: Request) -> Result<RequestId, UsageError> {
        if self.go_aways.received {
            return Err(UsageError("the peer sent GOAWAY"));
        }
        let request_id = self.next_request_id;
        self.next_request_id = request_id
            .checked_add(2)
            .ok_or(UsageError("request ids are exhausted"))?;
        self.requests.insert(request_id, request);
        Ok(RequestId(request_id, self.tag))
    }

    /// Drops a request together with what it held, without waiting for anything.
    fn end_request(&mut self, request: RequestId) {
        match self.requests.remove(&request.0) {
            Some(Request::Subscribe {
                track_alias: Some(track_alias),
                ..
            }) => self.subscribed_aliases.retire(track_alias),
            // An alias this endpoint chose is free again at once: it decides
            // when it stops sending under it.
            Some(Request::Publish { track_alias, .. }) => {
                self.published_aliases.remove(&track_alias);
            }
            Some(
                Request::Subscribe { .. }
                | Request::PublishNamespace { .. }
                | Request::SubscribeNamespace { .. }
                | Request::Fetch(_)
                | Request::TrackStatus,
            )
            | None => {}
        }
    }

    fn check_own(&self, request: RequestId) -> Result<(), UsageError> {
        if request.1 == self.tag {
            Ok(())
        } else {
            Err(UsageError("the request belongs to another session"))
        }
    }

    /// Whether this endpoint is the subscriber of a subscription to the track,
    /// through its own SUBSCRIBE or a PUBLISH of the peer.
    fn is_subscribed_to(&self, full_track_name: &FullTrackName) -> bool {
        let own = self.requests.values().any(|request| {
            matches!(request, Request::Subscribe { full_track_name: name, .. } if name == full_track_name)
        });
        own || self.peer.publishes(full_track_name)
    }

    /// The SUBSCRIBE of this endpoint for the track that has no answer yet:
    /// the subscription in the Pending (Subscriber) state of §5.1.
    fn pending_subscribe_to(&self, full_track_name: &FullTrackName) -> Option<RequestId> {
        self.requests.iter().find_map(|(id, request)| {
            matches!(
                request,
                Request::Subscribe { full_track_name: name, track_alias: None, .. }
                    if name == full_track_name
            )
            .then_some(RequestId(*id, self.tag))
        })
    }

    /// Whether this endpoint is the publisher of a subscription to the track,
    /// through its own PUBLISH or a SUBSCRIBE of the peer.
    fn is_publishing(&self, full_track_name: &FullTrackName) -> bool {
        let own = self.requests.values().any(|request| {
            matches!(request, Request::Publish { full_track_name: name, .. } if name == full_track_name)
        });
        own || self.peer.subscribes_to(full_track_name)
    }

    fn was_issued(&self, request: RequestId) -> bool {
        let first = first_request_id(self.role);
        request.0 < self.next_request_id && request.0 % 2 == first
    }
}

/// §10.2.1, with what §10.2 lets the update of a subscription carry, for a
/// `REQUEST_UPDATE` on the stream of a PUBLISH of this endpoint: the decoder
/// let through what the update of any request may carry.
fn check_update_of_publish(live: &Request, message: &ControlMessage) -> Result<(), Error> {
    if let (Request::Publish { .. }, ControlMessage::RequestUpdate(update)) = (live, message)
        && !params::all_allowed(&update.parameters, MessageKind::RequestUpdate)
    {
        return Err(Error::ProtocolViolation(
            "parameter not allowed in this message",
        ));
    }
    Ok(())
}

/// The `REQUEST_OK` of a `SUBSCRIBE_NAMESPACE` of this endpoint. No parameter
/// of §10.2 lists it, and no Track Property belongs to one.
fn answered_namespaces(established: &mut bool, ok: &RequestOk) -> Result<RequestEvent, Error> {
    if !ok.parameters.is_empty() || !ok.track_properties.is_empty() {
        return Err(Error::ProtocolViolation(
            "a REQUEST_OK of SUBSCRIBE_NAMESPACE with parameters or properties",
        ));
    }
    *established = true;
    Ok(RequestEvent::NamespacesSubscribed)
}

/// A NAMESPACE or `NAMESPACE_DONE` on the stream of a `SUBSCRIBE_NAMESPACE`
/// of this endpoint whose prefix is `prefix` (§10.16, §10.17, §10.18).
fn on_namespace(
    prefix: &TrackNamespace,
    named: &mut HashSet<TrackNamespace>,
    message: ControlMessage,
) -> Result<RequestEvent, Error> {
    let (suffix, done) = match message {
        ControlMessage::Namespace(suffix) => (suffix, false),
        ControlMessage::NamespaceDone(suffix) => (suffix, true),
        _ => {
            return Err(Error::ProtocolViolation(
                "message not allowed in this request state",
            ));
        }
    };
    let mut fields = prefix.fields().to_vec();
    fields.extend(suffix.fields().iter().cloned());
    // A namespace longer than §2.4.1 allows breaks the rule for any
    // namespace.
    let whole = TrackNamespace::new(fields)
        .map_err(|_| Error::ProtocolViolation("a namespace of NAMESPACE that is too long"))?;
    if done {
        // §10.18: "If a subscriber receives a NAMESPACE_DONE before the
        // corresponding NAMESPACE, it MUST close the session with a
        // 'PROTOCOL_VIOLATION'."
        if !named.remove(&suffix) {
            return Err(Error::ProtocolViolation(
                "NAMESPACE_DONE before the corresponding NAMESPACE",
            ));
        }
        return Ok(RequestEvent::NamespaceDone(whole));
    }
    // WHY a NAMESPACE for a namespace named before is taken as it is: the
    // draft does not make it an error, and it changes nothing here.
    if !named.contains(&suffix) && named.len() >= MAX_NAMESPACES_OF_A_SUBSCRIPTION {
        return Ok(RequestEvent::TooManyNamespaces {
            limit: MAX_NAMESPACES_OF_A_SUBSCRIPTION,
        });
    }
    named.insert(suffix);
    Ok(RequestEvent::Namespace(whole))
}

fn first_request_id(role: Role) -> u64 {
    match role {
        Role::Client => 0,
        Role::Server => 1,
    }
}

enum MisplacedOption {
    Authority,
    Path,
}

/// AUTHORITY and PATH are only for a client on native QUIC (§10.3.1.1, §10.3.1.2).
fn misplaced_setup_option(
    setup_options: &[KeyValuePair],
    sender: Role,
    transport: Transport,
) -> Option<MisplacedOption> {
    if sender == Role::Client && transport == Transport::NativeQuic {
        return None;
    }
    setup_options.iter().find_map(|option| match option.ty() {
        setup_option::AUTHORITY => Some(MisplacedOption::Authority),
        setup_option::PATH => Some(MisplacedOption::Path),
        _ => None,
    })
}

fn check_own_parameters(parameters: &[Parameter], kind: MessageKind) -> Result<(), UsageError> {
    if params::all_allowed(parameters, kind) {
        Ok(())
    } else {
        Err(UsageError("parameter not allowed in this message"))
    }
}

/// What a `REQUEST_OK` may carry depends on the request it answers (§10.2.1),
/// and its Track Properties are empty unless it answers `TRACK_STATUS` (§10.5).
fn check_request_ok(
    parameters: &[Parameter],
    track_properties: &[KeyValuePair],
    kind: MessageKind,
) -> Result<(), Error> {
    if !params::all_allowed(parameters, kind) {
        return Err(Error::ProtocolViolation(
            "parameter not allowed in this message",
        ));
    }
    if !track_properties.is_empty() {
        return Err(Error::ProtocolViolation(
            "track properties in a REQUEST_OK that must not carry them",
        ));
    }
    Ok(())
}

/// What becomes of a `REQUEST_UPDATE` the subscriber of an established
/// PUBLISH of this endpoint sent: the one update that is applied, the
/// `SUBGROUP_DELIVERY_TIMEOUT`, is accepted, and any other refused.
fn answer_update_of_publish(state: &mut PublishState, update: &RequestUpdate) -> RequestEvent {
    if let Some(applied) = peer::only_the_subgroup_timeout(&update.parameters) {
        return RequestEvent::SubscriptionUpdated {
            response: ControlMessage::RequestOk(RequestOk::default()),
            subgroup_delivery_timeout: applied.subgroup_delivery_timeout,
            new_group_request: applied.new_group_request,
        };
    }
    *state = PublishState::UpdateRefused;
    RequestEvent::UpdateRefused {
        response: ControlMessage::RequestError(RequestError {
            error_code: request_error_code::NOT_SUPPORTED,
            // 0: retrying the same update will not help.
            retry_interval: 0,
            error_reason: ReasonPhrase::default(),
            redirect: None,
        }),
    }
}

/// The `REQUEST_UPDATE`s this endpoint sent for one subscription, as their
/// answers come ([`update_answered`]).
#[derive(Debug, Default)]
pub(crate) struct SentUpdates {
    /// Those that have no answer yet.
    unanswered: u32,
    /// Whether the publisher refused one.
    refused: bool,
}

impl SentUpdates {
    /// Whether one more may be sent: [`MAX_UNANSWERED_UPDATES`] wait at most.
    pub(crate) fn check_room(&self) -> Result<(), UsageError> {
        if self.unanswered >= MAX_UNANSWERED_UPDATES {
            return Err(UsageError("too many updates wait for their answer"));
        }
        Ok(())
    }

    /// One more was sent.
    pub(crate) fn sent(&mut self) {
        self.unanswered += 1;
    }

    /// Whether one waits for its answer.
    pub(crate) fn waiting(&self) -> bool {
        self.unanswered > 0
    }
}

/// The answer to a `REQUEST_UPDATE` of a subscription. §10.9: "The receiver
/// of a `REQUEST_UPDATE` MUST respond with exactly one `REQUEST_OK` or
/// `REQUEST_ERROR`". Responses carry no Request ID, so they answer the
/// updates in the order sent.
///
/// After a refusal the publisher ends the subscription (§10.9.1: it "MUST
/// also terminate the subscription by sending a `PUBLISH_DONE` with error
/// code `UPDATE_FAILED`"), and what answers the later updates is taken and
/// dropped. WHY each answer counts one, where a refusal could be read as
/// the answer to all that wait: a publisher that coalesces sends "only a
/// single `REQUEST_ERROR`" (§10.9.1), but one that does not answers each, and
/// both are followed by the `PUBLISH_DONE`, which ends the request whatever
/// still waits. `fixtures/update-answers/` has the rows.
fn update_answered(
    updates: &mut SentUpdates,
    answer: ControlMessage,
) -> Result<RequestEvent, Error> {
    let event = match answer {
        ControlMessage::RequestError(error) => RequestEvent::UpdateFailed(error),
        ControlMessage::RequestOk(ok) => {
            // What a REQUEST_UPDATE_OK may carry: §10.2.10, §10.2.11.
            let allowed = ok
                .parameters
                .iter()
                .all(|each| matches!(each, Parameter::Expires(_) | Parameter::LargestObject(_)));
            if !allowed {
                return Err(Error::ProtocolViolation(
                    "parameter not allowed in this message",
                ));
            }
            // §10.5: Track Properties "are empty in ... `REQUEST_UPDATE_OK`".
            if !ok.track_properties.is_empty() {
                return Err(Error::ProtocolViolation(
                    "track properties in a REQUEST_OK that must not carry them",
                ));
            }
            RequestEvent::UpdateAccepted {
                parameters: ok.parameters,
            }
        }
        _ => return Err(Error::ProtocolViolation("no answer to a REQUEST_UPDATE")),
    };
    updates.unanswered = updates.unanswered.saturating_sub(1);
    if updates.refused {
        return Ok(RequestEvent::UpdateAnsweredAfterRefusal);
    }
    updates.refused = matches!(event, RequestEvent::UpdateFailed(_));
    Ok(event)
}

/// The two MUSTs of §10.6.1 for a Redirect in `REQUEST_ERROR`.
fn check_redirect(role: Role, request: &Request, error: &RequestError) -> Result<(), Error> {
    let Some(redirect) = &error.redirect else {
        return Ok(());
    };
    if role == Role::Server && !redirect.connect_uri.is_empty() {
        return Err(Error::ProtocolViolation(
            "redirect with a connect URI sent to a server",
        ));
    }
    if matches!(request, Request::PublishNamespace { .. })
        && !redirect.full_track_name.name().is_empty()
    {
        return Err(Error::ProtocolViolation(
            "redirect with a track name for a namespace request",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Redirect;
    use crate::params::GroupOrder;
    use crate::test_util::{closed, update_answer_steps};

    fn track(name: &[u8]) -> FullTrackName {
        FullTrackName::new(namespace(), name.to_vec()).unwrap()
    }

    fn namespace() -> TrackNamespace {
        TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap()
    }

    impl Session {
        /// `on_request_message` for a handle of this session, where only the
        /// peer can be at fault.
        fn feed(
            &mut self,
            request: RequestId,
            message: ControlMessage,
        ) -> Result<RequestEvent, Error> {
            self.on_request_message(request, message)
                .map_err(|fault| match fault {
                    RequestFault::Peer(error) => error,
                    RequestFault::Usage(error) => panic!("{error}"),
                })
        }
    }

    fn client() -> Session {
        Session::new(Role::Client, Transport::WebTransport)
    }

    fn subscribe_ok(track_alias: u64, track_properties: Vec<KeyValuePair>) -> ControlMessage {
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias,
            parameters: vec![],
            track_properties,
        })
    }

    fn request_ok(parameters: Vec<Parameter>) -> ControlMessage {
        ControlMessage::RequestOk(RequestOk {
            parameters,
            track_properties: vec![],
        })
    }

    fn request_error() -> RequestError {
        RequestError {
            error_code: 0x10,
            retry_interval: 0,
            error_reason: ReasonPhrase::default(),
            redirect: None,
        }
    }

    fn redirect(connect_uri: &[u8], name: &[u8]) -> RequestError {
        RequestError {
            error_code: 0x34,
            retry_interval: 1,
            error_reason: ReasonPhrase::default(),
            redirect: Some(Redirect {
                connect_uri: connect_uri.to_vec(),
                full_track_name: track(name),
            }),
        }
    }

    fn publish_done() -> PublishDone {
        PublishDone {
            status_code: 2,
            stream_count: 1,
            error_reason: ReasonPhrase::default(),
        }
    }

    const NOT_ALLOWED: Error =
        Error::ProtocolViolation("message not allowed in this request state");
    const ENDED: Error = Error::ProtocolViolation("message after the request ended");

    /// One message fed to a request: what it yields and the state it leaves.
    type Step = (ControlMessage, Result<RequestEvent, Error>, RequestState);

    /// Replays each case on a fresh request opened by `open`.
    fn run_steps(open: impl Fn(&mut Session) -> RequestId, cases: Vec<(&str, Vec<Step>)>) {
        for (name, steps) in cases {
            let mut session = client();
            let request = open(&mut session);
            assert_eq!(
                session.request_state(request),
                Some(RequestState::Pending),
                "{name}"
            );
            for (index, (message, expected, state)) in steps.into_iter().enumerate() {
                assert_eq!(
                    session.feed(request, message),
                    expected,
                    "{name}, step {index}"
                );
                assert_eq!(
                    session.request_state(request),
                    Some(state),
                    "{name}, step {index}"
                );
            }
        }
    }

    #[test]
    fn request_ids_follow_the_role() {
        let cases = [(Role::Client, [0, 2, 4]), (Role::Server, [1, 3, 5])];
        for (role, expected) in cases {
            let mut session = Session::new(role, Transport::WebTransport);
            let ids = [
                session.subscribe(track(b"a"), vec![]).unwrap().0.value(),
                session
                    .publish(track(b"b"), 1, vec![], vec![])
                    .unwrap()
                    .0
                    .value(),
                session
                    .publish_namespace(namespace(), vec![])
                    .unwrap()
                    .0
                    .value(),
            ];
            assert_eq!(ids, expected, "{role:?}");
        }
    }

    #[test]
    fn requests_carry_the_track_they_were_opened_for() {
        let mut session = client();
        let (_, subscribe) = session.subscribe(track(b"a"), vec![]).unwrap();
        assert_eq!(
            subscribe,
            ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                full_track_name: track(b"a"),
                parameters: vec![],
            })
        );
        let (_, publish) = session.publish(track(b"b"), 9, vec![], vec![]).unwrap();
        assert_eq!(
            publish,
            ControlMessage::Publish(Publish {
                request_id: 2,
                full_track_name: track(b"b"),
                track_alias: 9,
                parameters: vec![],
                track_properties: vec![],
            })
        );
    }

    #[test]
    fn setup_is_exchanged_once_in_each_direction() {
        let mut session = client();
        assert!(!session.is_established());
        assert_eq!(
            session.setup(vec![]),
            Ok(ControlMessage::Setup(Setup {
                setup_options: vec![]
            }))
        );
        assert_eq!(
            session.setup(vec![]),
            Err(UsageError("SETUP was already sent"))
        );
        assert!(!session.is_established());

        let peer_setup = Setup {
            setup_options: vec![KeyValuePair::bytes(0x07, b"relay".to_vec()).unwrap()],
        };
        assert_eq!(
            session.on_control_message(ControlMessage::Setup(peer_setup.clone())),
            Ok(ControlEvent::Setup(peer_setup.clone()))
        );
        assert!(session.is_established());
        assert_eq!(
            session.on_control_message(ControlMessage::Setup(peer_setup)),
            Err(Error::ProtocolViolation(
                "message not allowed on the control stream"
            ))
        );
    }

    #[test]
    fn the_control_stream_begins_with_setup() {
        assert_eq!(
            client().on_control_message(request_ok(vec![])),
            Err(Error::ProtocolViolation(
                "control stream did not begin with SETUP"
            ))
        );
    }

    fn authority() -> Vec<KeyValuePair> {
        vec![KeyValuePair::bytes(0x05, b"example.com".to_vec()).unwrap()]
    }

    fn path() -> Vec<KeyValuePair> {
        vec![KeyValuePair::bytes(0x01, b"/moq".to_vec()).unwrap()]
    }

    #[test]
    fn a_received_authority_or_path_must_come_from_a_native_quic_client() {
        // (our role, transport, option the peer sends, expected)
        let cases = [
            (
                Role::Client,
                Transport::NativeQuic,
                authority(),
                Err(Error::InvalidAuthority),
            ),
            (
                Role::Client,
                Transport::NativeQuic,
                path(),
                Err(Error::InvalidPath),
            ),
            (
                Role::Server,
                Transport::WebTransport,
                authority(),
                Err(Error::InvalidAuthority),
            ),
            (
                Role::Server,
                Transport::WebTransport,
                path(),
                Err(Error::InvalidPath),
            ),
            (Role::Server, Transport::NativeQuic, authority(), Ok(())),
            (Role::Server, Transport::NativeQuic, path(), Ok(())),
        ];
        for (role, transport, setup_options, expected) in cases {
            let message = ControlMessage::Setup(Setup {
                setup_options: setup_options.clone(),
            });
            let result = Session::new(role, transport)
                .on_control_message(message)
                .map(|_| ());
            assert_eq!(
                result, expected,
                "{role:?} over {transport:?}: {setup_options:?}"
            );
        }
    }

    #[test]
    fn only_a_native_quic_client_sends_authority_or_path() {
        // (our role, transport, option we send, expected)
        let cases = [
            (
                Role::Client,
                Transport::WebTransport,
                authority(),
                Err(UsageError("AUTHORITY is for a client on native QUIC")),
            ),
            (
                Role::Server,
                Transport::NativeQuic,
                path(),
                Err(UsageError("PATH is for a client on native QUIC")),
            ),
            (Role::Client, Transport::NativeQuic, authority(), Ok(())),
            (Role::Client, Transport::NativeQuic, path(), Ok(())),
        ];
        for (role, transport, setup_options, expected) in cases {
            let result = Session::new(role, transport)
                .setup(setup_options.clone())
                .map(|_| ());
            assert_eq!(
                result, expected,
                "{role:?} over {transport:?}: {setup_options:?}"
            );
        }
    }

    #[test]
    fn a_single_instance_setup_option_is_not_sent_twice() {
        let implementation = || KeyValuePair::bytes(0x07, b"xmoq".to_vec()).unwrap();
        let token = || KeyValuePair::bytes(0x03, vec![0x03, 0x00]).unwrap();
        let cases = [
            (
                vec![implementation(), implementation()],
                Err(UsageError("setup option repeated")),
            ),
            // AUTHORIZATION TOKEN may repeat (§10.3.1.4).
            (vec![token(), token(), implementation()], Ok(())),
        ];
        for (setup_options, expected) in cases {
            assert_eq!(
                client().setup(setup_options.clone()).map(|_| ()),
                expected,
                "{setup_options:?}"
            );
        }
    }

    /// A session of `role` that has exchanged SETUP.
    fn set_up(role: Role) -> Session {
        let mut session = Session::new(role, Transport::WebTransport);
        session.setup(vec![]).unwrap();
        session
            .on_control_message(ControlMessage::Setup(Setup {
                setup_options: vec![],
            }))
            .unwrap();
        session
    }

    fn go_away(new_session_uri: &[u8], request_id: Option<u64>) -> ControlMessage {
        ControlMessage::GoAway(GoAway {
            new_session_uri: new_session_uri.to_vec(),
            timeout_ms: 5000,
            request_id,
        })
    }

    /// §10.16 to §10.18: what the stream of a `SUBSCRIBE_NAMESPACE` of this
    /// endpoint may carry, and what the session makes of it. The prefix is
    /// (`xmoq`); a NAMESPACE names (`a`) under it.
    #[test]
    fn a_subscribe_namespace_is_answered_and_told_of_namespaces() {
        let ns = |fields: &[&[u8]]| {
            TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
        };
        let named = || ControlMessage::Namespace(ns(&[b"a"]));
        let done = || ControlMessage::NamespaceDone(ns(&[b"a"]));
        // (what comes, what the last of it leads to)
        let cases: [(Vec<ControlMessage>, Result<RequestEvent, Error>); 5] = [
            (
                vec![request_ok(vec![])],
                Ok(RequestEvent::NamespacesSubscribed),
            ),
            (
                vec![request_ok(vec![]), named()],
                Ok(RequestEvent::Namespace(ns(&[b"xmoq", b"a"]))),
            ),
            (
                vec![request_ok(vec![]), named(), done()],
                Ok(RequestEvent::NamespaceDone(ns(&[b"xmoq", b"a"]))),
            ),
            (
                vec![request_ok(vec![]), done()],
                Err(Error::ProtocolViolation(
                    "NAMESPACE_DONE before the corresponding NAMESPACE",
                )),
            ),
            (
                vec![named()],
                Err(Error::ProtocolViolation(
                    "message not allowed in this request state",
                )),
            ),
        ];
        for (messages, expected) in cases {
            let mut session = set_up(Role::Client);
            let (request, _) = session.subscribe_namespace(ns(&[b"xmoq"]), vec![]).unwrap();
            let mut last = Ok(RequestEvent::LateMessage);
            for message in messages {
                last = session.feed(request, message);
            }
            assert_eq!(last, expected);
        }
    }

    /// One NAMESPACE more than [`MAX_NAMESPACES_OF_A_SUBSCRIPTION`] ends the
    /// request, and the session goes on: what comes after on the stream is a
    /// message after the request ended.
    #[test]
    fn a_namespace_past_what_a_subscribe_namespace_keeps_ends_the_request() {
        let ns = |field: Vec<u8>| TrackNamespace::new(vec![field]).unwrap();
        let mut session = set_up(Role::Client);
        let (request, _) = session
            .subscribe_namespace(ns(b"xmoq".to_vec()), vec![])
            .unwrap();
        session.feed(request, request_ok(vec![])).unwrap();
        for index in 0..MAX_NAMESPACES_OF_A_SUBSCRIPTION {
            let named = ControlMessage::Namespace(ns(index.to_string().into_bytes()));
            assert!(matches!(
                session.feed(request, named),
                Ok(RequestEvent::Namespace(_))
            ));
        }
        let one_more = || ControlMessage::Namespace(ns(b"one more".to_vec()));
        assert_eq!(
            session.feed(request, one_more()),
            Ok(RequestEvent::TooManyNamespaces { limit: 4096 })
        );
        assert_eq!(
            session.feed(request, one_more()),
            Err(Error::ProtocolViolation("message after the request ended"))
        );
        assert!(
            session
                .subscribe_namespace(ns(b"other".to_vec()), vec![])
                .is_ok()
        );
    }

    /// §10.4: the GOAWAY this endpoint sends.
    #[test]
    fn a_goaway_is_sent_by_the_rules_of_its_fields() {
        let uri: &[u8] = b"https://relay.example/";
        let empty: &[u8] = b"";
        // (this endpoint's role, whether SETUP was sent, New Session URI,
        //  what the session says)
        let cases = [
            (Role::Server, true, uri, Ok(go_away(uri, Some(0)))),
            (Role::Server, true, empty, Ok(go_away(empty, Some(0)))),
            (Role::Client, true, empty, Ok(go_away(empty, Some(1)))),
            (
                Role::Client,
                true,
                uri,
                Err(UsageError("a GOAWAY of a client with a URI")),
            ),
            (
                Role::Server,
                false,
                empty,
                Err(UsageError("GOAWAY before SETUP")),
            ),
        ];
        for (role, setup_sent, new_session_uri, expected) in cases {
            let mut session = Session::new(role, Transport::WebTransport);
            if setup_sent {
                session.setup(vec![]).unwrap();
            }
            assert_eq!(
                session.go_away(new_session_uri.to_vec(), 5000),
                expected,
                "{role:?} {setup_sent} {new_session_uri:?}"
            );
            assert_eq!(session.going_away(), expected.is_ok(), "{role:?}");
        }
        // "more than one GOAWAY on the control stream" is a violation at
        // the receiver, so a second is not made.
        let mut session = set_up(Role::Server);
        session.go_away(vec![], 5000).unwrap();
        assert_eq!(
            session.go_away(vec![], 5000),
            Err(UsageError("a second GOAWAY"))
        );
    }

    /// §10.4: the Request ID of a GOAWAY, and the requests that come after
    /// it.
    #[test]
    fn requests_of_the_peer_after_a_goaway_are_refused_with_going_away() {
        let subscribe = |request_id, name: &[u8]| {
            ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name: track(name),
                parameters: vec![],
            })
        };
        let going_away = || {
            ControlMessage::RequestError(RequestError {
                error_code: 0x6,
                retry_interval: 0,
                error_reason: ReasonPhrase::default(),
                redirect: None,
            })
        };
        // (the Request IDs of the client's SUBSCRIBEs before the GOAWAY; the
        //  Request ID the GOAWAY names)
        let cases: [(&[u64], u64); 4] = [
            (&[], 0),
            (&[0], 2),
            (&[0, 2], 4),
            // 4 came, 2 has not: 2 might not have been processed.
            (&[0, 4], 2),
        ];
        for (before, named) in cases {
            let mut session = set_up(Role::Server);
            for request_id in before {
                let name = request_id.to_string();
                let opened =
                    session.on_peer_request_opened(subscribe(*request_id, name.as_bytes()));
                assert!(
                    matches!(opened, Ok(PeerRequestOpened::Request(..))),
                    "{before:?}"
                );
            }
            assert_eq!(
                session.go_away(vec![], 5000),
                Ok(go_away(b"", Some(named))),
                "{before:?}"
            );
            assert!(session.going_away(), "{before:?}");
            // Below the ID named and above it alike, and a request of any
            // kind.
            let subscribe_namespace = |request_id| {
                ControlMessage::SubscribeNamespace(SubscribeNamespace {
                    request_id,
                    track_namespace_prefix: TrackNamespace::new(vec![b"b".to_vec()]).unwrap(),
                    parameters: vec![],
                })
            };
            for (request_id, request) in [
                (named, subscribe(named, b"b")),
                (8, subscribe(8, b"b")),
                (10, subscribe_namespace(10)),
            ] {
                let opened = session.on_peer_request_opened(request);
                let Ok(PeerRequestOpened::Refused { response, .. }) = opened else {
                    panic!("{before:?} {request_id}: {opened:?}");
                };
                assert_eq!(response, going_away(), "{before:?} {request_id}");
            }
        }
    }

    /// §10.4: what a GOAWAY on the control stream may hold.
    #[test]
    fn a_goaway_is_taken_by_the_rules_of_its_fields() {
        let uri: &[u8] = b"https://relay.example/";
        let empty: &[u8] = b"";
        // (the receiver's role, New Session URI, Request ID, what the session says)
        let cases = [
            (Role::Client, uri, Some(0), Ok(())),
            (Role::Client, empty, Some(4), Ok(())),
            (Role::Server, empty, Some(1), Ok(())),
            (
                Role::Server,
                uri,
                Some(1),
                Err(Error::ProtocolViolation("a GOAWAY of a client with a URI")),
            ),
            (
                Role::Client,
                uri,
                None,
                Err(Error::ProtocolViolation("a GOAWAY without a Request ID")),
            ),
            (Role::Client, uri, Some(1), Err(Error::InvalidRequestId(1))),
            (
                Role::Server,
                empty,
                Some(2),
                Err(Error::InvalidRequestId(2)),
            ),
        ];
        for (role, uri, request_id, expected) in cases {
            let mut session = set_up(role);
            let taken = session
                .on_control_message(go_away(uri, request_id))
                .map(|event| assert!(matches!(event, ControlEvent::GoAway(_))));
            assert_eq!(taken, expected, "{role:?} {request_id:?}");
            assert_eq!(
                session.peer_going_away(),
                expected.is_ok(),
                "{role:?} {request_id:?}"
            );
        }
    }

    /// §10.4: "Upon receiving a GOAWAY on the control stream, an endpoint
    /// SHOULD NOT initiate new requests to the peer", and "MUST close the
    /// session with a `PROTOCOL_VIOLATION` ... if it receives more than one
    /// GOAWAY on the control stream".
    #[test]
    fn after_a_goaway_no_request_is_opened_and_a_second_goaway_is_a_violation() {
        let mut session = set_up(Role::Client);
        let (open, _) = session.subscribe(track(b"before"), vec![]).unwrap();
        session.on_control_message(go_away(b"", Some(2))).unwrap();
        assert_eq!(
            session.subscribe(track(b"after"), vec![]).map(|_| ()),
            Err(UsageError("the peer sent GOAWAY"))
        );
        assert_eq!(
            session.publish_namespace(namespace(), vec![]).map(|_| ()),
            Err(UsageError("the peer sent GOAWAY"))
        );
        // What was open stays so ("The GOAWAY message does not impact
        // subscription state").
        assert_eq!(session.request_state(open), Some(RequestState::Pending));
        assert_eq!(
            session.feed(open, subscribe_ok(0, vec![])).map(|_| ()),
            Ok(())
        );
        assert_eq!(
            session.on_control_message(go_away(b"", Some(2))),
            Err(Error::ProtocolViolation("a second GOAWAY"))
        );
    }

    /// §10.4, a GOAWAY on the stream of a request of this endpoint: told to
    /// the caller, the request as it was, by the rules of its fields.
    #[test]
    fn a_goaway_on_a_request_stream_is_told_and_moves_nothing() {
        let uri: &[u8] = b"https://relay.example/";
        // (the GOAWAYs fed in turn, what the last one leads to)
        let cases = [
            (
                vec![go_away(uri, None)],
                Ok(Some(b"https://relay.example/".to_vec())),
            ),
            (
                vec![go_away(uri, Some(0))],
                Err(Error::ProtocolViolation(
                    "a GOAWAY on a request stream with a Request ID",
                )),
            ),
            (
                vec![go_away(uri, None), go_away(uri, None)],
                Err(Error::ProtocolViolation(
                    "a second GOAWAY on a request stream",
                )),
            ),
        ];
        for (messages, expected) in cases {
            let mut session = set_up(Role::Client);
            let (open, _) = session.subscribe(track(b"a"), vec![]).unwrap();
            let mut last = Ok(None);
            for message in messages {
                last = session.feed(open, message).map(|event| match event {
                    RequestEvent::GoAway(go_away) => Some(go_away.new_session_uri),
                    _ => None,
                });
            }
            assert_eq!(last, expected);
            // "The GOAWAY message does not impact subscription state", nor
            // opens the session's way to new requests.
            assert_eq!(session.request_state(open), Some(RequestState::Pending));
            assert!(!session.peer_going_away());
        }
    }

    /// The same on the stream of a request of the peer, at a server, which may
    /// be sent no New Session URI (§10.4).
    #[test]
    fn a_goaway_on_a_request_stream_of_the_peer_follows_the_same_rules() {
        // (New Session URI, what the session says)
        let cases: [(&[u8], Result<(), Error>); 2] = [
            (b"", Ok(())),
            (
                b"https://relay.example/",
                Err(Error::ProtocolViolation("a GOAWAY of a client with a URI")),
            ),
        ];
        for (uri, expected) in cases {
            let mut session = set_up(Role::Server);
            let opened = session
                .on_peer_request_opened(ControlMessage::Subscribe(Subscribe {
                    request_id: 0,
                    full_track_name: track(b"a"),
                    parameters: vec![],
                }))
                .unwrap();
            let PeerRequestOpened::Request(id, _) = opened else {
                panic!("{opened:?}");
            };
            let fed = session
                .on_peer_request_message(id, go_away(uri, None))
                .map(|event| assert!(matches!(event, PeerRequestEvent::GoAway(_))));
            assert_eq!(fed, expected.map_err(RequestFault::Peer), "{uri:?}");
        }
    }

    #[test]
    fn a_received_setup_may_repeat_an_option() {
        let implementation = || KeyValuePair::bytes(0x07, b"relay".to_vec()).unwrap();
        let setup = Setup {
            setup_options: vec![implementation(), implementation()],
        };
        assert_eq!(
            client().on_control_message(ControlMessage::Setup(setup.clone())),
            Ok(ControlEvent::Setup(setup))
        );
    }

    #[test]
    fn subscribe_transitions() {
        let established = RequestEvent::SubscribeEstablished {
            track_alias: 7,
            parameters: vec![],
            track_properties: vec![],
        };
        let cases: Vec<(&str, Vec<Step>)> = vec![
            (
                "accepted, then done",
                vec![
                    (
                        subscribe_ok(7, vec![]),
                        Ok(established.clone()),
                        RequestState::Established,
                    ),
                    (
                        ControlMessage::PublishDone(publish_done()),
                        Ok(RequestEvent::SubscriptionDone(publish_done())),
                        RequestState::Terminated,
                    ),
                    (
                        ControlMessage::PublishDone(publish_done()),
                        Err(ENDED),
                        RequestState::Terminated,
                    ),
                ],
            ),
            (
                "rejected",
                vec![
                    (
                        ControlMessage::RequestError(request_error()),
                        Ok(RequestEvent::Rejected(request_error())),
                        RequestState::Terminated,
                    ),
                    (
                        subscribe_ok(7, vec![]),
                        Err(ENDED),
                        RequestState::Terminated,
                    ),
                ],
            ),
            (
                "redirected to another relay and track",
                vec![(
                    ControlMessage::RequestError(redirect(b"moqt://b", b"b")),
                    Ok(RequestEvent::Rejected(redirect(b"moqt://b", b"b"))),
                    RequestState::Terminated,
                )],
            ),
            (
                "a second response",
                vec![
                    (
                        subscribe_ok(7, vec![]),
                        Ok(established.clone()),
                        RequestState::Established,
                    ),
                    (
                        subscribe_ok(7, vec![]),
                        Err(NOT_ALLOWED),
                        RequestState::Established,
                    ),
                    (
                        ControlMessage::RequestError(request_error()),
                        Err(NOT_ALLOWED),
                        RequestState::Established,
                    ),
                ],
            ),
            (
                "PUBLISH_DONE before a response",
                vec![(
                    ControlMessage::PublishDone(publish_done()),
                    Err(NOT_ALLOWED),
                    RequestState::Pending,
                )],
            ),
            (
                "the response to another kind of request",
                vec![(request_ok(vec![]), Err(NOT_ALLOWED), RequestState::Pending)],
            ),
            (
                "a mandatory track property",
                vec![(
                    subscribe_ok(7, vec![KeyValuePair::varint(0x4000, 1).unwrap()]),
                    Ok(RequestEvent::TrackNotProcessed {
                        refusal: TrackRefusal::Unsupported(0x4000),
                        track_alias: 7,
                    }),
                    RequestState::Terminated,
                )],
            ),
        ];
        run_steps(
            |session| session.subscribe(track(b"a"), vec![]).unwrap().0,
            cases,
        );
    }

    #[test]
    fn publish_transitions() {
        let cases: Vec<(&str, Vec<Step>)> = vec![
            (
                "accepted with the defaults",
                vec![
                    (
                        request_ok(vec![]),
                        Ok(RequestEvent::PublishEstablished {
                            forward: true,
                            parameters: vec![],
                        }),
                        RequestState::Established,
                    ),
                    (
                        request_ok(vec![]),
                        Err(NOT_ALLOWED),
                        RequestState::Established,
                    ),
                    (
                        ControlMessage::RequestError(request_error()),
                        Err(NOT_ALLOWED),
                        RequestState::Established,
                    ),
                ],
            ),
            (
                "accepted without forwarding",
                vec![(
                    request_ok(vec![
                        Parameter::Forward(false),
                        Parameter::GroupOrder(GroupOrder::Ascending),
                    ]),
                    Ok(RequestEvent::PublishEstablished {
                        forward: false,
                        parameters: vec![
                            Parameter::Forward(false),
                            Parameter::GroupOrder(GroupOrder::Ascending),
                        ],
                    }),
                    RequestState::Established,
                )],
            ),
            (
                "rejected",
                vec![(
                    ControlMessage::RequestError(request_error()),
                    Ok(RequestEvent::Rejected(request_error())),
                    RequestState::Terminated,
                )],
            ),
            (
                "PUBLISH_OK with a SUBSCRIBE_OK parameter",
                vec![(
                    request_ok(vec![Parameter::LargestObject(crate::name::Location {
                        group: 0,
                        object: 0,
                    })]),
                    Err(Error::ProtocolViolation(
                        "parameter not allowed in this message",
                    )),
                    RequestState::Pending,
                )],
            ),
            (
                "PUBLISH_OK with track properties",
                vec![(
                    ControlMessage::RequestOk(RequestOk {
                        parameters: vec![],
                        track_properties: vec![KeyValuePair::varint(2, 1).unwrap()],
                    }),
                    Err(Error::ProtocolViolation(
                        "track properties in a REQUEST_OK that must not carry them",
                    )),
                    RequestState::Pending,
                )],
            ),
            (
                "SUBSCRIBE_OK",
                vec![(
                    subscribe_ok(7, vec![]),
                    Err(NOT_ALLOWED),
                    RequestState::Pending,
                )],
            ),
        ];
        run_steps(
            |session| session.publish(track(b"a"), 1, vec![], vec![]).unwrap().0,
            cases,
        );
    }

    #[test]
    fn publish_namespace_transitions() {
        let cases: Vec<(&str, Vec<Step>)> = vec![
            (
                "accepted",
                vec![
                    (
                        request_ok(vec![]),
                        Ok(RequestEvent::PublishNamespaceEstablished),
                        RequestState::Established,
                    ),
                    (
                        request_ok(vec![]),
                        Err(NOT_ALLOWED),
                        RequestState::Established,
                    ),
                ],
            ),
            (
                "rejected",
                vec![(
                    ControlMessage::RequestError(request_error()),
                    Ok(RequestEvent::Rejected(request_error())),
                    RequestState::Terminated,
                )],
            ),
            (
                "redirected to another relay",
                vec![(
                    ControlMessage::RequestError(redirect(b"moqt://b", b"")),
                    Ok(RequestEvent::Rejected(redirect(b"moqt://b", b""))),
                    RequestState::Terminated,
                )],
            ),
            (
                "redirected with a track name",
                vec![(
                    ControlMessage::RequestError(redirect(b"", b"b")),
                    Err(Error::ProtocolViolation(
                        "redirect with a track name for a namespace request",
                    )),
                    RequestState::Pending,
                )],
            ),
            (
                "PUBLISH_NAMESPACE_OK with a parameter",
                vec![(
                    request_ok(vec![Parameter::Forward(true)]),
                    Err(Error::ProtocolViolation(
                        "parameter not allowed in this message",
                    )),
                    RequestState::Pending,
                )],
            ),
        ];
        run_steps(
            |session| session.publish_namespace(namespace(), vec![]).unwrap().0,
            cases,
        );
    }

    #[test]
    fn track_status_transitions() {
        let largest = Parameter::LargestObject(Location {
            group: 4,
            object: 1,
        });
        let property = KeyValuePair::varint(2, 5).unwrap();
        let cases: Vec<(&str, Vec<Step>)> = vec![
            (
                "answered with the largest Location and a Track Property",
                vec![
                    (
                        ControlMessage::RequestOk(RequestOk {
                            parameters: vec![largest.clone()],
                            track_properties: vec![property.clone()],
                        }),
                        Ok(RequestEvent::TrackStatusAnswered {
                            parameters: vec![largest],
                            track_properties: vec![property],
                        }),
                        RequestState::Terminated,
                    ),
                    (request_ok(vec![]), Err(ENDED), RequestState::Terminated),
                ],
            ),
            (
                "rejected",
                vec![(
                    ControlMessage::RequestError(request_error()),
                    Ok(RequestEvent::Rejected(request_error())),
                    RequestState::Terminated,
                )],
            ),
            // §10.2.11: LARGEST_OBJECT alone may appear in it.
            (
                "TRACK_STATUS_OK with another parameter",
                vec![(
                    request_ok(vec![Parameter::Expires(1)]),
                    Err(Error::ProtocolViolation(
                        "parameter not allowed in this message",
                    )),
                    RequestState::Pending,
                )],
            ),
            (
                "SUBSCRIBE_OK",
                vec![(
                    subscribe_ok(1, vec![]),
                    Err(NOT_ALLOWED),
                    RequestState::Pending,
                )],
            ),
        ];
        run_steps(
            |session| session.track_status(track(b"a"), vec![]).unwrap().0,
            cases,
        );
        // §10.2.2: AUTHORIZATION_TOKEN is the one parameter it carries.
        assert_eq!(
            client()
                .track_status(track(b"a"), vec![Parameter::RendezvousTimeout(1)])
                .map(|_| ()),
            Err(UsageError("parameter not allowed in this message"))
        );
    }

    #[test]
    fn a_server_is_not_redirected_to_another_uri() {
        // (connect URI in the Redirect, expected)
        let cases: [(&[u8], Result<RequestEvent, Error>); 2] = [
            (
                b"moqt://b",
                Err(Error::ProtocolViolation(
                    "redirect with a connect URI sent to a server",
                )),
            ),
            (b"", Ok(RequestEvent::Rejected(redirect(b"", b"b")))),
        ];
        for (connect_uri, expected) in cases {
            let mut session = Session::new(Role::Server, Transport::WebTransport);
            let (request, _) = session.subscribe(track(b"a"), vec![]).unwrap();
            assert_eq!(
                session.feed(
                    request,
                    ControlMessage::RequestError(redirect(connect_uri, b"b"))
                ),
                expected,
                "{connect_uri:?}"
            );
        }
    }

    /// §11.4.2: Objects read before the `SUBSCRIBE_OK` that gives their
    /// alias are given to the subscription after it, in their order
    /// (`crate::early`), and not when no `SUBSCRIBE` waits.
    #[test]
    fn objects_read_before_their_subscribe_ok_are_given_after_it() {
        use crate::early::{Arrived, EarlyObjects};
        let mut session = client();
        let mut early = EarlyObjects::default();
        // No SUBSCRIBE waits: the stream is stopped.
        assert!(matches!(
            early.arrive(&session, 1, 7, 64, "stray"),
            Arrived::Stop {
                stream: 1,
                code: crate::stream_reset_code::CANCELLED
            }
        ));
        let (request, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        for line in ["line 0", "line 1"] {
            assert!(matches!(
                early.arrive(&session, 2, 7, 64, line),
                Arrived::Held
            ));
        }
        // Before the SUBSCRIBE_OK nothing is let go of.
        assert_eq!(early.settle(&session).released, Vec::<&str>::new());
        session.feed(request, subscribe_ok(7, vec![])).unwrap();
        let settled = early.settle(&session);
        assert_eq!(settled.released, ["line 0", "line 1"]);
        assert_eq!(settled.stopped, Vec::new());
        assert!(early.is_empty());
        assert!(matches!(
            early.arrive(&session, 2, 7, 64, "line 2"),
            Arrived::Deliver("line 2")
        ));
    }

    /// A stream reset while it held Objects lets go of them; held ones of
    /// an alias no SUBSCRIBE can give any more are stopped.
    #[test]
    fn held_objects_are_let_go_of_with_their_stream_or_their_subscribe() {
        use crate::early::{Arrived, EarlyObjects};
        let mut session = client();
        let mut early = EarlyObjects::default();
        let (request, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        assert!(matches!(
            early.arrive(&session, 1, 7, 64, "a"),
            Arrived::Held
        ));
        assert!(matches!(
            early.arrive(&session, 2, 8, 64, "b"),
            Arrived::Held
        ));
        early.stream_closed(1, true);
        session.feed(request, subscribe_ok(9, vec![])).unwrap();
        let settled = early.settle(&session);
        assert_eq!(settled.released, Vec::<&str>::new());
        assert_eq!(settled.stopped, [(2, crate::stream_reset_code::CANCELLED)]);
        assert!(early.is_empty());
    }

    #[test]
    fn a_track_alias_names_one_subscribed_track() {
        let mut session = client();
        let (first, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        let (second, _) = session.subscribe(track(b"b"), vec![]).unwrap();
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Unknown);

        session.feed(first, subscribe_ok(7, vec![])).unwrap();
        assert_eq!(
            session.subscribed_alias(7),
            SubscribedAlias::Established(&track(b"a"))
        );
        assert_eq!(
            session.feed(second, subscribe_ok(7, vec![])),
            Err(Error::DuplicateTrackAlias(7))
        );
        // The duplicate alias closes the session even when the same SUBSCRIBE_OK
        // also names a Mandatory Track Property, which alone would only end the
        // request.
        assert_eq!(
            session.feed(
                second,
                subscribe_ok(7, vec![KeyValuePair::varint(0x4000, 1).unwrap()])
            ),
            Err(Error::DuplicateTrackAlias(7))
        );
        assert_eq!(session.request_state(second), Some(RequestState::Pending));

        // Once the first subscription is over, the publisher may give its alias
        // to another track.
        session.cancel(first);
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
        assert!(session.feed(second, subscribe_ok(7, vec![])).is_ok());
        assert_eq!(
            session.subscribed_alias(7),
            SubscribedAlias::Established(&track(b"b"))
        );
    }

    #[test]
    fn the_alias_of_an_unsupported_track_is_remembered_as_retired() {
        let mut session = client();
        let (request, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        session
            .feed(
                request,
                subscribe_ok(7, vec![KeyValuePair::varint(0x4000, 1).unwrap()]),
            )
            .unwrap();
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn streams_closed_before_subscribe_ok_are_counted() {
        let mut session = client();
        let (request, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        // The data stream is handled before the SUBSCRIBE_OK that explains it.
        session.on_data_stream_closed(7);
        session.feed(request, subscribe_ok(7, vec![])).unwrap();
        // Stream Count 1: the one stream already ended.
        session
            .feed(request, ControlMessage::PublishDone(publish_done()))
            .unwrap();
        assert_eq!(session.subscribed_alias(7), SubscribedAlias::Retired);
    }

    #[test]
    fn publish_done_leaves_the_alias_draining() {
        // (what the caller reports after PUBLISH_DONE with Stream Count 1)
        type Report = fn(&mut Session);
        let reports: [(&str, Report); 2] = [
            ("the counted stream ended", |session| {
                session.on_data_stream_closed(7);
            }),
            ("the caller gave up waiting", |session| {
                session.on_drain_timeout(7);
            }),
        ];
        for (name, report) in reports {
            let mut session = client();
            let (request, _) = session.subscribe(track(b"a"), vec![]).unwrap();
            session.feed(request, subscribe_ok(7, vec![])).unwrap();
            session
                .feed(request, ControlMessage::PublishDone(publish_done()))
                .unwrap();
            assert_eq!(
                session.request_state(request),
                Some(RequestState::Terminated),
                "{name}"
            );
            assert_eq!(
                session.subscribed_alias(7),
                SubscribedAlias::Draining(&track(b"a")),
                "{name}"
            );
            report(&mut session);
            assert_eq!(
                session.subscribed_alias(7),
                SubscribedAlias::Retired,
                "{name}"
            );
        }
    }

    #[test]
    fn a_track_alias_names_one_published_track() {
        let mut session = client();
        let (first, _) = session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
        assert_eq!(
            session.publish(track(b"b"), 1, vec![], vec![]).map(|_| ()),
            Err(UsageError("track alias already in use"))
        );

        let done = session.publish_done(first, 2, 1, ReasonPhrase::default());
        assert_eq!(done, Ok(ControlMessage::PublishDone(publish_done())));
        assert_eq!(session.request_state(first), Some(RequestState::Terminated));
        assert!(session.publish(track(b"b"), 1, vec![], vec![]).is_ok());
    }

    #[test]
    fn cancelling_frees_what_the_request_held() {
        let mut session = client();
        let (publish, _) = session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
        session.cancel(publish);
        // Cancelling twice is harmless.
        session.cancel(publish);
        assert_eq!(
            session.request_state(publish),
            Some(RequestState::Terminated)
        );
        assert!(session.publish(track(b"b"), 1, vec![], vec![]).is_ok());
    }

    #[test]
    fn messages_that_cross_a_publish_done_are_ignored() {
        // Every update has a Request ID of its own (§10.1).
        let update = |request_id| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters: vec![],
            })
        };
        // (whether the PUBLISH was accepted before PUBLISH_DONE, what arrives after)
        let cases = [
            (false, vec![request_ok(vec![]), update(1), update(3)]),
            (false, vec![ControlMessage::RequestError(request_error())]),
            (true, vec![update(1), request_ok(vec![])]),
        ];
        for (accepted, late) in cases {
            let mut session = client();
            let (request, _) = session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
            if accepted {
                session.feed(request, request_ok(vec![])).unwrap();
            }
            session
                .publish_done(request, 2, 0, ReasonPhrase::default())
                .unwrap();
            for message in late {
                assert_eq!(
                    session.feed(request, message.clone()),
                    Ok(RequestEvent::LateMessage),
                    "accepted {accepted}: {message:?}"
                );
            }
            // Once the stream is reported gone, the request is forgotten.
            session.cancel(request);
            assert_eq!(
                session.feed(request, update(5)),
                Err(ENDED),
                "accepted {accepted}"
            );
        }
    }

    #[test]
    fn a_request_update_uses_up_a_request_id_of_the_peer() {
        #[derive(Debug, Clone, Copy)]
        enum Before {
            /// The PUBLISH is Established.
            Established,
            /// An update with Request ID 1 was refused.
            UpdateRefused,
            /// This endpoint sent `PUBLISH_DONE`.
            PublishDone,
            /// This endpoint sent `PUBLISH_DONE`, and an update with Request
            /// ID 1 crossed it.
            UpdateAfterPublishDone,
            /// The peer opened a request with Request ID 1.
            PeerRequest,
        }
        // One that is refused where it is answered at all.
        let update = |request_id| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters: vec![Parameter::Forward(false)],
            })
        };
        let refused = || RequestEvent::UpdateRefused {
            response: ControlMessage::RequestError(RequestError {
                error_code: 0x3,
                retry_interval: 0,
                error_reason: ReasonPhrase::default(),
                redirect: None,
            }),
        };
        // This endpoint is the client, so the peer's Request IDs are odd.
        // (what came before; the Request ID of the update; what the update
        //  leads to)
        let cases = [
            (Before::Established, 1, Ok(refused())),
            (Before::Established, 5, Ok(refused())),
            (Before::Established, 0, Err(Error::InvalidRequestId(0))),
            (Before::UpdateRefused, 1, Err(Error::InvalidRequestId(1))),
            (
                Before::UpdateRefused,
                3,
                Ok(RequestEvent::UpdateAlreadyRefused),
            ),
            (Before::PublishDone, 1, Ok(RequestEvent::LateMessage)),
            (Before::PublishDone, 2, Err(Error::InvalidRequestId(2))),
            (
                Before::UpdateAfterPublishDone,
                1,
                Err(Error::InvalidRequestId(1)),
            ),
            (
                Before::UpdateAfterPublishDone,
                3,
                Ok(RequestEvent::LateMessage),
            ),
            (Before::PeerRequest, 1, Err(Error::InvalidRequestId(1))),
            (Before::PeerRequest, 3, Ok(refused())),
        ];
        let subscribe = |request_id, name: &[u8]| {
            ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name: track(name),
                parameters: vec![],
            })
        };
        for (before, request_id, expected) in cases {
            let mut session = client();
            let (publish, _) = session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
            session.feed(publish, request_ok(vec![])).unwrap();
            match before {
                Before::Established => {}
                Before::UpdateRefused => {
                    session.feed(publish, update(1)).unwrap();
                }
                Before::PublishDone | Before::UpdateAfterPublishDone => {
                    session
                        .publish_done(publish, 2, 0, ReasonPhrase::default())
                        .unwrap();
                    if matches!(before, Before::UpdateAfterPublishDone) {
                        session.feed(publish, update(1)).unwrap();
                    }
                }
                Before::PeerRequest => {
                    session.on_peer_request_opened(subscribe(1, b"b")).unwrap();
                }
            }
            assert_eq!(
                session.feed(publish, update(request_id)),
                expected,
                "{before:?} {request_id}"
            );
            // Whatever became of the update, a request of the peer cannot
            // have its Request ID: it was used up, or it never was one of
            // the peer's.
            assert_eq!(
                session
                    .on_peer_request_opened(subscribe(request_id, b"c"))
                    .err(),
                Some(Error::InvalidRequestId(request_id)),
                "{before:?} {request_id}"
            );
            // And the session goes on taking Request IDs that are new.
            assert!(
                session.on_peer_request_opened(subscribe(99, b"c")).is_ok(),
                "{before:?} {request_id}"
            );
        }
    }

    #[test]
    fn an_update_of_a_published_subscription_is_applied_when_it_only_sets_the_subgroup_timeout() {
        // (the parameters of the update; the timeout that is reported, or
        // `Err` where the update is refused).
        let cases = [
            (vec![], Ok((None, None))),
            (
                vec![Parameter::SubgroupDeliveryTimeout(250)],
                Ok((Some(250), None)),
            ),
            (
                vec![Parameter::SubgroupDeliveryTimeout(0)],
                Ok((Some(0), None)),
            ),
            (
                vec![Parameter::ObjectDeliveryTimeout(250)],
                Ok((None, None)),
            ),
            (vec![Parameter::NewGroupRequest(0)], Ok((None, Some(0)))),
            (vec![Parameter::NewGroupRequest(12)], Ok((None, Some(12)))),
            (
                vec![
                    Parameter::SubgroupDeliveryTimeout(250),
                    Parameter::Forward(true),
                ],
                Err(()),
            ),
        ];
        for (parameters, expected) in cases {
            let mut session = client();
            let (publish, _) = session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
            session.feed(publish, request_ok(vec![])).unwrap();
            let update = |request_id, parameters| {
                ControlMessage::RequestUpdate(RequestUpdate {
                    request_id,
                    parameters,
                })
            };
            let reported = match session
                .feed(publish, update(1, parameters.clone()))
                .unwrap()
            {
                RequestEvent::SubscriptionUpdated {
                    response: ControlMessage::RequestOk(_),
                    subgroup_delivery_timeout,
                    new_group_request,
                } => Ok((subgroup_delivery_timeout, new_group_request)),
                RequestEvent::UpdateRefused { .. } => Err(()),
                other => panic!("{parameters:?}: {other:?}"),
            };
            assert_eq!(reported, expected, "{parameters:?}");
            // Applied, the subscription takes another and ends as it likes.
            if expected.is_ok() {
                assert!(matches!(
                    session.feed(publish, update(3, vec![])),
                    Ok(RequestEvent::SubscriptionUpdated { .. })
                ));
                assert!(
                    session
                        .publish_done(publish, 2, 0, ReasonPhrase::default())
                        .is_ok()
                );
            }
        }
    }

    #[test]
    fn a_subscriber_updates_its_subscription_and_reads_the_answers_in_order() {
        let timeout = |milliseconds| vec![Parameter::SubgroupDeliveryTimeout(milliseconds)];
        let update = |request_id, parameters| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters,
            })
        };
        let mut session = client();
        let (subscribe, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        // Before SUBSCRIBE_OK there is no subscription to update.
        assert_eq!(
            session.update_subscription(subscribe, timeout(250)),
            Err(UsageError("not an established subscription of a SUBSCRIBE"))
        );
        session.feed(subscribe, subscribe_ok(7, vec![])).unwrap();
        // §10.2.10: EXPIRES is the publisher's to send.
        assert_eq!(
            session.update_subscription(subscribe, vec![Parameter::Expires(5)]),
            Err(UsageError("parameter not allowed in this message"))
        );
        // A REQUEST_OK that answers nothing is no message of this stream.
        assert!(session.feed(subscribe, request_ok(vec![])).is_err());

        let mut session = client();
        let (subscribe, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        session.feed(subscribe, subscribe_ok(7, vec![])).unwrap();
        // §10.1: each update has a Request ID of this endpoint's own, after
        // that of the SUBSCRIBE (0).
        assert_eq!(
            session.update_subscription(subscribe, timeout(250)),
            Ok(update(2, timeout(250)))
        );
        assert_eq!(
            session.update_subscription(subscribe, vec![]),
            Ok(update(4, vec![]))
        );
        assert_eq!(
            session.feed(subscribe, request_ok(vec![Parameter::Expires(9)])),
            Ok(RequestEvent::UpdateAccepted {
                parameters: vec![Parameter::Expires(9)]
            })
        );
        // After PUBLISH_DONE there is no subscription to update (§5.1).
        let mut ended = client();
        let (over, _) = ended.subscribe(track(b"a"), vec![]).unwrap();
        ended.feed(over, subscribe_ok(7, vec![])).unwrap();
        ended
            .feed(over, ControlMessage::PublishDone(publish_done()))
            .unwrap();
        assert_eq!(
            ended.update_subscription(over, vec![]),
            Err(UsageError("not an established subscription of a SUBSCRIBE"))
        );
        // `MAX_UNANSWERED_UPDATES` wait at most, and one that is not sent
        // uses up no Request ID: the next one after an answer has the ID
        // that follows the sixteenth.
        let mut waiting = client();
        let (asked, _) = waiting.subscribe(track(b"a"), vec![]).unwrap();
        waiting.feed(asked, subscribe_ok(7, vec![])).unwrap();
        for sent in 0..16 {
            assert_eq!(
                waiting.update_subscription(asked, vec![]),
                Ok(update(2 + 2 * sent, vec![]))
            );
        }
        assert_eq!(
            waiting.update_subscription(asked, vec![]),
            Err(UsageError("too many updates wait for their answer"))
        );
        waiting.feed(asked, request_ok(vec![])).unwrap();
        assert_eq!(
            waiting.update_subscription(asked, vec![]),
            Ok(update(34, vec![]))
        );
        // The refusal answers what is left, and PUBLISH_DONE ends it.
        assert_eq!(
            session.feed(subscribe, ControlMessage::RequestError(request_error())),
            Ok(RequestEvent::UpdateFailed(request_error()))
        );
        assert!(session.feed(subscribe, request_ok(vec![])).is_err());
    }

    /// The contract `fixtures/update-answers/answers.tsv` pins for both
    /// implementations, for the subscriber of a SUBSCRIBE.
    #[test]
    fn a_subscriber_reads_the_answers_to_its_updates_as_the_shared_table_says() {
        for (name, steps, events) in update_answer_steps() {
            let mut session = client();
            let (subscribe, _) = session.subscribe(track(b"a"), vec![]).unwrap();
            session.feed(subscribe, subscribe_ok(7, vec![])).unwrap();
            let mut done = Vec::new();
            for step in steps.split(' ') {
                let message = match step {
                    "update" => {
                        session.update_subscription(subscribe, vec![]).unwrap();
                        done.push("-".to_string());
                        continue;
                    }
                    "ok" => request_ok(vec![]),
                    "error" => ControlMessage::RequestError(request_error()),
                    "done" => ControlMessage::PublishDone(publish_done()),
                    other => panic!("{name}: a step the test does not know: {other}"),
                };
                done.push(match session.feed(subscribe, message) {
                    Ok(RequestEvent::UpdateAccepted { .. }) => "accepted".to_string(),
                    Ok(RequestEvent::UpdateFailed(_)) => "failed".to_string(),
                    Ok(RequestEvent::UpdateAnsweredAfterRefusal) => "after-refusal".to_string(),
                    Ok(RequestEvent::SubscriptionDone(_)) => "done".to_string(),
                    Err(error) => closed(&error),
                    other => panic!("{name}: {step}: {other:?}"),
                });
                if done.last().is_some_and(|last| last.starts_with("closed")) {
                    break;
                }
            }
            assert_eq!(&done.join(" | "), &events, "{name}");
        }
    }

    #[test]
    fn a_request_update_is_refused_where_the_peer_may_send_one() {
        // Every update has a Request ID of its own (§10.1).
        let update = |request_id| {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters: vec![Parameter::Forward(false)],
            })
        };
        let refused = RequestEvent::UpdateRefused {
            response: ControlMessage::RequestError(RequestError {
                error_code: 0x3,
                retry_interval: 0,
                error_reason: ReasonPhrase::default(),
                redirect: None,
            }),
        };

        let mut session = client();
        let (publish, _) = session.publish(track(b"a"), 1, vec![], vec![]).unwrap();
        // Before PUBLISH_OK there is no subscription to update.
        assert_eq!(session.feed(publish, update(1)), Err(NOT_ALLOWED));
        session.feed(publish, request_ok(vec![])).unwrap();
        // The update that was not allowed did not use up its Request ID: the
        // session is to be closed for it anyway.
        assert_eq!(session.feed(publish, update(1)), Ok(refused));
        assert_eq!(
            session.request_state(publish),
            Some(RequestState::Established)
        );
        // Further updates get no second answer, and nothing else is taken.
        assert_eq!(
            session.feed(publish, update(3)),
            Ok(RequestEvent::UpdateAlreadyRefused)
        );
        assert_eq!(session.feed(publish, request_ok(vec![])), Err(NOT_ALLOWED));
        // The subscription can only end with UPDATE_FAILED (0x8).
        assert_eq!(
            session.publish_done(publish, 0x2, 1, ReasonPhrase::default()),
            Err(UsageError(
                "a refused REQUEST_UPDATE ends the subscription with UPDATE_FAILED"
            ))
        );
        assert_eq!(
            session.publish_done(publish, 0x8, 1, ReasonPhrase::default()),
            Ok(ControlMessage::PublishDone(PublishDone {
                status_code: 0x8,
                stream_count: 1,
                error_reason: ReasonPhrase::default(),
            }))
        );
        assert_eq!(
            session.request_state(publish),
            Some(RequestState::Terminated)
        );

        // The publisher of a track this endpoint subscribed to may not update.
        let (subscribe, _) = session.subscribe(track(b"b"), vec![]).unwrap();
        session.feed(subscribe, subscribe_ok(7, vec![])).unwrap();
        assert_eq!(session.feed(subscribe, update(5)), Err(NOT_ALLOWED));
    }

    #[test]
    fn publish_done_needs_a_live_publish() {
        let rejected = Err(UsageError(
            "PUBLISH_DONE ends a live subscription opened with PUBLISH",
        ));
        let mut session = client();

        let (subscribe, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        assert_eq!(
            session.publish_done(subscribe, 2, 0, ReasonPhrase::default()),
            rejected
        );

        let (publish, _) = session.publish(track(b"b"), 1, vec![], vec![]).unwrap();
        session.cancel(publish);
        assert_eq!(
            session.publish_done(publish, 2, 0, ReasonPhrase::default()),
            rejected
        );
    }

    #[test]
    fn a_handle_from_another_session_is_not_a_request_here() {
        let (foreign, _) = client().subscribe(track(b"a"), vec![]).unwrap();
        let mut session = client();
        // The same Request ID is live in this session: the handle is told
        // apart by its session, not by its number.
        let (own, _) = session.subscribe(track(b"a"), vec![]).unwrap();
        assert_eq!(own.value(), foreign.value());
        let other_session = UsageError("the request belongs to another session");

        assert_eq!(session.request_state(foreign), None);
        assert_eq!(
            session.on_request_message(foreign, subscribe_ok(7, vec![])),
            Err(RequestFault::Usage(other_session))
        );
        assert_eq!(
            session.publish_done(foreign, 2, 0, ReasonPhrase::default()),
            Err(other_session)
        );
        session.cancel(foreign);
        assert_eq!(session.request_state(own), Some(RequestState::Pending));
    }

    #[test]
    fn refuses_to_send_a_parameter_in_the_wrong_message() {
        assert_eq!(
            client()
                .subscribe(track(b"a"), vec![Parameter::Expires(1)])
                .map(|_| ()),
            Err(UsageError("parameter not allowed in this message"))
        );
    }
}
