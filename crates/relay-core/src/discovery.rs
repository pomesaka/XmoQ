//! Namespace discovery (§6.1, §10.16 to §10.18): a session asks with
//! `SUBSCRIBE_NAMESPACE` for the namespaces announced under a prefix, and is
//! sent a NAMESPACE for each, now and as they come, and a `NAMESPACE_DONE`
//! when one is withdrawn.

use std::collections::BTreeSet;

use xmoq_moqt::message::{ControlMessage, SubscribeNamespace, request_error_code};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::PeerRequestId;
use xmoq_moqt::stream_reset_code;

use crate::routing::Refusal;
use crate::{Action, Relay, RequestSlot, SessionKey, StreamKey};

/// `SUBSCRIBE_NAMESPACE` requests one session may have open at once. One
/// more is refused with `EXCESSIVE_LOAD`. Decided on 2026-10-24. WHY a limit: each
/// is looked at with every announcement and every withdrawal, and keeps the
/// namespaces it was told of.
pub const MAX_NAMESPACE_SUBSCRIPTIONS_PER_SESSION: usize = 8;

/// Namespaces under the prefix of one `SUBSCRIBE_NAMESPACE` that the relay
/// tells of. Decided on 2026-10-24. One more at the request is refused with
/// `NAMESPACE_TOO_LARGE` (§10.6.2: "the namespace prefix matches more
/// publishers than the relay is willing to enumerate"); one more later resets
/// the request stream with `EXCESSIVE_LOAD` (§3.3.3), which the subscriber
/// takes as a `NAMESPACE_DONE` for each (§10.18).
///
/// WHY 4,096: the gatherer of the chat asks for the namespaces of every room
/// with one request, and the aim was 10 rooms of 100 people when this was set;
/// a relay takes 1,000 sessions now (`HubConfig::max_sessions` of
/// `xmoq-relay`). WHY a limit: the relay keeps, for each request, the
/// namespaces it told of.
pub const MAX_NAMESPACES_OF_A_SUBSCRIPTION: usize = 4096;

/// NAMESPACE and `NAMESPACE_DONE` of one namespace subscription that the relay
/// has sent and the I/O side has not reported written
/// ([`crate::Event::NamespaceWritten`]). The rest of what changed is sent as
/// these are written. Decided on 2026-10-24.
///
/// WHY a window, and not everything at once: what waits for a request stream
/// at the I/O side is bounded (256 messages, `session_queue_len` of
/// `crates/relay/src/hub.rs`), and a stream that overflows it is reset. WHY
/// 64: well below that bound, with the answer before it.
pub const NAMESPACE_WINDOW: usize = 64;

/// A `SUBSCRIBE_NAMESPACE` the relay accepted.
#[derive(Debug)]
pub(crate) struct NamespaceSubscription {
    session: SessionKey,
    stream: StreamKey,
    prefix: TrackNamespace,
    /// The namespaces the session was sent NAMESPACE for and no
    /// `NAMESPACE_DONE` since, whole.
    told: BTreeSet<TrackNamespace>,
    /// Sent and not reported written: at most [`NAMESPACE_WINDOW`].
    in_flight: usize,
}

impl Relay {
    /// A `SUBSCRIBE_NAMESPACE` of the peer, allowed by the relay's
    /// authorization.
    pub(crate) fn on_subscribe_namespace(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        message: &SubscribeNamespace,
        actions: &mut Vec<Action>,
    ) {
        let open = self
            .namespace_subscriptions
            .iter()
            .filter(|subscription| subscription.session == session)
            .count();
        if open >= MAX_NAMESPACE_SUBSCRIPTIONS_PER_SESSION {
            let too_many = Refusal::code(request_error_code::EXCESSIVE_LOAD);
            self.refuse(session, stream, request, &too_many, actions);
            return;
        }
        let prefix = &message.track_namespace_prefix;
        if self.announced_under(prefix) > MAX_NAMESPACES_OF_A_SUBSCRIPTION {
            let too_large = Refusal::code(request_error_code::NAMESPACE_TOO_LARGE);
            self.refuse(session, stream, request, &too_large, actions);
            return;
        }
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // §6.1: "the publisher MUST send a single REQUEST_OK or REQUEST_ERROR
        // as the first message on the bidirectional stream in response".
        let response = state.moqt.accept_subscribe_namespace(request);
        self.put_slot(
            session,
            stream,
            RequestSlot::SubscribeNamespace { request },
            actions,
        );
        self.respond(session, stream, response, actions);
        self.namespace_subscriptions.push(NamespaceSubscription {
            session,
            stream,
            prefix: message.track_namespace_prefix.clone(),
            told: BTreeSet::new(),
            in_flight: 0,
        });
        self.tell_the_namespace_subscriptions(actions);
    }

    /// The `SUBSCRIBE_NAMESPACE` on `stream` is over: the peer ended its
    /// direction, or reset it (§6.1: "can be cancelled by closing the stream
    /// with either a FIN or `RESET_STREAM`").
    pub(crate) fn end_namespace_subscription(&mut self, session: SessionKey, stream: StreamKey) {
        self.namespace_subscriptions.retain(|subscription| {
            !(subscription.session == session && subscription.stream == stream)
        });
    }

    /// The namespace subscriptions of a session that is gone.
    pub(crate) fn drop_namespace_subscriptions_of(&mut self, session: SessionKey) {
        self.namespace_subscriptions
            .retain(|subscription| subscription.session != session);
    }

    /// Sends every namespace subscription what changed of the namespaces under
    /// its prefix: a NAMESPACE for one it was not told of, a `NAMESPACE_DONE`
    /// for one it was told of that no session announces any more. Called after
    /// every change of the announced namespaces and of the namespace
    /// subscriptions.
    ///
    /// §9.5 (relays): "it MUST send a NAMESPACE message to any subscriber that
    /// has sent `SUBSCRIBE_NAMESPACE` for that namespace, or a prefix of that
    /// namespace". §10.18: "The publisher MUST NOT send `NAMESPACE_DONE` for a
    /// namespace suffix before the corresponding NAMESPACE", which `told`
    /// keeps. WHY a namespace two sessions announce is told once, and done
    /// when the last withdraws it: the subscriber is told of namespaces, and
    /// the relay serves the namespace while one announces it. WHY a session
    /// is told of the namespaces it announced itself: §6.1, "including
    /// echoing back Track Namespaces under the prefix that have been
    /// published to it".
    pub(crate) fn tell_the_namespace_subscriptions(&mut self, actions: &mut Vec<Action>) {
        let announced = self.announced();
        let mut too_many = Vec::new();
        for subscription in &mut self.namespace_subscriptions {
            if !tell(subscription, &announced, actions) {
                too_many.push((subscription.session, subscription.stream));
            }
        }
        for (session, stream) in too_many {
            self.reset_namespace_subscription(
                session,
                stream,
                stream_reset_code::EXCESSIVE_LOAD,
                actions,
            );
        }
    }

    /// One NAMESPACE or `NAMESPACE_DONE` of a namespace subscription was
    /// written: the next is sent, if something is left to tell.
    pub(crate) fn on_namespace_written(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let announced = self.announced();
        let Some(subscription) = self
            .namespace_subscriptions
            .iter_mut()
            .find(|subscription| subscription.session == session && subscription.stream == stream)
        else {
            return;
        };
        subscription.in_flight = subscription.in_flight.saturating_sub(1);
        if !tell(subscription, &announced, actions) {
            self.reset_namespace_subscription(
                session,
                stream,
                stream_reset_code::EXCESSIVE_LOAD,
                actions,
            );
        }
    }

    /// The namespaces announced now, each once.
    fn announced(&self) -> BTreeSet<TrackNamespace> {
        self.namespaces
            .iter()
            .map(|entry| entry.namespace.clone())
            .collect()
    }

    /// Ends a namespace subscription from the relay's side: its request is over
    /// in the session, and its stream is reset with `code` and stopped
    /// (§3.3.2), which the subscriber takes as a `NAMESPACE_DONE` for each
    /// namespace (§10.18).
    pub(crate) fn reset_namespace_subscription(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        code: u32,
        actions: &mut Vec<Action>,
    ) {
        self.end_namespace_subscription(session, stream);
        if let Some(RequestSlot::SubscribeNamespace { request }) =
            self.take_slot(session, stream, actions)
            && let Some(state) = self.sessions.get_mut(&session)
        {
            state.moqt.on_peer_request_closed(request);
        }
        actions.push(Action::ResetRequest {
            session,
            stream,
            code,
        });
    }

    /// How many namespaces are announced under `prefix`, each once.
    fn announced_under(&self, prefix: &TrackNamespace) -> usize {
        self.namespaces
            .iter()
            .map(|entry| &entry.namespace)
            .filter(|namespace| prefix.is_prefix_of(namespace))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// Sends `subscription` what changed of the namespaces under its prefix, within
/// its window: `NAMESPACE_DONE` for those it was told of that are gone first,
/// then NAMESPACE for those it was not told of. `false`: more than
/// [`MAX_NAMESPACES_OF_A_SUBSCRIPTION`] are under the prefix, and nothing is
/// sent.
fn tell(
    subscription: &mut NamespaceSubscription,
    announced: &BTreeSet<TrackNamespace>,
    actions: &mut Vec<Action>,
) -> bool {
    let now: BTreeSet<&TrackNamespace> = announced
        .iter()
        .filter(|namespace| subscription.prefix.is_prefix_of(namespace))
        .collect();
    if now.len() > MAX_NAMESPACES_OF_A_SUBSCRIPTION {
        return false;
    }
    let gone: Vec<TrackNamespace> = subscription
        .told
        .iter()
        .filter(|namespace| !now.contains(namespace))
        .cloned()
        .collect();
    let new: Vec<TrackNamespace> = now
        .into_iter()
        .filter(|namespace| !subscription.told.contains(*namespace))
        .cloned()
        .collect();
    let changes = gone
        .into_iter()
        .map(|namespace| (namespace, true))
        .chain(new.into_iter().map(|namespace| (namespace, false)));
    for (namespace, done) in changes {
        if subscription.in_flight >= NAMESPACE_WINDOW {
            break;
        }
        let suffix = suffix(&subscription.prefix, &namespace);
        let message = if done {
            subscription.told.remove(&namespace);
            ControlMessage::NamespaceDone(suffix)
        } else {
            subscription.told.insert(namespace);
            ControlMessage::Namespace(suffix)
        };
        subscription.in_flight += 1;
        actions.push(Action::SendRequest {
            session: subscription.session,
            stream: subscription.stream,
            message,
        });
    }
    true
}

/// The fields of `namespace` after `prefix` (§10.16: "only the namespace
/// tuples after the 'Track Namespace Prefix' are included in the 'Track
/// Namespace Suffix'"). `prefix` is a prefix of `namespace`.
fn suffix(prefix: &TrackNamespace, namespace: &TrackNamespace) -> TrackNamespace {
    let rest = namespace
        .fields()
        .get(prefix.fields().len()..)
        .unwrap_or_default()
        .to_vec();
    // Fewer fields of the same namespace keep within its limits.
    TrackNamespace::new(rest).unwrap_or_else(|_| namespace.clone())
}
