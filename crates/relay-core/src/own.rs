//! The requests the relay opens on a session, its own: one way to open one
//! and one way to close one, whatever it asks.
//!
//! WHY one way: a kind of request that opens or closes its own by hand can
//! leave a step out, and a request the relay does not close keeps a task
//! and a stream on the I/O side until the session ends. Where each request
//! is kept (`SessionState::own`, `own_fetches`, `own_track_statuses`,
//! `Parent::announced`) stays with what it asks for.

use xmoq_moqt::message::{ControlMessage, request_error_code};
use xmoq_moqt::session::{RequestId, Session};

use crate::{Action, OwnRequestKey, Relay, SessionKey, SessionState};

/// Whether a request of the relay counts among the requests it may have
/// open on a session (`LimitsOfASession::own_requests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Counted {
    /// It counts, and `over_a_share` is a limit of its kind that the
    /// caller found reached: either refuses it with `EXCESSIVE_LOAD`.
    Yes { over_a_share: bool },
    /// It does not: a `PUBLISH_NAMESPACE` to the parent. WHY not: the
    /// parent bounds them itself, taking no more namespaces of this relay's
    /// session than its `--namespaces-per-session` (decided on 2026-10-26);
    /// and an announcement that waited for room among the relay's own
    /// SUBSCRIBE and FETCH requests would leave a participant below unheard
    /// above.
    No,
}

/// How a request of the relay is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnEnd {
    /// The session ended the request with the peer's answer: the relay
    /// finishes its direction of the stream.
    Answered,
    /// The relay gives the request up (§3.3.2): the session lets go of `id`,
    /// and the stream is reset both ways with `code`.
    GivenUp { id: RequestId, code: u32 },
    /// The session holds no Request ID for it any more (it refused the
    /// peer's answer, or the request was let go of already): the stream is
    /// reset both ways with `code`.
    Reset { code: u32 },
}

impl Relay {
    /// Opens a request of the relay on `session` with the message `make`
    /// has the session write, and has `keep` keep it, with its key and the
    /// session's Request ID, where its kind keeps them. `timed` asks for
    /// [`Action::StartUpstreamTimer`]. Returns the key.
    ///
    /// WHY `keep` here and not after the call: a request that is opened and
    /// kept nowhere is closed by nobody.
    ///
    /// # Errors
    ///
    /// The Error Code for whoever waits on the request: `DOES_NOT_EXIST`
    /// for a session that is not there or sent GOAWAY (§10.4: "an endpoint
    /// SHOULD NOT initiate new requests to the peer"), `EXCESSIVE_LOAD` for
    /// a limit, or what `make` returns.
    pub(crate) fn open_own_request(
        &mut self,
        session: SessionKey,
        (counted, timed): (Counted, bool),
        make: impl FnOnce(&mut Session) -> Result<(RequestId, ControlMessage), u64>,
        keep: impl FnOnce(&mut SessionState, OwnRequestKey, RequestId),
        actions: &mut Vec<Action>,
    ) -> Result<OwnRequestKey, u64> {
        let own_requests_most = self.limits_of_a_session.own_requests;
        let request = OwnRequestKey(self.next_own_request);
        let Some(state) = self.sessions.get_mut(&session) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        if state.moqt.peer_going_away() {
            return Err(request_error_code::DOES_NOT_EXIST);
        }
        if let Counted::Yes { over_a_share } = counted
            && (over_a_share || state.own_requests() >= own_requests_most)
        {
            return Err(request_error_code::EXCESSIVE_LOAD);
        }
        let (id, message) = make(&mut state.moqt)?;
        keep(state, request, id);
        self.next_own_request = self.next_own_request.saturating_add(1);
        actions.push(Action::OpenRequest {
            session,
            request,
            message,
        });
        if timed {
            actions.push(Action::StartUpstreamTimer { session, request });
        }
        Ok(request)
    }

    /// Closes a request of the relay on `session`; the I/O side stops its
    /// [`Action::StartUpstreamTimer`] with it. The caller has let go of it
    /// where it kept it. Nothing where the session is gone: so is the
    /// stream.
    pub(crate) fn close_own_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        end: OwnEnd,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let code = match end {
            OwnEnd::Answered => {
                actions.push(Action::FinishOwnRequest { session, request });
                return;
            }
            OwnEnd::GivenUp { id, code } => {
                state.moqt.cancel(id);
                code
            }
            OwnEnd::Reset { code } => code,
        };
        actions.push(Action::CancelOwnRequest {
            session,
            request,
            code,
        });
    }
}
