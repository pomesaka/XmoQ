//! What a viewer that connects again needs of its sockets, beside the rule
//! of when and where, and the times the room and the relay get
//! (`xmoq_viewer::reconnect`): the GOAWAY behind a refusal.
//!
//! FIXME: joining a room again and the wait before a try are written in
//! both programs, each with the `unless_ended` of its own loop; one here
//! once a wait that their signal ends is shared.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::Result;
use xmoq_moqt::message::GoAway;
use xmoq_moqt::session::{ControlEvent, Session};
use xmoq_moqt_io::UniEvent;

use crate::session::{Events, MOST_READ_AHEAD};

/// The GOAWAY behind a SUBSCRIBE refused with `GOING_AWAY` (§10.4), which
/// travels on the control stream and may be read after the refusal; `None`
/// where none comes within `wait_ms`. WHY a limit, the time an answer gets:
/// `GOING_AWAY` also comes from a peer that received a GOAWAY (§10.6.2) and
/// sends none. `@xmoq/media` waits the same.
///
/// What else comes meanwhile is kept for [`Events::recv`], before what comes
/// after, in its order: where no GOAWAY comes the session goes on, and an
/// Object of another track read in the wait is still that track's.
///
/// # Errors
///
/// A control message that breaks the session, and a connection that is gone.
pub async fn go_away_behind(
    session: &mut Session,
    events: &mut Events,
    wait_ms: u64,
) -> Result<Option<GoAway>> {
    let until = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    let mut kept = VecDeque::new();
    let found = loop {
        // As many as are kept while an answer is waited for: past them the
        // wait ends as one in which no GOAWAY came.
        if kept.len() >= MOST_READ_AHEAD {
            break Ok(None);
        }
        let Ok(event) = tokio::time::timeout_at(until, events.recv()).await else {
            break Ok(None);
        };
        match event {
            Some(UniEvent::Control(message)) => match session.on_control_message(message) {
                Ok(ControlEvent::GoAway(go_away)) => break Ok(Some(go_away)),
                // Taken by the session as the caller's loop would.
                Ok(_) => {}
                Err(error) => break Err(error.into()),
            },
            Some(event) => kept.push_back(event),
            None => break Err(anyhow::anyhow!("the connection is gone")),
        }
    };
    events.put_back(kept);
    found
}
