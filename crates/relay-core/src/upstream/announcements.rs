//! The namespaces the sessions of this relay announce, passed on to the
//! upstream relay (§9.5: "Relays MUST forward `PUBLISH_NAMESPACE` or PUBLISH
//! messages to all matching subscribers"), so that what is asked there for
//! them comes down to their publishers here: the chat's gatherer above, and
//! participants of the chat below.
//!
//! One `PUBLISH_NAMESPACE` of the relay on the session with the upstream for
//! each namespace of the upstream's prefix that at least one session of its
//! own announced, whoever and however many they are; it is withdrawn, by
//! cancelling it (§6.2), when the last of them withdraws or goes. A session
//! with the upstream that is new (after one went, or after a GOAWAY) is sent
//! them all again. The upstream's own sessions are left out: what the
//! upstream announces it has.
//!
//! WHY one for a namespace: the upstream asks the relay for a track of it
//! once, whoever has it here, and the relay asks its own publishers.
//! WHY the session is opened for an announcement alone, and kept for as long
//! as there is one: the gatherer above asks for the lines of a participant
//! whenever it says one, and with no session it could not reach it.

use std::collections::BTreeSet;

use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestEvent, RequestId};
use xmoq_moqt::{RequestFault, stream_reset_code};

use super::Link;
use crate::routing::RETRY_WHAT_IS_NOT_THERE_MS;
use crate::{Action, Event, OwnRequestEnd, OwnRequestKey, Relay, SessionKey, internal_error};

/// A `PUBLISH_NAMESPACE` of the relay on the session with the upstream.
#[derive(Debug)]
pub(crate) struct Passed {
    session: SessionKey,
    request: OwnRequestKey,
    id: RequestId,
    /// The upstream refused it, or ended its stream: it is not sent again
    /// on that session, which would get the same answer.
    over: bool,
}

impl Relay {
    /// The namespaces to be announced upstream: those of its prefix that a
    /// session announced which the relay is not the client of.
    fn to_pass_on(&self) -> BTreeSet<TrackNamespace> {
        let Some(upstream) = &self.upstream else {
            return BTreeSet::new();
        };
        self.namespaces
            .iter()
            .filter(|entry| upstream.namespace.is_prefix_of(&entry.namespace))
            .filter(|entry| !self.is_client_session(entry.session))
            .map(|entry| entry.namespace.clone())
            .collect()
    }

    /// Whether the relay opened `session` as the client: one with an
    /// upstream relay, now or before its GOAWAY.
    pub(crate) fn is_client_session(&self, session: SessionKey) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|state| state.moqt.role() == xmoq_moqt::session::Role::Client)
    }

    /// Brings what is announced upstream in line with what the sessions of
    /// the relay announce: withdraws what nobody announces any more, and
    /// announces what is new, on the session with the upstream, which is
    /// asked for where there is none.
    pub(crate) fn pass_announcements_upstream(&mut self, actions: &mut Vec<Action>) {
        let wanted = self.to_pass_on();
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        let withdrawn: Vec<TrackNamespace> = upstream
            .announced
            .keys()
            .filter(|namespace| !wanted.contains(*namespace))
            .cloned()
            .collect();
        let gone: Vec<Passed> = withdrawn
            .iter()
            .filter_map(|namespace| upstream.announced.remove(namespace))
            .collect();
        let missing: Vec<TrackNamespace> = wanted
            .into_iter()
            .filter(|namespace| !upstream.announced.contains_key(namespace))
            .collect();
        let link = upstream.link;
        for passed in gone {
            self.withdraw_upstream(&passed, actions);
        }
        if missing.is_empty() {
            return;
        }
        match link {
            Link::Open(session) => {
                for namespace in missing {
                    self.announce_upstream(session, namespace, actions);
                }
            }
            Link::Connecting => {}
            Link::Absent => self.connect_upstream_after(0, actions),
        }
    }

    /// Asks for the session with the upstream, `after_ms` from now.
    pub(crate) fn connect_upstream_after(&mut self, after_ms: u64, actions: &mut Vec<Action>) {
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        upstream.link = Link::Connecting;
        actions.push(Action::ConnectUpstream {
            uri: upstream.uri.clone(),
            after_ms,
        });
    }

    /// No session with the upstream came, and namespaces wait to be
    /// announced there: it is asked for again after a second, and so on
    /// while they wait. WHY: a participant of the chat here is not heard
    /// above until it is announced, and nothing else would ask.
    pub(crate) fn connect_again_for_announcements(&mut self, actions: &mut Vec<Action>) {
        if !self.to_pass_on().is_empty() {
            self.connect_upstream_after(RETRY_WHAT_IS_NOT_THERE_MS, actions);
        }
    }

    fn announce_upstream(
        &mut self,
        session: SessionKey,
        namespace: TrackNamespace,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // §10.2.1: none of the parameters of the announcements here.
        let Ok((id, message)) = state.moqt.publish_namespace(namespace.clone(), Vec::new()) else {
            return;
        };
        let request = OwnRequestKey(self.next_own_request);
        self.next_own_request = self.next_own_request.saturating_add(1);
        actions.push(Action::OpenRequest {
            session,
            request,
            message,
        });
        if let Some(upstream) = self.upstream.as_mut() {
            upstream.announced.insert(
                namespace,
                Passed {
                    session,
                    request,
                    id,
                    over: false,
                },
            );
        }
    }

    /// §6.2: a `PUBLISH_NAMESPACE` is withdrawn "by cancelling the request".
    fn withdraw_upstream(&mut self, passed: &Passed, actions: &mut Vec<Action>) {
        if passed.over {
            return;
        }
        let Some(state) = self.sessions.get_mut(&passed.session) else {
            return;
        };
        state.moqt.cancel(passed.id);
        actions.push(Action::CancelOwnRequest {
            session: passed.session,
            request: passed.request,
            code: stream_reset_code::CANCELLED,
        });
    }

    /// What was announced on `session` is no longer: it went, or is going
    /// away. The next session is sent it all again.
    pub(crate) fn forget_announcements_on(&mut self, session: SessionKey) {
        if let Some(upstream) = self.upstream.as_mut() {
            upstream
                .announced
                .retain(|_, passed| passed.session != session);
        }
    }

    /// Whether the relay announces something on `session`, which then
    /// holds something (`Relay::holds_nothing`).
    pub(crate) fn announces_upstream_on(&self, session: SessionKey) -> bool {
        self.upstream.as_ref().is_some_and(|upstream| {
            upstream
                .announced
                .values()
                .any(|passed| passed.session == session && !passed.over)
        })
    }

    fn passed_of(&mut self, session: SessionKey, request: OwnRequestKey) -> Option<&mut Passed> {
        self.upstream
            .as_mut()?
            .announced
            .values_mut()
            .find(|passed| passed.session == session && passed.request == request)
    }

    /// Takes the events about the relay's announcements upstream. Returns
    /// every other event.
    pub(crate) fn on_announcement_event(
        &mut self,
        event: Event,
        actions: &mut Vec<Action>,
    ) -> Option<Event> {
        match event {
            Event::OwnRequestMessage {
                session,
                request,
                message,
            } if self.passed_of(session, request).is_some() => {
                self.on_passed_message(session, request, message, actions);
                None
            }
            Event::OwnRequestClosed {
                session,
                request,
                how,
            } if self.passed_of(session, request).is_some() => {
                // A STOP_SENDING leaves the upstream's direction, and the
                // announcement with it (§3.3.2).
                if how != OwnRequestEnd::Stopped
                    && let Some(passed) = self.passed_of(session, request)
                {
                    passed.over = true;
                    let id = passed.id;
                    if let Some(state) = self.sessions.get_mut(&session) {
                        state.moqt.cancel(id);
                    }
                }
                None
            }
            other => Some(other),
        }
    }

    fn on_passed_message(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(id) = self.passed_of(session, request).map(|passed| passed.id) else {
            return;
        };
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        match state.moqt.on_request_message(id, message) {
            Ok(RequestEvent::Rejected(_)) => {
                if let Some(passed) = self.passed_of(session, request) {
                    passed.over = true;
                }
            }
            // Accepted, or a GOAWAY on its stream, which the relay does not
            // take as one of the upstream (known gap).
            Ok(_) => {}
            Err(RequestFault::Peer(error)) => self.fail_session(session, &error, actions),
            Err(RequestFault::Usage(error)) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
            }
        }
    }
}
