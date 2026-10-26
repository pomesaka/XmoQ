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
//!
//! **What the upstream refuses with a Retry Interval is sent again after
//! it** (§10.6.2: "The minimum time (in milliseconds) before the request
//! SHOULD be sent again, plus one"), and from the first such refusal on the
//! session the rest go one at a time, the next when the one before is
//! answered ([`Pace`]). What it refuses with none (0, "SHOULD NOT be
//! retried") is not sent again on that session. WHY: the upstream counts
//! this relay as one session, and the own relay takes five announcements of
//! a session at once and one more a second (`MAX_ANNOUNCEMENTS_AT_ONCE`);
//! the relay sends what its sessions announce all at once when its session
//! with the upstream opens, so that the sixth participant of a chat below
//! was refused and never heard above. WHY one at a time after a refusal:
//! all of them again after each Retry Interval would have the upstream
//! refuse all but one each time, as many refusals a second as there are
//! waiting. WHY NOT one announcement of a prefix for all of them: the
//! chat's gatherer above finds its participants by the namespace each one
//! announces.
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
use crate::{
    Action, Event, Operation, OwnRequestEnd, OwnRequestKey, Relay, SessionKey, internal_error,
};

/// How the namespaces to be announced upstream are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pace {
    /// As soon as they are there: nothing was refused for the upstream's
    /// load on this session.
    AllAtOnce,
    /// Until the Retry Interval of a refusal is over, nothing.
    Waiting,
    /// One at a time: the next once this request of the relay is answered.
    OneAtATime(OwnRequestKey),
}

/// A `PUBLISH_NAMESPACE` of the relay on the session with the upstream.
#[derive(Debug)]
pub(crate) struct Passed {
    session: SessionKey,
    request: OwnRequestKey,
    id: RequestId,
    /// The upstream refused it without a Retry Interval, or ended its
    /// stream: it is not sent again on that session, which would get the
    /// same answer.
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
        let (link, pace) = (upstream.link, upstream.pace);
        for passed in gone {
            self.withdraw_upstream(&passed, actions);
        }
        if missing.is_empty() {
            return;
        }
        match link {
            Link::Open(session) if pace == Pace::AllAtOnce => {
                for namespace in missing {
                    self.announce_upstream(session, namespace, actions);
                }
            }
            // What is missing goes when the one sent is answered, or when
            // the Retry Interval is over; or when the session is there.
            Link::Open(_) | Link::Connecting => {}
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

    /// Sends the first of what is to be announced and is not, alone, and
    /// goes on one at a time; with nothing to send, all at once again.
    fn announce_next_upstream(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        let wanted = self.to_pass_on();
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        let next = wanted
            .into_iter()
            .find(|namespace| !upstream.announced.contains_key(namespace));
        upstream.pace = Pace::AllAtOnce;
        if let Some(namespace) = next
            && let Some(request) = self.announce_upstream(session, namespace, actions)
            && let Some(upstream) = self.upstream.as_mut()
        {
            upstream.pace = Pace::OneAtATime(request);
        }
    }

    /// [`Event::UpstreamAnnouncementTimeout`].
    pub(crate) fn on_upstream_announcement_timeout(
        &mut self,
        session: SessionKey,
        actions: &mut Vec<Action>,
    ) {
        let waiting = self.upstream.as_ref().is_some_and(|upstream| {
            upstream.pace == Pace::Waiting && upstream.link == Link::Open(session)
        });
        if waiting {
            self.announce_next_upstream(session, actions);
        }
    }

    /// The request the announcement went in, where it went.
    fn announce_upstream(
        &mut self,
        session: SessionKey,
        namespace: TrackNamespace,
        actions: &mut Vec<Action>,
    ) -> Option<OwnRequestKey> {
        // §10.2.1: none of the parameters of the announcements here, and the
        // relay's own token (`Relay::token_for`).
        let parameters = self
            .token_for(session, Operation::Publish)
            .into_iter()
            .collect();
        let state = self.sessions.get_mut(&session)?;
        let (id, message) = state
            .moqt
            .publish_namespace(namespace.clone(), parameters)
            .ok()?;
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
        Some(request)
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
            upstream.pace = Pace::AllAtOnce;
        }
    }

    /// Whether the relay announces something on `session`, or waits to,
    /// which then holds something (`Relay::holds_nothing`).
    pub(crate) fn announces_upstream_on(&self, session: SessionKey) -> bool {
        self.upstream.as_ref().is_some_and(|upstream| {
            (upstream.link == Link::Open(session) && upstream.pace != Pace::AllAtOnce)
                || upstream
                    .announced
                    .values()
                    .any(|passed| passed.session == session && !passed.over)
        })
    }

    /// The request of the relay that went one at a time was answered, or
    /// ended: the next goes.
    fn answered_one_at_a_time(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let its_turn = self
            .upstream
            .as_ref()
            .is_some_and(|upstream| upstream.pace == Pace::OneAtATime(request));
        if its_turn {
            self.announce_next_upstream(session, actions);
        }
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
                    // What is left open of the stream is the relay's direction.
                    actions.push(Action::CancelOwnRequest {
                        session,
                        request,
                        code: stream_reset_code::CANCELLED,
                    });
                    self.answered_one_at_a_time(session, request, actions);
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
            // The session ended the request with the answer; the stream is
            // still open until the relay finishes its direction.
            Ok(RequestEvent::Rejected(error)) if error.retry_interval > 0 => {
                actions.push(Action::FinishOwnRequest { session, request });
                self.refused_for_now(session, request, error.retry_interval - 1, actions);
            }
            Ok(RequestEvent::Rejected(_)) => {
                actions.push(Action::FinishOwnRequest { session, request });
                if let Some(passed) = self.passed_of(session, request) {
                    passed.over = true;
                }
                self.answered_one_at_a_time(session, request, actions);
            }
            // Accepted, or a GOAWAY on its stream, which the relay does not
            // take as one of the upstream (known gap).
            Ok(_) => self.answered_one_at_a_time(session, request, actions),
            Err(RequestFault::Peer(error)) => self.fail_session(session, &error, actions),
            Err(RequestFault::Usage(error)) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
            }
        }
    }

    /// The upstream refused an announcement and says it may be sent again
    /// in `wait_ms`: it is to be announced again, and what is to be sent
    /// waits that long and then goes one at a time. The timer is started
    /// once for the refusals of a burst.
    fn refused_for_now(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        wait_ms: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        upstream
            .announced
            .retain(|_, passed| !(passed.session == session && passed.request == request));
        if upstream.pace != Pace::Waiting {
            upstream.pace = Pace::Waiting;
            actions.push(Action::StartUpstreamAnnouncementTimer { session, wait_ms });
        }
    }
}
