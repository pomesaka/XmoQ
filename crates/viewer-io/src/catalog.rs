//! The catalog of a broadcast that may not be there yet, followed on one
//! session: its SUBSCRIBE is kept as [`crate::kept`] keeps a track, held by
//! the relay for the `RENDEZVOUS_TIMEOUT` it carries and asked again by the
//! rule of `xmoq-again` after a refusal or an end, and once it is answered
//! the catalog is read by a Joining FETCH (`Requests::read_catalog`). The
//! loop of the session waits for the SUBSCRIBE's answer as one more arm
//! ([`FollowedCatalog::next`]), and goes on meanwhile; reading the catalog
//! once it was answered ([`FollowedCatalog::on`]) holds the loop for a
//! stream for the FETCH and 5 s at most, and the caller hears its signals
//! meanwhile.
//!
//! WHY a `KeptTrack` under it, and not a loop of its own: one rule of asking
//! again for every track the viewers keep, so that the catalog is asked
//! again as the video, the sound, the chat's lines and the transcript are.
//! WHY the FETCH here and not in `KeptTrack`: no other kept track reads its
//! latest Object before it is of use, and the FETCH's stream is the
//! caller's events, which `KeptTrack` does not see.

use anyhow::{Result, anyhow};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestId, Session};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::Connection;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_msf::CatalogTrack;

use crate::kept::{Asking, Came, KeptTrack, Said, Wanted};
use crate::session::{CATALOG, CATALOG_LIMIT, CatalogFetch, CatalogNotRead, Events, Requests};

/// What became of the broadcast, by its catalog.
#[derive(Debug)]
pub enum Became {
    /// The catalog came: the broadcast is there, with these tracks.
    Began(Vec<CatalogTrack>),
    /// The catalog's subscription ended, `how`: the broadcast is over, and
    /// its catalog is asked for again.
    Ended(String),
    /// The catalog is not there (yet), for `why`, and is asked for again in
    /// `pause_ms`.
    Waiting {
        /// Why, for a log.
        why: String,
        /// The pause before it is asked for again.
        pause_ms: u64,
    },
    /// Nothing that changes whether the broadcast is there.
    Nothing,
}

/// The catalog of one broadcast, followed on one session.
pub struct FollowedCatalog<C: Connection = WtConnection> {
    namespace: TrackNamespace,
    kept: KeptTrack<C>,
    /// The FETCH the catalog was read with, while it is held.
    fetch: Option<CatalogFetch<C>>,
    /// Whether [`Became::Began`] was said and no [`Became::Ended`] since.
    /// WHY apart from `fetch`: the FETCH can be given up (a malformed
    /// track) while the subscription goes on, and the end of the
    /// subscription is the end of the broadcast all the same.
    began: bool,
}

impl<C: Connection> FollowedCatalog<C> {
    /// The catalog of the broadcast `namespace`, asked for at once from the
    /// loop, its SUBSCRIBE with `rendezvous_ms` where it is given.
    #[must_use]
    pub fn new(
        namespace: TrackNamespace,
        rendezvous_ms: Option<u64>,
        asking_with: Asking<C>,
    ) -> Self {
        let wanted = Wanted {
            namespace: namespace.clone(),
            name: CATALOG.to_string(),
            rendezvous_ms,
            payload_most: Some(CATALOG_LIMIT),
            // WHY: a broadcast that is not there yet is what is waited for,
            // and a SUBSCRIBE the relay held and did not answer is one.
            again_after_failure: true,
        };
        Self {
            namespace,
            kept: KeptTrack::new(wanted, asking_with),
            fetch: None,
            began: false,
        }
    }

    /// Whether the catalog came, and its subscription has not ended since.
    #[must_use]
    pub fn is_there(&self) -> bool {
        self.began
    }

    /// The catalog's FETCH, while it is held.
    #[cfg(test)]
    fn fetch_request(&self) -> Option<RequestId> {
        self.fetch.as_ref().map(CatalogFetch::request)
    }

    /// The Track Alias of the catalog, while it is subscribed to.
    #[must_use]
    pub fn track_alias(&self) -> Option<u64> {
        self.kept.track_alias()
    }

    /// What comes next.
    pub async fn next(&mut self) -> Came<C> {
        self.kept.next().await
    }

    /// What came, and what became of the broadcast. Where the SUBSCRIBE was
    /// answered, the catalog is read here, `events` taken meanwhile as
    /// `Requests::read_catalog` takes them.
    ///
    /// # Errors
    ///
    /// What breaks the session: a message that breaks the protocol, the
    /// relay's GOAWAY (`GoneAway`), a FETCH that cannot be sent. A catalog
    /// that does not come in time or cannot be read is not one: the
    /// subscription is cut and asked for again.
    pub async fn on(
        &mut self,
        session: &mut Session,
        events: &mut Events,
        came: Came<C>,
    ) -> Result<Became> {
        let Some(said) = self.kept.on(session, came)? else {
            return Ok(Became::Nothing);
        };
        Ok(match said {
            Said::Subscribed { .. } => return self.read(session, events).await,
            Said::Ended { how, pause_ms } => self.ended(session, how, pause_ms),
            Said::Refused {
                error_code,
                pause_ms,
            } => Became::Waiting {
                why: format!("refused error_code={error_code:#x}"),
                pause_ms,
            },
            Said::Failed { why, pause_ms } => Became::Waiting { why, pause_ms },
            Said::GivenUp(why) => return Err(anyhow!("the catalog was given up: {why}")),
            Said::Other(_) => Became::Nothing,
        })
    }

    /// The session ended `request` for a malformed track
    /// (`Session::cancel_malformed`): what became of the broadcast where it
    /// was the catalog's SUBSCRIBE (asked for again) or FETCH (the catalog
    /// came already); `None` where it is another's.
    pub fn malformed(&mut self, session: &mut Session, request: RequestId) -> Option<Became> {
        if let Some(Said::Ended { how, pause_ms }) = self.kept.malformed(request) {
            return Some(self.ended(session, how, pause_ms));
        }
        if self
            .fetch
            .as_ref()
            .is_some_and(|fetch| fetch.request() == request)
        {
            if let Some(fetch) = self.fetch.take() {
                fetch.end(stream_reset_code::MALFORMED_TRACK);
            }
            return Some(Became::Nothing);
        }
        None
    }

    /// Gives the catalog up, in `session` too.
    pub fn cancel(self, session: &mut Session) {
        self.kept.cancel(session);
        if let Some(fetch) = self.fetch {
            fetch.cancel(session);
        }
    }

    /// The subscription ended: the broadcast where it had come.
    fn ended(&mut self, session: &mut Session, how: String, pause_ms: u64) -> Became {
        if let Some(fetch) = self.fetch.take() {
            fetch.cancel(session);
        }
        if std::mem::take(&mut self.began) {
            Became::Ended(how)
        } else {
            Became::Waiting { why: how, pause_ms }
        }
    }

    /// Reads the catalog of the subscription.
    async fn read(&mut self, session: &mut Session, events: &mut Events) -> Result<Became> {
        let Some(subscription) = self.kept.subscription() else {
            return Ok(Became::Nothing);
        };
        let (request, track_alias) = (subscription.request, subscription.track_alias);
        let asking = self.kept.asking_with();
        let stops = asking.stops.clone();
        let mut requests = Requests {
            connection: &*asking.connection,
            session,
            namespace: &self.namespace,
            token: asking.token.as_deref(),
            answer_wait_ms: asking.answer_wait_ms,
            open_wait_ms: asking.open_wait_ms,
            rendezvous_ms: None,
        };
        match requests
            .read_catalog(request, track_alias, events, &stops)
            .await
        {
            Ok((tracks, fetch)) => {
                self.fetch = Some(fetch);
                self.began = true;
                Ok(Became::Began(tracks))
            }
            Err(error) if error.downcast_ref::<CatalogNotRead>().is_some() => {
                let pause_ms = match self.kept.cut(session) {
                    Some(Said::Ended { pause_ms, .. }) => pause_ms,
                    _ => 0,
                };
                Ok(Became::Waiting {
                    why: format!("{error:#}"),
                    pause_ms,
                })
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;
    use xmoq_moqt::data::{ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject};
    use xmoq_moqt::message::{
        ControlMessage, PublishDone, ReasonPhrase, RequestError, SubscribeOk, request_error_code,
    };
    use xmoq_moqt::session::{Role, Transport};
    use xmoq_moqt_io::memory::{self, MemoryConnection};
    use xmoq_moqt_io::{RecvStream, SendStream, StreamStops, Trace, UniEvent, send_message};

    use super::*;

    /// What the relay does next.
    #[derive(Debug, Clone, Copy)]
    enum Relay {
        /// Answers the next SUBSCRIBE of the catalog with `REQUEST_ERROR` of
        /// this code and a Retry Interval of "at once".
        Refuses(u64),
        /// Answers it with `SUBSCRIBE_OK`, and sends the catalog.
        Gives,
        /// Ends the subscription it gave with `PUBLISH_DONE`.
        Ends,
        /// The track of the catalog's FETCH is found malformed
        /// (`Session::cancel_malformed`), the FETCH alone.
        BreaksTheFetch,
    }

    /// What `became` is written as in the table.
    fn written(became: &Became) -> String {
        match became {
            Became::Began(tracks) => format!("began {}", tracks.len()),
            Became::Ended(how) => format!("ended {how}"),
            Became::Waiting { why, pause_ms } => format!("waiting {why} {pause_ms}"),
            Became::Nothing => "nothing".to_string(),
        }
    }

    /// Accepts the next request stream and answers on it with `message`.
    async fn answer(
        relay: &MemoryConnection,
        message: ControlMessage,
    ) -> <MemoryConnection as Connection>::Send {
        let (mut send, _recv) = relay.accept_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "relay")
            .await
            .unwrap();
        send
    }

    /// The catalog of moqlivemock's publisher, as an Object of the catalog
    /// track.
    fn catalog_object(track_alias: u64) -> UniEvent {
        UniEvent::Object {
            stream: 3,
            header: SubgroupHeader {
                track_alias,
                group_id: 0,
                subgroup_id: SubgroupId::Explicit(0),
                publisher_priority: None,
                properties: false,
                end_of_group: true,
                first_object: true,
            },
            object: SubgroupObject {
                object_id: 0,
                properties: vec![],
                status: ObjectStatus::Normal,
                payload: include_bytes!("../../../fixtures/msf-01/catalog-moqlivemock.json")
                    .to_vec(),
            },
        }
    }

    /// What became of the broadcast after each of `steps`, and whether the
    /// catalog is there at the end.
    async fn followed(steps: &[Relay]) -> (Vec<String>, bool) {
        let (ours, relay) = memory::pair();
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        let (queue, received) = mpsc::channel(8);
        let mut events = Events::new(received);
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec(), b"demo".to_vec()]).unwrap();
        let mut catalog = FollowedCatalog::new(
            namespace,
            Some(crate::follow::RENDEZVOUS_MS),
            Asking {
                connection: Arc::new(ours),
                stops: StreamStops::default(),
                token: None,
                answer_wait_ms: 10_000,
                open_wait_ms: 10_000,
            },
        );
        let mut subscriptions = Vec::new();
        let mut became = Vec::new();
        for step in steps {
            if let Relay::BreaksTheFetch = step {
                let request = catalog.fetch_request().expect("a FETCH");
                let malformed = catalog.malformed(&mut session, request);
                became.push(malformed.map_or_else(|| "none".to_string(), |b| written(&b)));
                continue;
            }
            if let Relay::Ends = step {
                let mut send: <MemoryConnection as Connection>::Send =
                    subscriptions.pop().expect("a subscription to end");
                let done = ControlMessage::PublishDone(PublishDone {
                    status_code: 2,
                    stream_count: 0,
                    error_reason: ReasonPhrase::default(),
                });
                let trace = Trace::default();
                let (came, written_done) = tokio::join!(
                    catalog.next(),
                    send_message(&mut send, &done, &trace, "relay")
                );
                written_done.unwrap();
                let ended = catalog.on(&mut session, &mut events, came).await.unwrap();
                became.push(written(&ended));
                continue;
            }
            let ask_now = catalog.next().await;
            let asked = catalog
                .on(&mut session, &mut events, ask_now)
                .await
                .unwrap();
            assert_eq!(written(&asked), "nothing", "{steps:?}");
            let message = match step {
                Relay::Refuses(error_code) => ControlMessage::RequestError(RequestError {
                    error_code: *error_code,
                    retry_interval: 1,
                    error_reason: ReasonPhrase::default(),
                    redirect: None,
                }),
                _ => ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 1,
                    parameters: vec![],
                    track_properties: vec![],
                }),
            };
            let (settled, send) = tokio::join!(catalog.next(), answer(&relay, message));
            if let Relay::Gives = step {
                subscriptions.push(send);
                queue.send(catalog_object(1)).await.unwrap();
                // The catalog's FETCH is written, and not answered: the
                // catalog came on the subscription first.
                let (read, _fetch) = tokio::join!(
                    catalog.on(&mut session, &mut events, settled),
                    relay.accept_bi()
                );
                became.push(written(&read.unwrap()));
            } else {
                let read = catalog.on(&mut session, &mut events, settled).await;
                became.push(written(&read.unwrap()));
            }
        }
        (became, catalog.is_there())
    }

    #[tokio::test]
    async fn the_catalog_is_asked_for_until_it_comes_and_again_when_it_ends() {
        // (what the relay does, in order; what became of the broadcast after
        //  each)
        let cases: [(&[Relay], &[&str]); 5] = [
            (&[Relay::Gives], &["began 13"]),
            (
                &[
                    Relay::Refuses(request_error_code::DOES_NOT_EXIST),
                    Relay::Gives,
                ],
                &["waiting refused error_code=0x10 100", "began 13"],
            ),
            // The shortest pause doubles from one refusal to the next
            // (`AskAgain` of `xmoq-again`).
            (
                &[
                    Relay::Refuses(request_error_code::TIMEOUT),
                    Relay::Refuses(request_error_code::DOES_NOT_EXIST),
                ],
                &[
                    "waiting refused error_code=0x2 100",
                    "waiting refused error_code=0x10 200",
                ],
            ),
            (
                &[Relay::Gives, Relay::Ends],
                &["began 13", "ended PUBLISH_DONE status_code=0x2"],
            ),
            // The broadcast ends with its subscription, whatever became of
            // the catalog's FETCH.
            (
                &[Relay::Gives, Relay::BreaksTheFetch, Relay::Ends],
                &["began 13", "nothing", "ended PUBLISH_DONE status_code=0x2"],
            ),
        ];
        for (steps, expected) in cases {
            let (became, there) = followed(steps).await;
            assert_eq!(became, expected, "{steps:?}");
            assert_eq!(
                there,
                expected
                    .last()
                    .is_some_and(|last| last.starts_with("began")),
                "{steps:?}"
            );
        }
    }

    /// How `recv` ends: "fin", the error it ends with, or "open" after a
    /// second.
    async fn end_of(recv: &mut impl RecvStream) -> String {
        let reading = async {
            let mut buffer = [0; 256];
            loop {
                match recv.read(&mut buffer).await {
                    Ok(Some(_)) => {}
                    Ok(None) => return "fin".to_string(),
                    Err(error) => return format!("{error:?}"),
                }
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), reading)
            .await
            .unwrap_or_else(|_| "open".to_string())
    }

    /// What `send` was stopped with, or "open" after a second.
    async fn stop_of(send: &mut impl SendStream) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(1), send.stopped())
            .await
            .map_or_else(|_| "open".to_string(), |error| format!("{error:?}"))
    }

    /// A catalog that cannot be read: its FETCH is given up (§3.3.2, §5.2),
    /// and its subscription cut, so that the catalog can be asked for anew.
    #[tokio::test]
    async fn a_catalog_that_cannot_be_read_gives_its_fetch_up_and_is_asked_for_again() {
        let (ours, relay) = memory::pair();
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        let (queue, received) = mpsc::channel(8);
        let mut events = Events::new(received);
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec(), b"demo".to_vec()]).unwrap();
        let mut catalog = FollowedCatalog::new(
            namespace.clone(),
            None,
            Asking {
                connection: Arc::new(ours),
                stops: StreamStops::default(),
                token: None,
                answer_wait_ms: 10_000,
                open_wait_ms: 10_000,
            },
        );
        let ask_now = catalog.next().await;
        catalog
            .on(&mut session, &mut events, ask_now)
            .await
            .unwrap();
        let subscribe_ok = ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 1,
            parameters: vec![],
            track_properties: vec![],
        });
        let (settled, _subscription) = tokio::join!(catalog.next(), answer(&relay, subscribe_ok));
        let UniEvent::Object {
            stream,
            header,
            mut object,
        } = catalog_object(1)
        else {
            unreachable!("an Object");
        };
        object.payload = b"[]".to_vec();
        queue
            .send(UniEvent::Object {
                stream,
                header,
                object,
            })
            .await
            .unwrap();
        let (read, fetch) = tokio::join!(
            catalog.on(&mut session, &mut events, settled),
            relay.accept_bi()
        );
        let (mut fetch_send, mut fetch_recv) = fetch.unwrap();
        assert_eq!(
            written(&read.unwrap()),
            "waiting the catalog: the catalog is not a JSON object 1000"
        );
        assert!(!catalog.is_there());
        assert_eq!(end_of(&mut fetch_recv).await, "Reset(Some(1))");
        assert_eq!(stop_of(&mut fetch_send).await, "Stopped(Some(1))");
        let track =
            xmoq_moqt::name::FullTrackName::new(namespace, CATALOG.as_bytes().to_vec()).unwrap();
        assert!(session.subscribe(track, vec![]).is_ok());
    }
}
