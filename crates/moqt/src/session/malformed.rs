//! What a subscriber cancels when a track turns out malformed (§2.4.2).

use super::{PeerRequestId, Request, RequestId, Session, SubscribedAlias};

/// A request [`Session::cancel_malformed`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedRequest {
    /// A SUBSCRIBE of this endpoint.
    Subscribe(RequestId),
    /// A PUBLISH of the peer that this endpoint took.
    Publish(PeerRequestId),
    /// A FETCH of this endpoint.
    Fetch(RequestId),
}

impl Session {
    /// §2.4.2: "When a subscriber detects a Malformed Track, it MUST cancel
    /// any corresponding subscription or fetches for that Track from that
    /// publisher (see Section 3.3.2)". Ends in this session every
    /// subscription and FETCH of the track that `track_alias` names, and
    /// retires the alias. Returns them, in the order of their Request IDs,
    /// for the caller to cancel their streams: the request stream of each
    /// with `MALFORMED_TRACK`, the fetch stream of a FETCH too, and the
    /// subgroup streams of the alias (§10.11). Nothing for an alias that
    /// names no track any more: it is Retired, or Unknown.
    ///
    /// WHY a subscription that `PUBLISH_DONE` ended (a Draining alias) also
    /// has its FETCH requests cancelled: they are of the same track, from
    /// the same publisher.
    pub fn cancel_malformed(&mut self, track_alias: u64) -> Vec<MalformedRequest> {
        let track = match self.subscribed_aliases.state(track_alias) {
            SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => track.clone(),
            SubscribedAlias::Retired | SubscribedAlias::Unknown => return Vec::new(),
        };
        let mut own: Vec<(u64, MalformedRequest)> = self
            .requests
            .iter()
            .filter_map(|(id, request)| {
                let handle = RequestId(*id, self.tag);
                match request {
                    Request::Subscribe {
                        full_track_name, ..
                    } if *full_track_name == track => {
                        Some((*id, MalformedRequest::Subscribe(handle)))
                    }
                    Request::Fetch(fetch) if fetch.track == track => {
                        Some((*id, MalformedRequest::Fetch(handle)))
                    }
                    _ => None,
                }
            })
            .collect();
        own.sort_by_key(|(id, _)| *id);
        let mut cancelled: Vec<MalformedRequest> =
            own.into_iter().map(|(_, request)| request).collect();
        cancelled.extend(
            self.peer_publishes_of(&track)
                .into_iter()
                .map(MalformedRequest::Publish),
        );
        for request in &cancelled {
            match *request {
                MalformedRequest::Subscribe(request) | MalformedRequest::Fetch(request) => {
                    self.cancel(request);
                }
                MalformedRequest::Publish(request) => self.on_peer_request_closed(request),
            }
        }
        // A Draining alias has no request left that would retire it.
        self.subscribed_aliases.retire(track_alias);
        cancelled
    }
}
