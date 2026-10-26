//! One track kept on one session: subscribed to, and asked for again by the
//! rule of `xmoq-again` when its subscription ends or a SUBSCRIBE for it is
//! refused. The sound of `xmoq-watch`, the video of `xmoq-agent`, and each
//! track a [`crate::follow::Follower`] follows are kept so.
//!
//! What becomes of each thing that can come, as the pages have it, so that
//! a viewer of the terminal, an agent and a page do the same:
//! - `PUBLISH_DONE`, a stream that closes or breaks, and a GOAWAY on the
//!   request's stream (§10.4) end the subscription, which is asked for again
//!   after the pause;
//! - a refusal is asked again after the pause, or its Retry Interval;
//! - a SUBSCRIBE that could not be sent, or was not answered, is asked again
//!   as a refusal without a Retry Interval where the track is so wanted
//!   ([`Wanted::again_after_failure`]), and else gives the track up on this
//!   session, the next session asking for it again;
//! - a message that breaks the protocol (`RequestFault::Peer`), on the
//!   subscription's stream or as the answer to its SUBSCRIBE, is the caller's
//!   error, and the session is closed for it.
//!
//! FIXME: the rule is read against the pages' only by eye; a table of what
//! comes and what becomes of it in `fixtures/ask-again/`, read by a test
//! here and by those of `keepTrackOnSession` (`@xmoq/media`), `@xmoq/chat`
//! and `@xmoq/transcript`, would hold the two together. WHY NOT yet: the
//! decision takes the session of `xmoq-moqt`, which `xmoq-viewer` does not
//! depend on, and the three on the TypeScript side have no shape in common
//! to read it with.
//!
//! WHY a GOAWAY asks again on this session, also where it names a URI
//! (§10.4 has the request sent there): the rest of the session stays here,
//! and the caller moves the whole of it when the relay's GOAWAY comes on the
//! control stream. WHY NOT go on reading a subscription after a GOAWAY on its
//! stream: §10.4 "SHOULD re-issue that specific request ... and close the old
//! request stream".

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use xmoq_again::AskAgain;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{Connection, RequestReadError, RequestStreamEvent, StreamStops};

use crate::session::{Asked, NotSubscribed, Requests, Sent, Subscription, send_subscribe, settle};

/// A SUBSCRIBE on its way, and what came of it.
type Answer<C> = std::pin::Pin<
    Box<dyn std::future::Future<Output = (Asked, Result<Sent<C>>)> + std::marker::Send>,
>;

/// What the requests are sent with.
pub struct Asking<C: Connection = WtConnection> {
    /// Where their streams are opened.
    pub connection: Arc<C>,
    /// Where the limit on the track's Objects is given, and a FETCH's
    /// stream is taken.
    pub stops: StreamStops,
    /// The AUTHORIZATION TOKEN of the requests.
    pub token: Option<Vec<u8>>,
    /// `--answer-wait-ms`.
    pub answer_wait_ms: u64,
    /// `--open-wait-ms`.
    pub open_wait_ms: u64,
}

/// The track kept.
pub struct Wanted {
    /// Its namespace.
    pub namespace: TrackNamespace,
    /// Its name.
    pub name: String,
    /// The `RENDEZVOUS_TIMEOUT` of its SUBSCRIBE, where its publisher may
    /// come later.
    pub rendezvous_ms: Option<u64>,
    /// The most an Object of the track may be, where it is held to less than
    /// the readers' limit of every subgroup stream.
    pub payload_most: Option<usize>,
    /// Whether a SUBSCRIBE that could not be sent, or was not answered, is
    /// asked again on the session, or gives the track up there.
    ///
    /// WHY both: the chat's `lines` and a transcript's `speech` are what an
    /// agent reads, for hours on one session, and `@xmoq/chat` and
    /// `@xmoq/transcript` ask for them again whatever kept them away. The
    /// sound beside a picture is given up, as `keepTrackOnSession` of
    /// `@xmoq/media` gives it up: the next session asks for it again.
    pub again_after_failure: bool,
}

/// What the kept track has to be told of, from the loop of the session.
pub enum Came<C: Connection = WtConnection> {
    /// What came on the stream of the subscription.
    Answer(Result<RequestStreamEvent, RequestReadError>),
    /// The time to ask for the track (again) has come.
    AskNow,
    /// What came of the SUBSCRIBE, boxed: it is the largest by far.
    Settled(Box<(Asked, Result<Sent<C>>)>),
}

/// What became of the track, for the caller to tell.
#[derive(Debug)]
pub enum Said {
    /// It is subscribed to (again), with this Track Alias
    /// ([`KeptTrack::subscription`]).
    Subscribed {
        /// The Track Alias of the new subscription.
        track_alias: u64,
    },
    /// Its subscription ended, `how`, and it is asked for again in
    /// `pause_ms`.
    Ended {
        /// How it ended, for a log.
        how: String,
        /// The pause before it is asked for again.
        pause_ms: u64,
    },
    /// Its SUBSCRIBE was refused, and it is asked for again in `pause_ms`.
    Refused {
        /// The Error Code of the `REQUEST_ERROR`.
        error_code: u64,
        /// The pause before it is asked for again.
        pause_ms: u64,
    },
    /// Its SUBSCRIBE could not be sent, or was not answered, for this, and
    /// it is asked for again in `pause_ms` ([`Wanted::again_after_failure`]).
    Failed {
        /// Why, for a log.
        why: String,
        /// The pause before it is asked for again.
        pause_ms: u64,
    },
    /// It is not asked for again on this session, for this.
    GivenUp(String),
    /// A message on its stream that leaves the subscription as it is.
    Other(RequestEvent),
}

/// What a thing on the subscription's stream does to it.
#[derive(Debug, PartialEq, Eq)]
enum Answered {
    /// It ends the subscription, `how`; the session holds it no more.
    Ended(String),
    /// It leaves the subscription as it is.
    Other(RequestEvent),
}

/// Hands what came on the stream of the subscription `request` to the
/// session, and says what it does to the subscription.
fn answered(
    session: &mut Session,
    request: RequestId,
    event: Result<RequestStreamEvent, RequestReadError>,
) -> Result<Answered, RequestFault> {
    let how = match event {
        Ok(RequestStreamEvent::Message(message)) => {
            match session.on_request_message(request, message) {
                Ok(RequestEvent::SubscriptionDone(done)) => {
                    format!("PUBLISH_DONE status_code={:#x}", done.status_code)
                }
                Ok(RequestEvent::GoAway(go_away)) => {
                    session.cancel(request);
                    format!("REQUEST_GOAWAY timeout_ms={}", go_away.timeout_ms)
                }
                Ok(other) => return Ok(Answered::Other(other)),
                Err(RequestFault::Peer(error)) => return Err(RequestFault::Peer(error)),
                // The session has the subscription still.
                Err(RequestFault::Usage(error)) => {
                    session.cancel(request);
                    format!("{error}")
                }
            }
        }
        // Without PUBLISH_DONE the session still has the subscription,
        // which would stand in the way of the next SUBSCRIBE for the track.
        Ok(RequestStreamEvent::Closed(closed)) => {
            session.cancel(request);
            format!("closed {closed:?}")
        }
        Err(error) => {
            session.cancel(request);
            format!("{error}")
        }
    };
    Ok(Answered::Ended(how))
}

/// One track kept on one session.
pub struct KeptTrack<C: Connection = WtConnection> {
    wanted: Wanted,
    asking_with: Asking<C>,
    /// The subscription, while there is one.
    subscription: Option<Subscription<C>>,
    /// The SUBSCRIBE on its way, with the session's handle of it.
    asking: Option<(RequestId, Answer<C>)>,
    ask_at: Option<tokio::time::Instant>,
    again: AskAgain,
}

/// Waits for `item` where there is one, and for ever where there is none.
pub(crate) async fn on<T>(item: Option<impl std::future::Future<Output = T>>) -> T {
    match item {
        Some(future) => future.await,
        None => std::future::pending().await,
    }
}

impl<C: Connection> KeptTrack<C> {
    /// A track that is asked for at once, from the loop ([`Came::AskNow`]).
    #[must_use]
    pub fn new(wanted: Wanted, asking_with: Asking<C>) -> Self {
        Self {
            wanted,
            asking_with,
            subscription: None,
            asking: None,
            ask_at: Some(tokio::time::Instant::now()),
            again: AskAgain::default(),
        }
    }

    /// A track whose first subscription the caller made, where a session
    /// without it is of no use. That one is told of by no
    /// [`Said::Subscribed`]: the caller has it.
    #[must_use]
    pub fn subscribed(
        wanted: Wanted,
        asking_with: Asking<C>,
        subscription: Subscription<C>,
    ) -> Self {
        let mut kept = Self {
            wanted,
            asking_with,
            subscription: None,
            asking: None,
            ask_at: None,
            again: AskAgain::default(),
        };
        kept.take(subscription);
        kept
    }

    /// The subscription, while there is one.
    #[must_use]
    pub fn subscription(&self) -> Option<&Subscription<C>> {
        self.subscription.as_ref()
    }

    /// What the requests are sent with, for the caller's own requests of
    /// the track (a FETCH).
    #[must_use]
    pub fn asking_with(&self) -> &Asking<C> {
        &self.asking_with
    }

    /// The Track Alias of the track, while it is subscribed to.
    #[must_use]
    pub fn track_alias(&self) -> Option<u64> {
        self.subscription
            .as_ref()
            .map(|subscription| subscription.track_alias)
    }

    /// What comes next.
    pub async fn next(&mut self) -> Came<C> {
        let Self {
            subscription,
            ask_at,
            asking,
            ..
        } = self;
        tokio::select! {
            event = on(subscription.as_mut().map(|subscription| subscription.reader.next_event())) => Came::Answer(event),
            () = on(ask_at.map(tokio::time::sleep_until)) => Came::AskNow,
            settled = on(asking.as_mut().map(|(_, answer)| answer)) => Came::Settled(Box::new(settled)),
        }
    }

    /// What came, and what became of the track of it, where anything did.
    ///
    /// # Errors
    ///
    /// A message on the subscription's stream, or an answer to its
    /// SUBSCRIBE, that breaks the protocol: the session is to be closed with
    /// its code.
    pub fn on(
        &mut self,
        session: &mut Session,
        came: Came<C>,
    ) -> Result<Option<Said>, RequestFault> {
        match came {
            Came::Answer(event) => self.on_answer(session, event),
            Came::AskNow => {
                self.ask_at = None;
                Ok(self.ask(session))
            }
            Came::Settled(settled) => {
                self.asking = None;
                let (asked, sent) = *settled;
                self.settled(session, &asked, sent).map(Some)
            }
        }
    }

    /// Ends the subscription from this side, and asks for the track again
    /// after the pause; `None` where there is none.
    pub fn cut(&mut self, session: &mut Session) -> Option<Said> {
        let subscription = self.subscription.as_ref()?;
        session.cancel(subscription.request);
        Some(self.ended("cut".to_string(), stream_reset_code::CANCELLED))
    }

    /// The session ended the SUBSCRIBE `request` for a malformed track
    /// (`Session::cancel_malformed`): where it is this track's, its stream is
    /// reset with `MALFORMED_TRACK` (§2.4.2), and the track is asked for
    /// again after the pause, as after any end; `None` where it is another's.
    ///
    /// WHY asked again: what broke the object model may be one Group of a
    /// publisher that goes on, and the pages ask for the track again after
    /// it ends whatever ended it.
    ///
    /// FIXME: no interop run has a viewer find a track malformed; the relays
    /// find it first and end the subscription with `PUBLISH_DONE`. One would
    /// have `xmoq-quiet-peer` stand as the relay, serve a catalog and its
    /// tracks, and send an Object after the End of a Group, and wait for
    /// `TRACK_MALFORMED` and the track asked for again in the logs of
    /// `xmoq-watch` and `xmoq-agent`.
    pub fn malformed(&mut self, request: RequestId) -> Option<Said> {
        if self.subscription.as_ref()?.request != request {
            return None;
        }
        Some(self.ended(
            "MALFORMED_TRACK".to_string(),
            stream_reset_code::MALFORMED_TRACK,
        ))
    }

    /// Gives the track up: its subscription, or the SUBSCRIBE that waits for
    /// its answer. Both are ended in `session` too, which would otherwise
    /// hold them and refuse the next SUBSCRIBE for the track there
    /// (`Session::subscribe`: one subscription a track).
    pub fn cancel(mut self, session: &mut Session) {
        if let Some((request, _)) = self.asking.take() {
            session.cancel(request);
        }
        if let Some(subscription) = self.let_go() {
            session.cancel(subscription.request);
            subscription.cancel();
        }
    }

    /// Sends the SUBSCRIBE.
    fn ask(&mut self, session: &mut Session) -> Option<Said> {
        let mut requests = Requests {
            connection: &*self.asking_with.connection,
            session,
            namespace: &self.wanted.namespace,
            token: self.asking_with.token.as_deref(),
            answer_wait_ms: self.asking_with.answer_wait_ms,
            open_wait_ms: self.asking_with.open_wait_ms,
            rendezvous_ms: self.wanted.rendezvous_ms,
        };
        match requests.ask(&self.wanted.name, None) {
            Ok(asked) => {
                let connection = Arc::clone(&self.asking_with.connection);
                let open_wait_ms = self.asking_with.open_wait_ms;
                let request = asked.request();
                let answer: Answer<C> = Box::pin(async move {
                    let sent = send_subscribe(&*connection, open_wait_ms, &asked).await;
                    (asked, sent)
                });
                self.asking = Some((request, answer));
                None
            }
            Err(error) => Some(self.not_sent(format!("{error:#}"))),
        }
    }

    /// A SUBSCRIBE that could not be sent, or was not answered, for `why`.
    fn not_sent(&mut self, why: String) -> Said {
        if !self.wanted.again_after_failure {
            return Said::GivenUp(why);
        }
        // As the pages that ask again: a failure is paused for as a refusal
        // without a Retry Interval is (`afterRefusal` of `@xmoq/moqt`).
        let pause_ms = self.again.after_refusal(None);
        self.ask_in(pause_ms);
        Said::Failed { why, pause_ms }
    }

    /// What came of the SUBSCRIBE.
    fn settled(
        &mut self,
        session: &mut Session,
        asked: &Asked,
        sent: Result<Sent<C>>,
    ) -> Result<Said, RequestFault> {
        Ok(match settle(session, asked, sent) {
            Ok(subscription) => {
                self.again.subscribed();
                let track_alias = subscription.track_alias;
                self.take(subscription);
                Said::Subscribed { track_alias }
            }
            Err(NotSubscribed::Refused(error)) => {
                let pause_ms = self.again.after_refusal(error.retry_wait_ms());
                self.ask_in(pause_ms);
                Said::Refused {
                    error_code: error.error_code,
                    pause_ms,
                }
            }
            Err(NotSubscribed::Failed(error)) => self.not_sent(format!("{error:#}")),
            Err(NotSubscribed::Peer(fault)) => return Err(fault),
        })
    }

    /// What came on the stream of the subscription.
    fn on_answer(
        &mut self,
        session: &mut Session,
        event: Result<RequestStreamEvent, RequestReadError>,
    ) -> Result<Option<Said>, RequestFault> {
        let Some(subscription) = self.subscription.as_ref() else {
            return Ok(None);
        };
        Ok(Some(
            match answered(session, subscription.request, event)? {
                Answered::Ended(how) => self.ended(how, stream_reset_code::CANCELLED),
                Answered::Other(other) => Said::Other(other),
            },
        ))
    }

    /// The subscription is over, `how`: asked for again after the pause.
    /// Its streams are reset and stopped with `code`.
    fn ended(&mut self, how: String, code: u32) -> Said {
        if let Some(subscription) = self.let_go() {
            subscription.cancel_with(code);
        }
        let pause_ms = self.again.after_end();
        self.ask_in(pause_ms);
        Said::Ended { how, pause_ms }
    }

    fn take(&mut self, subscription: Subscription<C>) {
        if let Some(most) = self.wanted.payload_most {
            self.asking_with
                .stops
                .limit_payload(subscription.track_alias, most);
        }
        self.subscription = Some(subscription);
    }

    fn let_go(&mut self) -> Option<Subscription<C>> {
        let subscription = self.subscription.take()?;
        if self.wanted.payload_most.is_some() {
            self.asking_with
                .stops
                .unlimit_payload(subscription.track_alias);
        }
        Some(subscription)
    }

    fn ask_in(&mut self, pause_ms: u64) {
        self.ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause_ms));
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::message::{ControlMessage, GoAway, PublishDone, ReasonPhrase, SubscribeOk};
    use xmoq_moqt::name::FullTrackName;
    use xmoq_moqt::session::{RequestState, Role, Transport};
    use xmoq_moqt_io::memory;
    use xmoq_moqt_io::{ReadError, RecvStream, RequestClose, Trace, send_message};

    use super::*;

    /// What the thing on the stream is to do.
    #[derive(Debug)]
    enum Expected {
        /// End the subscription so, the session holding it no more.
        Ended(&'static str),
        /// Close the session.
        Peer,
    }

    fn subscribe_ok() -> ControlMessage {
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 1,
            parameters: vec![],
            track_properties: vec![],
        })
    }

    #[test]
    fn what_comes_on_the_stream_ends_the_subscription_or_the_session() {
        let cases = [
            (
                "PUBLISH_DONE",
                Ok(RequestStreamEvent::Message(ControlMessage::PublishDone(
                    PublishDone {
                        status_code: 2,
                        stream_count: 1,
                        error_reason: ReasonPhrase::default(),
                    },
                ))),
                Expected::Ended("PUBLISH_DONE status_code=0x2"),
            ),
            // §10.4: asked for again, where it went on being read.
            (
                "a GOAWAY",
                Ok(RequestStreamEvent::Message(ControlMessage::GoAway(
                    GoAway {
                        new_session_uri: b"https://elsewhere/".to_vec(),
                        timeout_ms: 7,
                        request_id: None,
                    },
                ))),
                Expected::Ended("REQUEST_GOAWAY timeout_ms=7"),
            ),
            (
                "a FIN",
                Ok(RequestStreamEvent::Closed(RequestClose::Fin)),
                Expected::Ended("closed Fin"),
            ),
            // A second answer breaks the protocol, where it was let go of.
            (
                "a second SUBSCRIBE_OK",
                Ok(RequestStreamEvent::Message(subscribe_ok())),
                Expected::Peer,
            ),
        ];
        for (name, event, expected) in cases {
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let track = FullTrackName::new(
                TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
                b"audio".to_vec(),
            )
            .unwrap();
            let (request, _) = session.subscribe(track, vec![]).unwrap();
            session.on_request_message(request, subscribe_ok()).unwrap();
            match (answered(&mut session, request, event), expected) {
                (Ok(Answered::Ended(how)), Expected::Ended(expected)) => {
                    assert_eq!(how, expected, "{name}");
                    assert_eq!(
                        session.request_state(request),
                        Some(RequestState::Terminated),
                        "{name}"
                    );
                }
                (Err(RequestFault::Peer(_)), Expected::Peer) => {}
                (other, expected) => panic!("{name}: {other:?}, not {expected:?}"),
            }
        }
    }

    /// Reads `recv` until it ends: at its FIN, or with an error.
    async fn read_to_end(recv: &mut impl RecvStream) -> Result<(), ReadError> {
        let mut buffer = [0; 256];
        while recv.read(&mut buffer).await?.is_some() {}
        Ok(())
    }

    #[tokio::test]
    async fn a_track_given_up_can_be_subscribed_to_again_on_the_session() {
        // (the case; whether the relay answers the SUBSCRIBE before the
        // track is given up)
        let cases = [
            ("while its SUBSCRIBE waits for the answer", false),
            ("once it is subscribed to", true),
        ];
        for (name, answered) in cases {
            let (ours, relay) = memory::pair();
            // The session's, which outlives the track given up.
            let ours = Arc::new(ours);
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
            let mut kept = KeptTrack::new(
                Wanted {
                    namespace: namespace.clone(),
                    name: "video".to_string(),
                    rendezvous_ms: None,
                    payload_most: None,
                    again_after_failure: false,
                },
                Asking {
                    connection: Arc::clone(&ours),
                    stops: StreamStops::default(),
                    token: None,
                    answer_wait_ms: 10_000,
                    open_wait_ms: 10_000,
                },
            );
            let ask_now = kept.next().await;
            assert!(kept.on(&mut session, ask_now).unwrap().is_none(), "{name}");
            // The SUBSCRIBE is written, and waits for its answer.
            let (mut relay_send, mut relay_recv) = tokio::select! {
                _ = kept.next() => panic!("{name}: nothing is to come before the relay answers"),
                accepted = relay.accept_bi() => accepted.unwrap(),
            };
            if answered {
                let (answer, trace) = (subscribe_ok(), Trace::default());
                let answering = send_message(&mut relay_send, &answer, &trace, "relay");
                let (settled, written) = tokio::join!(kept.next(), answering);
                written.unwrap();
                assert!(
                    matches!(
                        kept.on(&mut session, settled),
                        Ok(Some(Said::Subscribed { track_alias: 1 }))
                    ),
                    "{name}"
                );
            }
            kept.cancel(&mut session);
            let track = FullTrackName::new(namespace, b"video".to_vec()).unwrap();
            assert!(session.subscribe(track, vec![]).is_ok(), "{name}");
            // §3.3.2: the relay's stream is ended abruptly, with CANCELLED.
            assert_eq!(
                read_to_end(&mut relay_recv).await,
                Err(ReadError::Reset(Some(stream_reset_code::CANCELLED))),
                "{name}"
            );
        }
    }
}
