//! The request slots of a session, and the one way to change them.
//!
//! WHY a module of its own: a Joining FETCH lives by the SUBSCRIBE it names,
//! and a place that ends a subscription without saying so leaves the FETCH
//! behind. The slots can be changed only by the three functions of [`Relay`]
//! here, each of which tells the Joining FETCH requests in the same breath
//! ([`Relay::joined`]). Outside this module the map cannot be reached, and
//! there is no value a caller could forget to pass on.

use std::collections::HashMap;

use crate::{Action, Relay, RequestSlot, SessionKey, StreamKey};

/// How far a SUBSCRIBE of the peer has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subscribed {
    /// The relay has not answered it: the track has no source yet.
    Waiting,
    /// Answered with `SUBSCRIBE_OK`.
    Accepted,
}

/// What a change of a request slot did to a subscription of the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Joinable {
    /// No subscription began or ended.
    Unchanged,
    /// A SUBSCRIBE that waited was accepted.
    Established,
    /// A SUBSCRIBE, waiting or accepted, is over.
    Over,
}

impl Joinable {
    /// From what the slot held of a SUBSCRIBE before and after.
    fn of(before: Option<Subscribed>, after: Option<Subscribed>) -> Self {
        match (before, after) {
            (Some(Subscribed::Waiting), Some(Subscribed::Accepted)) => Self::Established,
            (Some(_), None) => Self::Over,
            (None | Some(_), _) => Self::Unchanged,
        }
    }
}

/// The request streams the peer has open on a session, by stream.
///
/// WHY no `Default`: `std::mem::take` would empty the map from anywhere in
/// the crate, past the three functions below. Putting another value in its
/// place by assignment the type cannot keep anyone from.
#[derive(Debug)]
pub(crate) struct Requests(HashMap<StreamKey, RequestSlot>);

impl Requests {
    pub(crate) fn new() -> Self {
        Self(HashMap::new())
    }

    pub(crate) fn get(&self, stream: StreamKey) -> Option<&RequestSlot> {
        self.0.get(&stream)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&StreamKey, &RequestSlot)> {
        self.0.iter()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the peer finished its direction of a request stream while the
    /// request goes on.
    pub(crate) fn peer_finished(&self, stream: StreamKey) -> bool {
        self.get(stream).is_some_and(RequestSlot::peer_finished)
    }

    /// Notes that the peer finished its direction of a stream. `false` for
    /// a request that does not go on after that, and for no request.
    pub(crate) fn note_peer_finished(&mut self, stream: StreamKey) -> bool {
        self.0
            .get_mut(&stream)
            .is_some_and(RequestSlot::note_peer_finished)
    }

    fn put(&mut self, stream: StreamKey, slot: RequestSlot) -> Joinable {
        let after = slot.subscribed();
        let before = self.0.insert(stream, slot);
        Joinable::of(before.as_ref().and_then(RequestSlot::subscribed), after)
    }

    fn remove(&mut self, stream: StreamKey) -> (Option<RequestSlot>, Joinable) {
        let slot = self.0.remove(&stream);
        let change = Joinable::of(slot.as_ref().and_then(RequestSlot::subscribed), None);
        (slot, change)
    }
}

impl Relay {
    /// Puts a request slot in place, and tells the Joining FETCH requests
    /// that name the stream what that did to their subscription.
    pub(crate) fn put_slot(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        slot: RequestSlot,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let change = state.requests.put(stream, slot);
        self.joined(session, stream, change, actions);
    }

    /// Takes a request slot away, and tells the Joining FETCH requests that
    /// name the stream.
    pub(crate) fn take_slot(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) -> Option<RequestSlot> {
        let state = self.sessions.get_mut(&session)?;
        let (slot, change) = state.requests.remove(stream);
        self.joined(session, stream, change, actions);
        slot
    }

    /// Marks a request stream of the peer as one the relay has said its last
    /// on, with no state left for it in the session.
    ///
    /// WHY the slot goes at once when the peer had finished its direction: a
    /// slot is otherwise taken away when the stream is reported closed, and
    /// that report came already. It would stay for the life of the session.
    ///
    /// WHY the other places put `Finished` in directly: they answer a request
    /// in the step that opened it, or end a PUBLISH, and neither can have the
    /// mark; a FIN on a PUBLISH ends it there and then. A new place that
    /// answers a request later than it was opened belongs here.
    pub(crate) fn finish_slot(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let finished = self
            .sessions
            .get(&session)
            .is_some_and(|state| state.requests.peer_finished(stream));
        if finished {
            self.take_slot(session, stream, actions);
        } else {
            self.put_slot(session, stream, RequestSlot::Finished(None), actions);
        }
    }
}
