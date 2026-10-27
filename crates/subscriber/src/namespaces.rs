//! A `SUBSCRIBE_NAMESPACE` kept on a session: when it is asked for again
//! after it ended or was refused, by the rule of `xmoq-again`, and what each
//! thing on its stream is to do ([`KeptNamespace::on_stream`]). The gatherer
//! of the chat, the transcriber and the agent keep theirs so; each sends it
//! itself, and the agent reads its stream by `on_stream`.

use std::time::{Duration, Instant};

use xmoq_again::AskAgain;
use xmoq_again::next_session::{Growing, Pause};
use xmoq_moqt::message::GoAway;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt::{Error, RequestFault};
use xmoq_moqt_io::{RequestReadError, RequestStreamEvent};

/// What a thing on the stream of a [`KeptNamespace`] is to do.
#[derive(Debug, PartialEq, Eq)]
pub enum NamespaceOutcome {
    /// Its `REQUEST_OK`: the namespaces follow.
    Subscribed,
    /// NAMESPACE: a namespace under the prefix, whole.
    Namespace(TrackNamespace),
    /// `NAMESPACE_DONE`: a namespace withdrawn, whole.
    Done(TrackNamespace),
    /// The request is over, `how`, and is asked for again in `pause_ms`;
    /// the caller cancels its stream. The namespaces told of on it are as
    /// though each got `NAMESPACE_DONE` (§10.18 for a FIN or a reset).
    Ended {
        /// What ended it.
        how: String,
        /// The pause before it is asked for again, in milliseconds.
        pause_ms: u64,
    },
    /// The request was refused, `how`, and is asked for again in
    /// `pause_ms`; the caller cancels its stream. No namespace was told of
    /// on it.
    Refused {
        /// The refusal.
        how: String,
        /// The pause before it is asked for again, in milliseconds.
        pause_ms: u64,
    },
    /// A GOAWAY on its stream: the caller moves the session.
    Moved(GoAway),
    /// Anything else the session told of; the request stays as it was.
    Other(RequestEvent),
}

/// One `SUBSCRIBE_NAMESPACE` for `prefix`, and when it is asked for again.
#[derive(Debug, Clone)]
pub struct KeptNamespace {
    prefix: TrackNamespace,
    again_at: Option<Instant>,
    asked_at: Instant,
    /// The pause after it ended, which grows while it ends again and again
    /// for a role whose [`Pause`] grows.
    ends: Growing,
    /// The pause after it was refused, by the Retry Interval.
    refusals: AskAgain,
}

impl KeptNamespace {
    /// The request for `prefix`, to be asked for at once; after an end it
    /// waits by `pause`, the role's: `Pause::SERVING` for the gatherer and
    /// the transcriber, `Pause::VIEWER` for the agent.
    ///
    /// WHY by the role after an end, and by the Retry Interval after a
    /// refusal: a relay that ends the request every time (one that tells
    /// more namespaces than the session keeps) is asked no more often than
    /// the role's longest pause, and one that refuses says when to come back
    /// (§10.6.2), as for a track (`fixtures/ask-again/same-session.tsv`).
    #[must_use]
    pub fn new(prefix: TrackNamespace, pause: Pause) -> Self {
        Self {
            prefix,
            again_at: Some(Instant::now()),
            asked_at: Instant::now(),
            ends: Growing::new(pause),
            refusals: AskAgain::default(),
        }
    }

    /// The prefix it asks for.
    #[must_use]
    pub fn prefix(&self) -> &TrackNamespace {
        &self.prefix
    }

    /// When it is to be asked for (again); `None` while it is open.
    #[must_use]
    pub fn again_at(&self) -> Option<Instant> {
        self.again_at
    }

    /// Whether its time to be asked for has come at `now`.
    #[must_use]
    pub fn due(&self, now: Instant) -> bool {
        self.again_at.is_some_and(|at| at <= now)
    }

    /// It was asked for at `now`.
    pub fn asked(&mut self, now: Instant) {
        self.again_at = None;
        self.asked_at = now;
    }

    /// It was answered with `REQUEST_OK`: the shortest pause after a refusal
    /// begins anew.
    pub fn subscribed(&mut self) {
        self.refusals.subscribed();
    }

    /// It ended at `now`, or could not be sent: the pause before it is asked
    /// for again, in milliseconds, counted from when it was asked.
    pub fn ended(&mut self, now: Instant) -> u64 {
        let lasted = u64::try_from(now.saturating_duration_since(self.asked_at).as_millis())
            .unwrap_or(u64::MAX);
        let pause_ms = self.ends.after(lasted);
        self.again_at = Some(now + Duration::from_millis(pause_ms));
        pause_ms
    }

    /// It was refused at `now`, with the wait its Retry Interval names
    /// (`RequestError::retry_wait_ms`; `None` for none): the pause before it
    /// is asked for again, in milliseconds.
    pub fn refused(&mut self, named_ms: Option<u64>, now: Instant) -> u64 {
        let pause_ms = self.refusals.after_refusal(named_ms);
        self.again_at = Some(now + Duration::from_millis(pause_ms));
        pause_ms
    }

    /// Hands what came at `now` on the stream of this request, `request`
    /// of `session`, to the session, and says what the caller does.
    ///
    /// WHY one NAMESPACE more than the session keeps asks again, where a
    /// message that breaks the protocol closes the session: the limit is
    /// this endpoint's, and the relay was within the draft; a fresh request
    /// is told the namespaces there are then.
    ///
    /// # Errors
    ///
    /// A message that breaks the protocol, on the stream or as what the
    /// session reads of it: the session is to be closed with the code of
    /// the error (`Error::termination_code`, `PROTOCOL_VIOLATION` for
    /// `NAMESPACE_DONE` before its NAMESPACE, §10.18). The request is left
    /// as it was.
    pub fn on_stream(
        &mut self,
        session: &mut Session,
        request: RequestId,
        event: Result<RequestStreamEvent, RequestReadError>,
        now: Instant,
    ) -> Result<NamespaceOutcome, Error> {
        let how = match event {
            Ok(RequestStreamEvent::Message(message)) => {
                match session.on_request_message(request, message) {
                    Ok(RequestEvent::NamespacesSubscribed) => {
                        self.subscribed();
                        return Ok(NamespaceOutcome::Subscribed);
                    }
                    Ok(RequestEvent::Namespace(namespace)) => {
                        return Ok(NamespaceOutcome::Namespace(namespace));
                    }
                    Ok(RequestEvent::NamespaceDone(namespace)) => {
                        return Ok(NamespaceOutcome::Done(namespace));
                    }
                    Ok(RequestEvent::Rejected(error)) => {
                        let pause_ms = self.refused(error.retry_wait_ms(), now);
                        return Ok(NamespaceOutcome::Refused {
                            how: format!("refused with {:#x}", error.error_code),
                            pause_ms,
                        });
                    }
                    Ok(RequestEvent::GoAway(go_away)) => {
                        return Ok(NamespaceOutcome::Moved(go_away));
                    }
                    Ok(RequestEvent::TooManyNamespaces { limit }) => {
                        format!("more namespaces than the {limit} a request keeps")
                    }
                    Ok(other) => return Ok(NamespaceOutcome::Other(other)),
                    Err(RequestFault::Peer(error)) => return Err(error),
                    // The request is of another session; this one is not at
                    // fault.
                    Err(RequestFault::Usage(error)) => format!("{error}"),
                }
            }
            Ok(RequestStreamEvent::Closed(closed)) => format!("the relay closed it ({closed:?})"),
            Err(RequestReadError::Peer(error)) => return Err(error),
            Err(error @ RequestReadError::Lost) => format!("{error}"),
        };
        session.cancel(request);
        let pause_ms = self.ended(now);
        Ok(NamespaceOutcome::Ended { how, pause_ms })
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::TerminationCode;
    use xmoq_moqt::message::{ControlMessage, ReasonPhrase, RequestError, RequestOk};
    use xmoq_moqt::session::{MAX_NAMESPACES_OF_A_SUBSCRIPTION, RequestState, Role, Transport};
    use xmoq_moqt_io::RequestClose;

    use super::*;

    /// The rows of `fixtures/own-requests/stream-ends.tsv` for a
    /// `SUBSCRIBE_NAMESPACE`, which `@xmoq/moqt` reads as well. In Rust the
    /// session does not decide what the end of the stream of a request of
    /// this end does; for a `SUBSCRIBE_NAMESPACE` a [`KeptNamespace`] does.
    /// §6.1: "A `SUBSCRIBE_NAMESPACE` or `SUBSCRIBE_TRACKS` can be cancelled
    /// by closing the stream with either a FIN or `RESET_STREAM`", so either
    /// ends it, after its `REQUEST_OK` as before.
    #[test]
    fn the_end_of_the_stream_of_a_subscribe_namespace_does_what_the_shared_table_says() {
        let table = include_str!("../../../fixtures/own-requests/stream-ends.tsv");
        let mut read = 0;
        for row in table.lines() {
            let [name, request, state, event, expected] = row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("a row of five columns: {row}");
            };
            if request != "subscribe-namespace" {
                continue;
            }
            read += 1;
            let prefix = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let (request, _) = session.subscribe_namespace(prefix.clone(), vec![]).unwrap();
            let mut kept = KeptNamespace::new(prefix, Pause::VIEWER);
            let now = Instant::now();
            kept.asked(now);
            if state == "established" {
                let ok = Ok(RequestStreamEvent::Message(ControlMessage::RequestOk(
                    RequestOk::default(),
                )));
                assert!(matches!(
                    kept.on_stream(&mut session, request, ok, now),
                    Ok(NamespaceOutcome::Subscribed)
                ));
            }
            let how = match event {
                "fin" => RequestClose::Fin,
                "reset" => RequestClose::Reset(Some(0x1)),
                other => panic!("{name}: {other}"),
            };
            let got = match kept.on_stream(
                &mut session,
                request,
                Ok(RequestStreamEvent::Closed(how)),
                now,
            ) {
                Ok(NamespaceOutcome::Ended { .. }) => "ends",
                Ok(_) => "nothing",
                Err(error) => panic!("{name}: {error:?}"),
            };
            assert_eq!(got, expected, "{name}");
        }
        assert_eq!(read, 2);
    }

    /// What happens to the request, and the pause after it.
    enum Step {
        /// It ended after it lasted this many milliseconds.
        Ended(u64),
        /// It was refused with this wait named.
        Refused(Option<u64>),
        /// It was answered.
        Subscribed,
    }

    #[test]
    fn it_is_asked_again_by_the_roles_pause_after_an_end_and_by_the_retry_interval_after_a_refusal()
    {
        let prefix = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        // (case, the role's pause, what happens, the pause after each)
        let cases: [(&str, Pause, Vec<Step>, &[u64]); 5] = [
            (
                "the gatherer's grows while it ends at once",
                Pause::SERVING,
                vec![Step::Ended(0), Step::Ended(0), Step::Ended(0)],
                &[1_000, 2_000, 4_000],
            ),
            (
                "and begins again after it lasted a minute",
                Pause::SERVING,
                vec![Step::Ended(0), Step::Ended(0), Step::Ended(60_000)],
                &[1_000, 2_000, 1_000],
            ),
            (
                "the agent's does not grow",
                Pause::VIEWER,
                vec![Step::Ended(0), Step::Ended(0)],
                &[1_000, 1_000],
            ),
            (
                "a refusal waits its Retry Interval, or the pause of one without",
                Pause::VIEWER,
                vec![Step::Refused(Some(3_000)), Step::Refused(None)],
                &[3_000, 1_000],
            ),
            (
                "refusals that say at once double from 100 ms, and an answer begins them anew",
                Pause::SERVING,
                vec![
                    Step::Refused(Some(0)),
                    Step::Refused(Some(0)),
                    Step::Subscribed,
                    Step::Refused(Some(0)),
                ],
                &[100, 200, 100],
            ),
        ];
        for (case, pause, steps, expected) in cases {
            let mut kept = KeptNamespace::new(prefix.clone(), pause);
            let mut now = Instant::now();
            assert!(kept.due(now), "{case}: asked for at once");
            let mut pauses = Vec::new();
            for step in steps {
                kept.asked(now);
                assert_eq!(kept.again_at(), None, "{case}: open");
                match step {
                    Step::Ended(lasted) => {
                        now += Duration::from_millis(lasted);
                        pauses.push(kept.ended(now));
                    }
                    Step::Refused(named) => pauses.push(kept.refused(named, now)),
                    Step::Subscribed => {
                        kept.subscribed();
                        continue;
                    }
                }
                let pause = *pauses.last().unwrap();
                assert!(!kept.due(now), "{case}: not before its pause");
                now += Duration::from_millis(pause);
                assert!(kept.due(now), "{case}: after it");
            }
            assert_eq!(pauses, expected, "{case}");
        }
    }

    fn namespace(fields: &[&str]) -> TrackNamespace {
        TrackNamespace::new(
            fields
                .iter()
                .map(|field| field.as_bytes().to_vec())
                .collect(),
        )
        .unwrap()
    }

    fn request_ok() -> ControlMessage {
        ControlMessage::RequestOk(RequestOk {
            parameters: vec![],
            track_properties: vec![],
        })
    }

    fn named(suffix: &str) -> ControlMessage {
        ControlMessage::Namespace(namespace(&[suffix]))
    }

    /// What the caller is told.
    #[derive(Debug)]
    enum Expected {
        Subscribed,
        Namespace(&'static [&'static str]),
        Done(&'static [&'static str]),
        /// Asked again after an end: how, and the pause.
        Ended(&'static str, u64),
        /// Asked again after a refusal: how, and the pause.
        Refused(&'static str, u64),
        /// Moved, with the GOAWAY's Timeout.
        Moved(u64),
        /// The session closed, with this code.
        Close(u32),
    }

    /// (case, what the session was told on the stream before, what came,
    /// what the caller is told)
    type Case = (
        &'static str,
        Vec<ControlMessage>,
        Result<RequestStreamEvent, RequestReadError>,
        Expected,
    );

    fn cases() -> [Case; 13] {
        let mut too_many = vec![request_ok()];
        too_many
            .extend((0..MAX_NAMESPACES_OF_A_SUBSCRIPTION).map(|index| named(&index.to_string())));
        [
            (
                "REQUEST_OK",
                vec![],
                Ok(RequestStreamEvent::Message(request_ok())),
                Expected::Subscribed,
            ),
            (
                "a NAMESPACE",
                vec![request_ok()],
                Ok(RequestStreamEvent::Message(named("alice"))),
                Expected::Namespace(&["xmoq", "alice"]),
            ),
            (
                "its NAMESPACE_DONE",
                vec![request_ok(), named("alice")],
                Ok(RequestStreamEvent::Message(ControlMessage::NamespaceDone(
                    namespace(&["alice"]),
                ))),
                Expected::Done(&["xmoq", "alice"]),
            ),
            (
                "a REQUEST_ERROR waits its Retry Interval",
                vec![],
                Ok(RequestStreamEvent::Message(ControlMessage::RequestError(
                    RequestError {
                        error_code: 0x1,
                        retry_interval: 3_001,
                        error_reason: ReasonPhrase::default(),
                        redirect: None,
                    },
                ))),
                Expected::Refused("refused with 0x1", 3_000),
            ),
            (
                "a GOAWAY",
                vec![request_ok()],
                Ok(RequestStreamEvent::Message(ControlMessage::GoAway(
                    GoAway {
                        new_session_uri: b"https://elsewhere/".to_vec(),
                        timeout_ms: 7,
                        request_id: None,
                    },
                ))),
                Expected::Moved(7),
            ),
            // The limit is this endpoint's: asked again, not closed.
            (
                "one NAMESPACE more than the session keeps",
                too_many,
                Ok(RequestStreamEvent::Message(named("one more"))),
                Expected::Ended("more namespaces than the 4096 a request keeps", 1_000),
            ),
            (
                "a FIN",
                vec![request_ok()],
                Ok(RequestStreamEvent::Closed(RequestClose::Fin)),
                Expected::Ended("the relay closed it (Fin)", 1_000),
            ),
            (
                "a reset",
                vec![request_ok()],
                Ok(RequestStreamEvent::Closed(RequestClose::Reset(Some(5)))),
                Expected::Ended("the relay closed it (Reset(Some(5)))", 1_000),
            ),
            (
                "the connection lost",
                vec![request_ok()],
                Err(RequestReadError::Lost),
                Expected::Ended("connection lost", 1_000),
            ),
            (
                "NAMESPACE_DONE before its NAMESPACE",
                vec![request_ok()],
                Ok(RequestStreamEvent::Message(ControlMessage::NamespaceDone(
                    namespace(&["alice"]),
                ))),
                Expected::Close(0x3),
            ),
            (
                "a NAMESPACE before REQUEST_OK",
                vec![],
                Ok(RequestStreamEvent::Message(named("alice"))),
                Expected::Close(0x3),
            ),
            (
                "a second REQUEST_OK",
                vec![request_ok()],
                Ok(RequestStreamEvent::Message(request_ok())),
                Expected::Close(0x3),
            ),
            (
                "bytes on the stream that break the protocol",
                vec![request_ok()],
                Err(RequestReadError::Peer(Error::UnknownMessageType(0x99))),
                Expected::Close(0x3),
            ),
        ]
    }

    #[test]
    fn what_comes_on_the_stream_tells_a_namespace_asks_again_or_closes_the_session() {
        for (case, before, event, expected) in cases() {
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let mut kept = KeptNamespace::new(namespace(&["xmoq"]), Pause::VIEWER);
            let (request, _) = session
                .subscribe_namespace(kept.prefix().clone(), vec![])
                .unwrap();
            let now = Instant::now();
            kept.asked(now);
            for message in before {
                session.on_request_message(request, message).unwrap();
            }
            let outcome = kept.on_stream(&mut session, request, event, now);
            let again_ms = match (outcome, expected) {
                (Ok(NamespaceOutcome::Subscribed), Expected::Subscribed) => None,
                (Ok(NamespaceOutcome::Namespace(got)), Expected::Namespace(fields))
                | (Ok(NamespaceOutcome::Done(got)), Expected::Done(fields)) => {
                    assert_eq!(got, namespace(fields), "{case}");
                    None
                }
                (Ok(NamespaceOutcome::Ended { how, pause_ms }), Expected::Ended(expected, ms))
                | (
                    Ok(NamespaceOutcome::Refused { how, pause_ms }),
                    Expected::Refused(expected, ms),
                ) => {
                    assert_eq!((how.as_str(), pause_ms), (expected, ms), "{case}");
                    assert_eq!(
                        session.request_state(request),
                        Some(RequestState::Terminated),
                        "{case}: the session holds it no more"
                    );
                    Some(ms)
                }
                (Ok(NamespaceOutcome::Moved(go_away)), Expected::Moved(timeout_ms)) => {
                    assert_eq!(go_away.timeout_ms, timeout_ms, "{case}");
                    None
                }
                (Err(error), Expected::Close(code)) => {
                    assert_eq!(
                        error.termination_code().map(TerminationCode::code),
                        Some(code),
                        "{case}"
                    );
                    None
                }
                (other, expected) => panic!("{case}: {other:?}, not {expected:?}"),
            };
            assert_eq!(
                kept.again_at(),
                again_ms.map(|ms| now + Duration::from_millis(ms)),
                "{case}: when it is asked again"
            );
        }
    }
}
