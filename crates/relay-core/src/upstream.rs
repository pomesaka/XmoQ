//! A relay with another relay above it: an edge of a tree of relays
//! (`docs/scale-and-meetings.md`). It is given a namespace prefix, and asks
//! the upstream relay for a track of it that no session of its own
//! announced, as it asks a publisher that announced the namespace. §6:
//! "Given sufficient out of band information, it is valid for a subscriber
//! to send a SUBSCRIBE or FETCH message to a publisher (including a relay)
//! without any previous MOQT messages besides SETUP."
//!
//! WHY the session is opened when a request needs it, and again after it
//! went (decided on 2026-10-25): a relay closes a session that holds nothing
//! after 30 seconds (`HubConfig::idle_session_timeout` of `xmoq-relay`), so
//! the upstream closes the edge's session whenever the edge has nobody to
//! ask for, and so does the edge itself. Opened on demand, that is no
//! failure, and the upstream needs no rule of its own for edges. WHY NOT a
//! session opened at the start and kept: the upstream would have to tell an
//! edge from a viewer to leave its session open, and without that the edge
//! would connect again every 30 seconds for nothing.

use std::collections::BTreeSet;

use xmoq_moqt::message::{ReasonPhrase, request_error_code, retry_interval};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};

use crate::routing::{RETRY_WHAT_IS_NOT_THERE_MS, Refusal};
use crate::{Action, Relay, SessionKey};

/// The upstream relay, as far as the relay knows it.
#[derive(Debug)]
pub(crate) struct Upstream {
    /// The tracks it is asked for: those whose namespace this is a prefix of.
    /// No fields: every track.
    namespace: TrackNamespace,
    link: Link,
    /// Where [`Action::ConnectUpstream`] connects: the New Session URI of
    /// the last GOAWAY of the upstream that named one, and empty for the
    /// place the relay was given.
    uri: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    /// No session, and none asked for.
    Absent,
    /// [`Action::ConnectUpstream`] is under way.
    Connecting,
    Open(SessionKey),
}

impl Relay {
    /// Gives the relay an upstream relay: a track whose namespace has
    /// `namespace` as a prefix and that no session announced is asked of it
    /// ([`Action::ConnectUpstream`]). A `namespace` without fields takes
    /// every track.
    pub fn set_upstream(&mut self, namespace: TrackNamespace) {
        self.upstream = Some(Upstream {
            namespace,
            link: Link::Absent,
            uri: Vec::new(),
        });
    }

    /// Whether the upstream is the one to ask for `track`: it takes the
    /// track's namespace, and no session announced one of it.
    ///
    /// WHY not where a session announced: with announcements passed on
    /// upstream (to come), the upstream would ask this relay for the track
    /// again, and the relay would take it twice, once from the publisher and
    /// once through the upstream.
    fn upstream_takes(&self, track: &FullTrackName) -> bool {
        let announced = self
            .namespaces
            .iter()
            .any(|entry| entry.namespace.is_prefix_of(track.namespace()));
        !announced
            && self
                .upstream
                .as_ref()
                .is_some_and(|upstream| upstream.namespace.is_prefix_of(track.namespace()))
    }

    /// The session with the upstream, where it is the one to ask for
    /// `track` and is open.
    pub(crate) fn upstream_for(&self, track: &FullTrackName) -> Option<SessionKey> {
        match self.upstream.as_ref()?.link {
            Link::Open(session) if self.upstream_takes(track) => Some(session),
            _ => None,
        }
    }

    /// Whether a request for `track` is to wait for a session with the
    /// upstream, which is asked for here where nobody has asked for it yet.
    pub(crate) fn waits_for_upstream(
        &mut self,
        track: &FullTrackName,
        actions: &mut Vec<Action>,
    ) -> bool {
        if !self.upstream_takes(track) {
            return false;
        }
        let Some(upstream) = self.upstream.as_mut() else {
            return false;
        };
        match upstream.link {
            Link::Open(_) => false,
            Link::Connecting => true,
            Link::Absent => {
                upstream.link = Link::Connecting;
                actions.push(Action::ConnectUpstream {
                    uri: upstream.uri.clone(),
                });
                true
            }
        }
    }

    /// A session the relay opened as the client is there: the upstream is
    /// asked for the tracks that wait for it, and for those that are
    /// subscribed to and that it is the one to ask for: after a GOAWAY of
    /// the upstream before, the tracks the session that is going away
    /// still carries. The FETCH and `TRACK_STATUS` requests that wait for
    /// it are asked anew.
    ///
    /// WHY the tracks that have a source as well (§3.6: the client
    /// "establishes a new session in the background"): the new session
    /// becomes one more source of each (§9.3), and the subscribers go on
    /// with it when the old one ends, where they would otherwise be told
    /// `PUBLISH_DONE` and have to subscribe again.
    pub(crate) fn upstream_opened(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        upstream.link = Link::Open(session);
        let namespace = upstream.namespace.clone();
        let tracks: BTreeSet<FullTrackName> = self
            .tracks
            .subscribed_in(&namespace)
            .into_iter()
            .filter(|track| self.upstream_takes(track))
            .collect();
        for track in tracks {
            let asked = self.ask_the_publishers(&track, Some(session), None, actions);
            let waits_alone = self
                .tracks
                .pending(&track)
                .is_some_and(|pending| pending.asks.is_empty());
            if waits_alone {
                let code = asked
                    .nobody_because
                    .unwrap_or(request_error_code::DOES_NOT_EXIST);
                self.refuse_waiters(&track, &Refusal::code(code), actions);
            }
        }
        self.ask_fetches_held_for_upstream(actions);
        self.ask_track_statuses_held_for_upstream(actions);
    }

    /// The upstream sent GOAWAY on the control stream of `session` (§3.6,
    /// §10.4): no new request goes on it, the relay connects to the URI it
    /// names (or to the same place, where it names none), and asks there
    /// for what it is subscribed to ([`Self::upstream_opened`]).
    ///
    /// WHY at once, where nothing new is asked: §3.6 has the client
    /// establish the new session "in the background" and close the old one
    /// once it has nothing left on it, and the upstream closes the old one
    /// when its Timeout is over.
    pub(crate) fn upstream_going_away(
        &mut self,
        session: SessionKey,
        uri: Vec<u8>,
        actions: &mut Vec<Action>,
    ) {
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        if upstream.link != Link::Open(session) {
            return;
        }
        upstream.link = Link::Absent;
        if !uri.is_empty() {
            upstream.uri = uri;
        }
        let namespace = upstream.namespace.clone();
        let carried = self.tracks.subscribed_in(&namespace);
        if let Some(track) = carried.iter().find(|track| self.upstream_takes(track)) {
            let track = track.clone();
            self.waits_for_upstream(&track, actions);
        }
    }

    /// No session with the upstream came of [`Action::ConnectUpstream`].
    pub(crate) fn upstream_failed(&mut self, actions: &mut Vec<Action>) {
        let Some(upstream) = self.upstream.as_mut() else {
            return;
        };
        upstream.link = Link::Absent;
        // WHY INTERNAL_ERROR, and asked again after a second: §10.6.2 has no
        // code for a relay that cannot reach its upstream ("INTERNAL_ERROR:
        // An implementation specific or generic error occurred"), and the
        // upstream may be reached later, as a publisher may come.
        let unreachable = Refusal::new(
            request_error_code::INTERNAL_ERROR,
            retry_interval::after_ms(RETRY_WHAT_IS_NOT_THERE_MS),
            ReasonPhrase::new(b"the upstream relay could not be reached").unwrap_or_default(),
        );
        for track in self.waiting_for_upstream() {
            self.refuse_waiters(&track, &unreachable, actions);
        }
        self.refuse_fetches_held_for_upstream(&unreachable, actions);
        self.refuse_track_statuses_held_for_upstream(&unreachable, actions);
    }

    /// The session with the upstream is gone: the next request that needs
    /// it opens another.
    pub(crate) fn upstream_gone(&mut self, session: SessionKey) {
        if let Some(upstream) = self.upstream.as_mut()
            && upstream.link == Link::Open(session)
        {
            upstream.link = Link::Absent;
        }
    }

    /// The tracks without a source that nobody is asked for and that the
    /// upstream is the one to ask, by name, so that the same events lead to
    /// the same actions.
    fn waiting_for_upstream(&self) -> BTreeSet<FullTrackName> {
        self.tracks
            .all_pending()
            .filter(|(track, pending)| pending.asks.is_empty() && self.upstream_takes(track))
            .map(|(track, _)| track.clone())
            .collect()
    }
}
