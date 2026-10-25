//! The video of a broadcast on one session, for its pictures: the catalog
//! is read once, its first H.264 track subscribed to, and asked for again
//! when its subscription ends or is refused, by the rule of `xmoq-again`, as
//! `xmoq-watch` asks for its sound again.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use xmoq_again::AskAgain;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestEvent, Session};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{RequestReadError, RequestStreamEvent, StreamStops};
use xmoq_viewer_io::session::{
    Asked, CatalogKept, Events, NotSubscribed, Requests, Sent, Subscription, send_subscribe, settle,
};

/// A SUBSCRIBE for the video that was sent again, on its way to its answer.
type Answer = std::pin::Pin<Box<dyn std::future::Future<Output = (Asked, Result<Sent>)> + Send>>;

/// What the video has to be told of, from the loop of the session.
pub enum Came {
    /// What came on the stream of the subscription.
    Answer(Result<RequestStreamEvent, RequestReadError>),
    /// The time to ask for the video again has come.
    AskNow,
    /// What came of the SUBSCRIBE sent again, boxed: it is the largest.
    Settled(Box<(Asked, Result<Sent>)>),
}

/// What the requests of the video are sent with, kept to ask again.
pub struct Asking {
    /// Where the streams of the requests are opened.
    pub connection: Arc<WtConnection>,
    /// The broadcast's namespace.
    pub namespace: TrackNamespace,
    /// The session's token, where the room gave one.
    pub token: Option<Vec<u8>>,
    /// `--answer-wait-ms`.
    pub answer_wait_ms: u64,
    /// `--open-wait-ms`.
    pub open_wait_ms: u64,
}

/// The video of one session.
pub struct Video {
    name: String,
    asking_with: Asking,
    _catalog: CatalogKept,
    /// The subscription, while there is one.
    subscription: Option<Subscription>,
    again: AskAgain,
    ask_at: Option<tokio::time::Instant>,
    asking: Option<Answer>,
}

/// Waits for `item` where there is one, and for ever where there is none.
async fn on<T>(item: Option<impl std::future::Future<Output = T>>) -> T {
    match item {
        Some(future) => future.await,
        None => std::future::pending().await,
    }
}

impl Video {
    /// Reads the catalog and subscribes to its first H.264 video track.
    ///
    /// WHY the first: the demo's publisher puts its main version first, and
    /// `xmoq-watch` takes it where nothing else is asked.
    pub async fn begin(
        asking_with: Asking,
        session: &mut Session,
        events: &mut Events,
        stops: &StreamStops,
    ) -> Result<Self> {
        let mut requests = Requests {
            connection: &asking_with.connection,
            session,
            namespace: &asking_with.namespace,
            token: asking_with.token.as_deref(),
            answer_wait_ms: asking_with.answer_wait_ms,
            open_wait_ms: asking_with.open_wait_ms,
            rendezvous_ms: None,
        };
        let (tracks, catalog) = requests.catalog(events, stops).await?;
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
        Ok(Self {
            name,
            asking_with,
            _catalog: catalog,
            subscription: Some(subscription),
            again: AskAgain::default(),
            ask_at: None,
            asking: None,
        })
    }

    /// The Track Alias of the video, while it is subscribed to.
    pub fn track_alias(&self) -> Option<u64> {
        self.subscription
            .as_ref()
            .map(|subscription| subscription.track_alias)
    }

    /// What comes next.
    pub async fn next(&mut self) -> Came {
        let Self {
            subscription,
            ask_at,
            asking,
            ..
        } = self;
        tokio::select! {
            event = on(subscription.as_mut().map(|subscription| subscription.reader.next_event())) => Came::Answer(event),
            () = on(ask_at.map(tokio::time::sleep_until)) => Came::AskNow,
            settled = on(asking.as_mut()) => Came::Settled(Box::new(settled)),
        }
    }

    /// What came. `Err` with why where the video is given up on this
    /// session: a SUBSCRIBE that could not be sent, or was not answered, as
    /// `xmoq-watch` gives up its sound.
    pub fn on(&mut self, session: &mut Session, came: Came) -> Result<(), String> {
        match came {
            Came::Answer(event) => {
                self.ended(session, event);
                Ok(())
            }
            Came::AskNow => {
                self.ask_at = None;
                let mut requests = Requests {
                    connection: &self.asking_with.connection,
                    session,
                    namespace: &self.asking_with.namespace,
                    token: self.asking_with.token.as_deref(),
                    answer_wait_ms: self.asking_with.answer_wait_ms,
                    open_wait_ms: self.asking_with.open_wait_ms,
                    rendezvous_ms: None,
                };
                let asked = requests
                    .ask(&self.name, None)
                    .map_err(|error| format!("{error:#}"))?;
                let connection = Arc::clone(&self.asking_with.connection);
                let open_wait_ms = self.asking_with.open_wait_ms;
                self.asking = Some(Box::pin(async move {
                    let sent = send_subscribe(&connection, open_wait_ms, &asked).await;
                    (asked, sent)
                }));
                Ok(())
            }
            Came::Settled(settled) => {
                self.asking = None;
                let (asked, sent) = *settled;
                match settle(session, &asked, sent) {
                    Ok(subscription) => {
                        self.again.subscribed();
                        eprintln!(
                            "XMOQ_AGENT_VIDEO_AGAIN track_alias={}",
                            subscription.track_alias
                        );
                        self.subscription = Some(subscription);
                        Ok(())
                    }
                    Err(NotSubscribed::Refused(error)) => {
                        let pause = self.again.after_refusal(error.retry_wait_ms());
                        eprintln!(
                            "XMOQ_AGENT_VIDEO_REFUSED error_code={:#x}; asking again in {pause} ms",
                            error.error_code
                        );
                        self.ask_in(pause);
                        Ok(())
                    }
                    Err(NotSubscribed::Failed(error)) => Err(format!("{error:#}")),
                }
            }
        }
    }

    /// The subscription ended (`PUBLISH_DONE`), or its stream did: it is asked
    /// for again after a pause.
    fn ended(
        &mut self,
        session: &mut Session,
        event: Result<RequestStreamEvent, RequestReadError>,
    ) {
        let Some(subscription) = self.subscription.take() else {
            return;
        };
        let request = subscription.request;
        let how = match event {
            Ok(RequestStreamEvent::Message(message)) => {
                match session.on_request_message(request, message) {
                    Ok(RequestEvent::SubscriptionDone(done)) => {
                        format!("PUBLISH_DONE status_code={:#x}", done.status_code)
                    }
                    Ok(other) => {
                        // Anything else on the stream leaves the subscription
                        // as it is.
                        eprintln!("XMOQ_AGENT_VIDEO_SAID {other:?}");
                        self.subscription = Some(subscription);
                        return;
                    }
                    Err(error) => format!("{error}"),
                }
            }
            Ok(RequestStreamEvent::Closed(closed)) => {
                session.cancel(request);
                format!("closed {closed:?}")
            }
            Err(error) => {
                session.cancel(request);
                format!("{error}")
            }
        };
        subscription.cancel();
        let pause = self.again.after_end();
        eprintln!("XMOQ_AGENT_VIDEO_ENDED {how}; asking again in {pause} ms");
        self.ask_in(pause);
    }

    fn ask_in(&mut self, pause_ms: u64) {
        self.ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause_ms));
    }

    /// Gives the subscription up, where there is one.
    pub fn cancel(self) {
        if let Some(subscription) = self.subscription {
            subscription.cancel();
        }
    }
}
