//! Session and request state (§3.3, §5.1, §6.2, §10.1, §11.1).
//!
//! This file holds the SETUP exchange and the requests this endpoint opens;
//! [`peer`] holds the requests the peer opens.

mod peer;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

pub use peer::{
    AfterRefusal, MAX_PEER_REQUESTS, PeerRequest, PeerRequestEvent, PeerRequestId,
    PeerRequestOpened, REQUEST_IDS_AHEAD_LIMIT,
};

pub use crate::aliases::SubscribedAlias;
use crate::aliases::SubscribedAliases;
use crate::kvp::KeyValuePair;
use crate::message::{
    ControlMessage, Publish, PublishDone, PublishNamespace, ReasonPhrase, RequestError, Setup,
    Subscribe, SubscribeOk, publish_done_status, request_error_code, setup_option,
};
use crate::name::{FullTrackName, TrackNamespace};
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
    },
    Publish {
        full_track_name: FullTrackName,
        track_alias: u64,
        state: PublishState,
    },
    PublishNamespace {
        established: bool,
    },
}

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
            Self::PublishNamespace { established } => *established,
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
    /// `SUBSCRIBE_OK` named a Mandatory Track Property, none of which this crate
    /// understands. The request is Terminated and the caller has to cancel the
    /// stream (§2.5.1).
    UnsupportedTrack {
        /// Type of the first Mandatory Track Property found.
        property_type: u64,
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
    /// `REQUEST_ERROR`: the peer rejected the request, which is now Terminated.
    Rejected(RequestError),
    /// `PUBLISH_DONE` ended a subscription this endpoint asked for. The request
    /// is Terminated; the Track Alias keeps draining, see
    /// [`SubscribedAlias::Draining`].
    SubscriptionDone(PublishDone),
    /// `REQUEST_UPDATE` from the subscriber of a track this endpoint publishes.
    /// This crate applies no updates. The caller sends `response`, a
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
}

/// The state of one MOQT session, as far as the endpoint that opens requests
/// needs it: the SETUP exchange, Request IDs, Track Aliases and open requests.
#[derive(Debug)]
pub struct Session {
    role: Role,
    transport: Transport,
    setup_sent: bool,
    peer_setup_received: bool,
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

impl Session {
    /// A session before any SETUP is exchanged.
    #[must_use]
    pub fn new(role: Role, transport: Transport) -> Self {
        Self {
            role,
            transport,
            setup_sent: false,
            peer_setup_received: false,
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

    /// Whether both SETUP messages have been exchanged.
    #[must_use]
    pub fn is_established(&self) -> bool {
        self.setup_sent && self.peer_setup_received
    }

    /// Feeds a message read from the peer's control stream and returns the
    /// peer's SETUP.
    ///
    /// # Errors
    ///
    /// A protocol error when the first message is not SETUP, when the SETUP holds
    /// an option its sender may not use, or for any message after the SETUP. The
    /// session then has to be closed with [`Error::termination_code`].
    pub fn on_control_message(&mut self, message: ControlMessage) -> Result<Setup, Error> {
        if self.peer_setup_received {
            // After SETUP only GOAWAY belongs on a control stream (Table 5), and
            // the decoder does not produce it.
            return Err(Error::ProtocolViolation(
                "message not allowed on the control stream",
            ));
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
        Ok(setup)
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
        })?;
        let message = ControlMessage::Subscribe(Subscribe {
            request_id: request_id.0,
            full_track_name,
            parameters,
        });
        Ok((request_id, message))
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
        Ok(self.on_own_request_message(request, message)?)
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
            if self.ended_by_publish_done.contains(&request.0) {
                return Ok(RequestEvent::LateMessage);
            }
            return Err(Error::ProtocolViolation("message after the request ended"));
        };
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
            // FIXME: check the Request ID the update consumes (§10.1), together
            // with the other Request IDs of the peer.
            (Request::Publish { state, .. }, ControlMessage::RequestUpdate(_))
                if *state == PublishState::Established =>
            {
                *state = PublishState::UpdateRefused;
                Ok(RequestEvent::UpdateRefused {
                    response: ControlMessage::RequestError(RequestError {
                        error_code: request_error_code::NOT_SUPPORTED,
                        // 0: retrying the same update will not help.
                        retry_interval: 0,
                        error_reason: ReasonPhrase::default(),
                        redirect: None,
                    }),
                })
            }
            (
                Request::Publish {
                    state: PublishState::UpdateRefused,
                    ..
                },
                ControlMessage::RequestUpdate(_),
            ) => Ok(RequestEvent::UpdateAlreadyRefused),
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
            // §5.1 and §6.2: exactly one response per request, and every other
            // message decoded here either opens a request or answers a different
            // kind.
            _ => Err(Error::ProtocolViolation(
                "message not allowed in this request state",
            )),
        }
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
        if let Some(property) = ok
            .track_properties
            .iter()
            .find(|property| property.is_mandatory_track_property())
        {
            let property_type = property.ty();
            self.end_request(request);
            // The publisher may already be sending under the alias. Remembered
            // as retired, its streams are told apart from ones that are merely
            // ahead of their SUBSCRIBE_OK (§11.1).
            self.subscribed_aliases.retire_if_unknown(ok.track_alias);
            return Ok(RequestEvent::UnsupportedTrack {
                property_type,
                track_alias: ok.track_alias,
            });
        }
        if let Some(Request::Subscribe {
            full_track_name,
            track_alias,
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
            Some(Request::Subscribe { .. } | Request::PublishNamespace { .. }) | None => {}
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
                Request::Subscribe { full_track_name: name, track_alias: None }
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
    use crate::message::{Redirect, RequestOk, RequestUpdate};
    use crate::params::GroupOrder;

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
            Ok(peer_setup.clone())
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

    #[test]
    fn a_received_setup_may_repeat_an_option() {
        let implementation = || KeyValuePair::bytes(0x07, b"relay".to_vec()).unwrap();
        let setup = Setup {
            setup_options: vec![implementation(), implementation()],
        };
        assert_eq!(
            client().on_control_message(ControlMessage::Setup(setup.clone())),
            Ok(setup)
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
                    Ok(RequestEvent::UnsupportedTrack {
                        property_type: 0x4000,
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
        let update = || {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id: 1,
                parameters: vec![],
            })
        };
        // (whether the PUBLISH was accepted before PUBLISH_DONE, what arrives after)
        let cases = [
            (false, vec![request_ok(vec![]), update(), update()]),
            (false, vec![ControlMessage::RequestError(request_error())]),
            (true, vec![update(), request_ok(vec![])]),
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
                session.feed(request, update()),
                Err(ENDED),
                "accepted {accepted}"
            );
        }
    }

    #[test]
    fn a_request_update_is_refused_where_the_peer_may_send_one() {
        let update = || {
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id: 1,
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
        assert_eq!(session.feed(publish, update()), Err(NOT_ALLOWED));
        session.feed(publish, request_ok(vec![])).unwrap();
        assert_eq!(session.feed(publish, update()), Ok(refused));
        assert_eq!(
            session.request_state(publish),
            Some(RequestState::Established)
        );
        // Further updates get no second answer, and nothing else is taken.
        assert_eq!(
            session.feed(publish, update()),
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
        assert_eq!(session.feed(subscribe, update()), Err(NOT_ALLOWED));
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
