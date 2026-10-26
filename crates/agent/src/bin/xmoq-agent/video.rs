//! The video of a broadcast on one session, for its pictures: of the
//! catalog read once, its first H.264 track kept on the session
//! (`xmoq_subscriber::kept`), as `xmoq-watch` keeps its sound.

use anyhow::{Result, anyhow};
use xmoq_moqt::RequestFault;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestId, Session};
use xmoq_msf::CatalogTrack;
use xmoq_subscriber::kept::{Asking, Came, KeptTrack, Said, Wanted};
use xmoq_subscriber::session::{CatalogKept, Events, Requests};

/// The video of one session.
pub struct Video {
    kept: KeptTrack,
    /// The catalog's requests, where the video read the catalog itself.
    catalog: Option<CatalogKept>,
}

impl Video {
    /// Reads the catalog of `namespace` and subscribes to its first H.264
    /// video track ([`Self::of`]).
    pub async fn begin(
        asking_with: Asking,
        namespace: TrackNamespace,
        session: &mut Session,
        events: &mut Events,
    ) -> Result<Self> {
        let mut requests = Requests {
            connection: asking_with.connection.as_ref(),
            session,
            namespace: &namespace,
            token: asking_with.token.as_deref(),
            answer_wait_ms: asking_with.answer_wait_ms,
            open_wait_ms: asking_with.open_wait_ms,
            rendezvous_ms: None,
        };
        let (tracks, catalog) = requests.catalog(events, &asking_with.stops).await?;
        Self::of(
            asking_with,
            namespace,
            &tracks,
            Some(catalog),
            session,
            events,
        )
        .await
    }

    /// Subscribes to the first H.264 video track of `tracks`, the catalog of
    /// `namespace`, read with `catalog` where it is this video's to keep.
    ///
    /// WHY the first: as `xmoq-watch` takes it where nothing else is asked
    /// (`xmoq_watch::tracks`): MSF-01 gives the order of a catalog's tracks
    /// no meaning, and a publisher that wants a version taken unasked lists
    /// it first.
    pub async fn of(
        asking_with: Asking,
        namespace: TrackNamespace,
        tracks: &[CatalogTrack],
        catalog: Option<CatalogKept>,
        session: &mut Session,
        events: &mut Events,
    ) -> Result<Self> {
        let mut requests = Requests {
            connection: asking_with.connection.as_ref(),
            session,
            namespace: &namespace,
            token: asking_with.token.as_deref(),
            answer_wait_ms: asking_with.answer_wait_ms,
            open_wait_ms: asking_with.open_wait_ms,
            rendezvous_ms: None,
        };
        let name = xmoq_msf::video_tracks(tracks, "avc")
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
            catalog,
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
        Ok(self.kept.on(session, came)?.and_then(tell))
    }

    /// The session ended `request` for a malformed track
    /// (`Session::cancel_malformed`): whether it was of this video, its
    /// subscription (asked for again) or its catalog's SUBSCRIBE or FETCH.
    pub fn malformed(&mut self, request: RequestId) -> bool {
        if let Some(said) = self.kept.malformed(request) {
            tell(said);
            return true;
        }
        let of_the_catalog = self
            .catalog
            .as_mut()
            .is_some_and(|catalog| catalog.malformed(request));
        if of_the_catalog {
            eprintln!("XMOQ_AGENT_CATALOG_MALFORMED");
        }
        of_the_catalog
    }

    /// Gives the video up ([`KeptTrack::cancel`]), and its catalog
    /// ([`CatalogKept::cancel`]).
    pub fn cancel(self, session: &mut Session) {
        self.kept.cancel(session);
        if let Some(catalog) = self.catalog {
            catalog.cancel(session);
        }
    }
}

/// Writes what became of the video; `Some` with why where it is given up on
/// the session.
fn tell(said: Said) -> Option<String> {
    match said {
        Said::Subscribed { track_alias } => {
            eprintln!("XMOQ_AGENT_VIDEO_AGAIN track_alias={track_alias}");
        }
        Said::Ended { how, pause_ms } => {
            eprintln!("XMOQ_AGENT_VIDEO_ENDED {how}; asking again in {pause_ms} ms");
        }
        Said::Refused {
            error_code,
            pause_ms,
        } => eprintln!(
            "XMOQ_AGENT_VIDEO_REFUSED error_code={error_code:#x}; asking again in {pause_ms} ms"
        ),
        // Not asked again on this session, as `again_after_failure` says.
        Said::GivenUp(why) | Said::Failed { why, .. } => return Some(why),
        Said::Other(other) => eprintln!("XMOQ_AGENT_VIDEO_SAID {other:?}"),
    }
    None
}
