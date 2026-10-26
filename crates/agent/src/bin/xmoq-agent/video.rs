//! The video of a broadcast on one session, for its pictures: the catalog
//! is read once, and its first H.264 track kept on the session
//! (`xmoq_viewer_io::kept`), as `xmoq-watch` keeps its sound.

use anyhow::{Result, anyhow};
use xmoq_moqt::RequestFault;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::Session;
use xmoq_viewer_io::kept::{Asking, Came, KeptTrack, Said, Wanted};
use xmoq_viewer_io::session::{CatalogKept, Events, Requests};

/// The video of one session.
pub struct Video {
    kept: KeptTrack,
    _catalog: CatalogKept,
}

impl Video {
    /// Reads the catalog of `namespace` and subscribes to its first H.264
    /// video track.
    ///
    /// WHY the first: the demo's publisher puts its main version first, and
    /// `xmoq-watch` takes it where nothing else is asked.
    pub async fn begin(
        asking_with: Asking,
        namespace: TrackNamespace,
        session: &mut Session,
        events: &mut Events,
    ) -> Result<Self> {
        let mut requests = Requests {
            connection: &asking_with.connection,
            session,
            namespace: &namespace,
            token: asking_with.token.as_deref(),
            answer_wait_ms: asking_with.answer_wait_ms,
            open_wait_ms: asking_with.open_wait_ms,
            rendezvous_ms: None,
        };
        let (tracks, catalog) = requests.catalog(events, &asking_with.stops).await?;
        let name = xmoq_msf::video_tracks(&tracks, "avc")
            .next()
            .ok_or_else(|| anyhow!("the catalog has no H.264 video track with LOC packaging"))?
            .name
            .clone();
        let subscription = events
            .while_waiting(requests.subscribe(&name, None))
            .await??;
        eprintln!(
            "XMOQ_AGENT_VIDEO track={name} track_alias={}",
            subscription.track_alias
        );
        let wanted = Wanted {
            namespace,
            name,
            rendezvous_ms: None,
            payload_most: None,
            again_after_failure: false,
        };
        Ok(Self {
            kept: KeptTrack::subscribed(wanted, asking_with, subscription),
            _catalog: catalog,
        })
    }

    /// The Track Alias of the video, while it is subscribed to.
    pub fn track_alias(&self) -> Option<u64> {
        self.kept.track_alias()
    }

    /// What comes next.
    pub async fn next(&mut self) -> Came {
        self.kept.next().await
    }

    /// What came. `Some` with why where the video is given up on this
    /// session: a SUBSCRIBE that could not be sent, or was not answered, as
    /// `xmoq-watch` gives up its sound.
    ///
    /// # Errors
    ///
    /// As `KeptTrack::on`: the session is to be closed.
    pub fn on(
        &mut self,
        session: &mut Session,
        came: Came,
    ) -> Result<Option<String>, RequestFault> {
        match self.kept.on(session, came)? {
            Some(Said::Subscribed { track_alias }) => {
                eprintln!("XMOQ_AGENT_VIDEO_AGAIN track_alias={track_alias}");
            }
            Some(Said::Ended { how, pause_ms }) => {
                eprintln!("XMOQ_AGENT_VIDEO_ENDED {how}; asking again in {pause_ms} ms");
            }
            Some(Said::Refused {
                error_code,
                pause_ms,
            }) => eprintln!(
                "XMOQ_AGENT_VIDEO_REFUSED error_code={error_code:#x}; asking again in {pause_ms} ms"
            ),
            // Not asked again on this session, as `again_after_failure` says.
            Some(Said::GivenUp(why) | Said::Failed { why, .. }) => return Ok(Some(why)),
            Some(Said::Other(other)) => eprintln!("XMOQ_AGENT_VIDEO_SAID {other:?}"),
            None => {}
        }
        Ok(None)
    }

    /// Gives the subscription up, where there is one.
    pub fn cancel(self) {
        self.kept.cancel();
    }
}
