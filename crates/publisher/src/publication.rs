//! The PUBLISH of one track of one Object a Group, from the PUBLISH to its
//! `PUBLISH_DONE` ([`Publication`]): what waits for its `PUBLISH_OK`, the
//! streams it opened, and when it is published again. Without input or
//! output: the client sends the messages. The count of the streams being
//! sent is shared with their tasks through a `tokio` watch, which
//! [`Ending::when_drained`] waits on.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::watch;

/// How long a track whose PUBLISH failed waits before it is published
/// again, on the next Object that comes for it. Provisional.
pub const PUBLISH_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// The Track Alias of the first track a client publishes; each next one
/// has the next (§11.1: one alias a track of the session).
pub const FIRST_TRACK_ALIAS: u64 = 1;

/// Where a PUBLISH of the client stands, for what the end of its stream
/// does to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishStage {
    /// Sent, without its answer.
    Pending,
    /// Answered with `PUBLISH_OK`.
    Established,
    /// Ended by the client with `PUBLISH_DONE` and a FIN; the request's task
    /// waits for the relay to end its direction (`crate::streams`).
    Done,
}

/// How the relay ended a direction of the stream of a PUBLISH of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStreamEnd {
    /// It finished its direction.
    Fin,
    /// It reset its direction.
    Reset,
    /// It sent `STOP_SENDING` for the client's direction.
    StopSending,
}

/// What the end of a direction of the stream of a PUBLISH does to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEndDoes {
    /// The PUBLISH is over: the client cancels it.
    Ends,
    /// Nothing: the PUBLISH goes on.
    Nothing,
    /// The PUBLISH was over already; this is the end of the relay's
    /// direction its task waited for after `PUBLISH_DONE`, and is reported.
    PeerEnd,
}

/// What `end` does to a PUBLISH of the client at `stage`; the gatherer of
/// the chat and the transcriber go by it, and
/// `fixtures/own-requests/stream-ends.tsv` has the rows. A reset ends a
/// PUBLISH (§3.3.2: "Implementations SHOULD cancel requests by abruptly
/// terminating any directions of a stream that are still open by resetting
/// or sending `STOP_SENDING`"). Before the answer, a FIN ends it too: §5.1,
/// "A subscriber MUST send exactly one `PUBLISH_OK` ... or `REQUEST_ERROR`
/// in response to a PUBLISH", and the direction that would carry it is
/// over.
///
/// WHY a `STOP_SENDING` before the answer ends nothing, where after
/// `PUBLISH_OK` it ends the subscription (§5.1: a subscriber ends one with
/// `STOP_SENDING`): §5.1 has no such transition from Pending, and §3.3.2
/// lets a receiver cancel; the text does not decide, and this is the
/// ruling of `fixtures/own-requests/stream-ends.tsv`, which `@xmoq/moqt` follows
/// too. The PUBLISH waits for the subscriber's reset or answer.
/// WHY NOT end it: the subscriber that stopped may still answer.
///
/// After `PUBLISH_DONE` the end of the relay's direction is what the task
/// waits for (§10.11), and a `STOP_SENDING` there finds the client's
/// direction finished.
#[must_use]
pub fn stream_end(stage: PublishStage, end: RequestStreamEnd) -> StreamEndDoes {
    match (stage, end) {
        (PublishStage::Done, RequestStreamEnd::Fin | RequestStreamEnd::Reset) => {
            StreamEndDoes::PeerEnd
        }
        (PublishStage::Done | PublishStage::Pending, RequestStreamEnd::StopSending)
        | (PublishStage::Established, RequestStreamEnd::Fin) => StreamEndDoes::Nothing,
        (PublishStage::Pending, RequestStreamEnd::Fin | RequestStreamEnd::Reset)
        | (PublishStage::Established, RequestStreamEnd::Reset | RequestStreamEnd::StopSending) => {
            StreamEndDoes::Ends
        }
    }
}

/// The PUBLISH of a track, and the Objects `T` that wait for it.
pub struct Publication<T> {
    /// The token of its PUBLISH, while there is one.
    publish: Option<u64>,
    /// The Track Alias of its latest PUBLISH; 0 before the first, which is
    /// no alias a client gives ([`FIRST_TRACK_ALIAS`]).
    track_alias: u64,
    /// Whether the PUBLISH has its `PUBLISH_OK`, whatever its Forward State.
    answered: bool,
    /// Whether the PUBLISH has its `PUBLISH_OK` with Forward State 1:
    /// Objects are sent on it.
    live: bool,
    /// The subgroup streams of its PUBLISH that were opened
    /// (`GroupToSend::opened` of [`crate::streams`]).
    opened: Arc<AtomicU64>,
    /// What waits for the `PUBLISH_OK`, the latest `most_waiting`.
    waiting: VecDeque<T>,
    most_waiting: usize,
    /// Not published again before this, after a PUBLISH that failed.
    not_before: Option<Instant>,
    /// The Objects whose streams are being sent (`Sending` of
    /// [`crate::streams`]). WHY one for every PUBLISH of the track, and not
    /// a new one with each as `opened`: a PUBLISH is sent again only after
    /// the `PUBLISH_DONE` of the one before, which waited for its streams to
    /// be closed; the streams counted are the new PUBLISH's alone.
    sending: watch::Sender<usize>,
}

/// A PUBLISH that ends once its streams are closed: its `PUBLISH_DONE`
/// waits for every subgroup stream of it to be closed (§10.11: "A sender
/// MUST NOT send `PUBLISH_DONE` until it has closed all streams it will
/// ever open"), and then counts them (Stream Count).
#[derive(Clone)]
pub struct Ending {
    /// The token of the PUBLISH.
    pub token: u64,
    /// The Status Code of its `PUBLISH_DONE`.
    pub status: u64,
    opened: Arc<AtomicU64>,
    sending: watch::Receiver<usize>,
}

impl Ending {
    /// Completes with the token of the PUBLISH when no stream of it is being
    /// sent. Also where its [`Publication`] went with its last sender: then
    /// none is.
    pub fn when_drained(&self) -> impl Future<Output = u64> + Send + 'static {
        let (token, mut sending) = (self.token, self.sending.clone());
        async move {
            let _ = sending.wait_for(|count| *count == 0).await;
            token
        }
    }

    /// The subgroup streams it opened: the Stream Count of its
    /// `PUBLISH_DONE`.
    #[must_use]
    pub fn streams(&self) -> u64 {
        self.opened.load(Ordering::Relaxed)
    }
}

impl<T> Publication<T> {
    /// Not published, with no more than `most_waiting` Objects waiting for
    /// a `PUBLISH_OK`; the oldest goes for one more.
    #[must_use]
    pub fn new(most_waiting: usize) -> Self {
        Self {
            publish: None,
            track_alias: 0,
            answered: false,
            live: false,
            opened: Arc::new(AtomicU64::new(0)),
            waiting: VecDeque::new(),
            most_waiting,
            not_before: None,
            sending: watch::channel(0).0,
        }
    }

    /// Whether it is to be published at `now`: it has no PUBLISH, and is
    /// not waiting after one that failed.
    #[must_use]
    pub fn to_publish(&self, now: Instant) -> bool {
        self.publish.is_none() && self.not_before.is_none_or(|after| now >= after)
    }

    /// A PUBLISH with `token` was sent for it, with `track_alias`.
    pub fn published(&mut self, token: u64, track_alias: u64) {
        self.publish = Some(token);
        self.track_alias = track_alias;
        self.answered = false;
        self.live = false;
        self.opened = Arc::new(AtomicU64::new(0));
        self.not_before = None;
    }

    /// The token of its PUBLISH, while there is one.
    #[must_use]
    pub fn publish(&self) -> Option<u64> {
        self.publish
    }

    /// Where the PUBLISH of `token` stands, where it is this track's.
    #[must_use]
    pub fn stage(&self, token: u64) -> Option<PublishStage> {
        (self.publish == Some(token)).then_some(if self.answered {
            PublishStage::Established
        } else {
            PublishStage::Pending
        })
    }

    /// Whether Objects are sent on it.
    #[must_use]
    pub fn live(&self) -> bool {
        self.live
    }

    /// The Track Alias of its PUBLISH.
    #[must_use]
    pub fn track_alias(&self) -> u64 {
        self.track_alias
    }

    /// The count of the subgroup streams of its PUBLISH that were opened.
    #[must_use]
    pub fn opened(&self) -> &Arc<AtomicU64> {
        &self.opened
    }

    /// The count of the Objects whose streams are being sent.
    #[must_use]
    pub fn sending(&self) -> &watch::Sender<usize> {
        &self.sending
    }

    /// `object` waits for the `PUBLISH_OK`, where there is a PUBLISH;
    /// returns whether it does.
    pub fn wait(&mut self, object: T) -> bool {
        if self.publish.is_none() {
            return false;
        }
        if self.waiting.len() >= self.most_waiting {
            self.waiting.pop_front();
        }
        self.waiting.push_back(object);
        true
    }

    /// The `PUBLISH_OK` of the PUBLISH with `token`, with its Forward State:
    /// what waited for it, to be sent. Nothing for a PUBLISH that is
    /// ending, or one before.
    pub fn established(&mut self, token: u64, forward: bool) -> VecDeque<T> {
        if self.publish != Some(token) {
            return VecDeque::new();
        }
        self.answered = true;
        self.live = forward;
        std::mem::take(&mut self.waiting)
    }

    /// The PUBLISH with `token` is over; the next Object publishes it again,
    /// after [`PUBLISH_AGAIN_AFTER`] where it `failed`. Returns whether it
    /// was its PUBLISH.
    pub fn unpublished(&mut self, token: u64, failed: bool, now: Instant) -> bool {
        if self.publish != Some(token) {
            return false;
        }
        self.publish = None;
        self.answered = false;
        self.live = false;
        self.waiting.clear();
        self.not_before = failed.then(|| now + PUBLISH_AGAIN_AFTER);
        true
    }

    /// Its PUBLISH ends with `status`: nothing is sent on it from now on,
    /// and the `PUBLISH_DONE` waits for the [`Ending`]. `None` where it has
    /// none.
    pub fn end(&mut self, status: u64) -> Option<Ending> {
        let token = self.publish.take()?;
        self.answered = false;
        self.live = false;
        self.waiting.clear();
        Some(Ending {
            token,
            status,
            opened: Arc::clone(&self.opened),
            sending: self.sending.subscribe(),
        })
    }

    /// It may be published again at once, after its `PUBLISH_DONE`.
    pub fn may_publish_again(&mut self) {
        self.not_before = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `fixtures/own-requests/stream-ends.tsv` for a PUBLISH,
    /// which `@xmoq/moqt` reads as well. In Rust the session does not decide
    /// what the end of the stream of a request of this end does; for a
    /// PUBLISH [`stream_end`] does, by which the gatherer of the chat and the
    /// transcriber go.
    /// A `PUBLISH_OK` with Forward State 0 answers the PUBLISH as one with
    /// 1 does: what the end of its stream does goes by the answer, not by
    /// whether Objects are sent.
    #[test]
    fn a_publish_is_established_by_its_publish_ok_whatever_its_forward_state() {
        for (forward, answered, expected) in [
            (true, false, Some(PublishStage::Pending)),
            (true, true, Some(PublishStage::Established)),
            (false, true, Some(PublishStage::Established)),
        ] {
            let mut publication = Publication::<()>::new(1);
            publication.published(TOKEN, FIRST_TRACK_ALIAS);
            if answered {
                publication.established(TOKEN, forward);
            }
            assert_eq!(publication.stage(TOKEN), expected, "{forward} {answered}");
            assert_eq!(publication.stage(TOKEN + 1), None);
        }
    }

    #[test]
    fn the_end_of_the_stream_of_a_publish_does_what_the_shared_table_says() {
        let table = include_str!("../../../fixtures/own-requests/stream-ends.tsv");
        let mut read = 0;
        for row in table.lines() {
            let [name, request, state, event, expected] = row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("a row of five columns: {row}");
            };
            if request != "publish" {
                continue;
            }
            read += 1;
            let at = match state {
                "pending" => PublishStage::Pending,
                "established" => PublishStage::Established,
                "done" => PublishStage::Done,
                other => panic!("{name}: {other}"),
            };
            let end = match event {
                "fin" => RequestStreamEnd::Fin,
                "reset" => RequestStreamEnd::Reset,
                "stop-sending" => RequestStreamEnd::StopSending,
                other => panic!("{name}: {other}"),
            };
            let got = match stream_end(at, end) {
                StreamEndDoes::Ends => "ends",
                StreamEndDoes::Nothing => "nothing",
                StreamEndDoes::PeerEnd => "reported",
            };
            assert_eq!(got, expected, "{name}");
        }
        assert_eq!(read, 9);
    }

    const TOKEN: u64 = 7;
    const ALIAS: u64 = 1;

    /// What the client did to a publication.
    #[derive(Debug, Clone, Copy)]
    enum Event {
        Publish,
        Wait(u32),
        /// Its `PUBLISH_OK`, for this token, with this Forward State.
        Ok(u64, bool),
        /// It is over, for this token, failed or not.
        Over(u64, bool),
        End,
    }

    fn after(events: &[Event], now: Instant) -> (Publication<u32>, Vec<u32>) {
        let mut publication = Publication::new(2);
        let mut sent = Vec::new();
        for event in events {
            match *event {
                Event::Publish => publication.published(TOKEN, ALIAS),
                Event::Wait(object) => {
                    publication.wait(object);
                }
                Event::Ok(token, forward) => sent = publication.established(token, forward).into(),
                Event::Over(token, failed) => {
                    publication.unpublished(token, failed, now);
                }
                Event::End => {
                    publication.end(0);
                }
            }
        }
        (publication, sent)
    }

    #[test]
    fn what_waits_for_a_publish_ok_is_sent_with_it() {
        // (what happened; what is sent at the PUBLISH_OK; whether Objects
        // are sent on it then)
        let cases: [(&str, Vec<Event>, Vec<u32>, bool); 6] = [
            (
                "the latest two wait",
                vec![
                    Event::Publish,
                    Event::Wait(1),
                    Event::Wait(2),
                    Event::Wait(3),
                    Event::Ok(TOKEN, true),
                ],
                vec![2, 3],
                true,
            ),
            (
                "nothing waits without a PUBLISH",
                vec![Event::Wait(1), Event::Publish, Event::Ok(TOKEN, true)],
                vec![],
                true,
            ),
            (
                "Forward State 0: nothing is sent live",
                vec![Event::Publish, Event::Wait(1), Event::Ok(TOKEN, false)],
                vec![1],
                false,
            ),
            (
                "the PUBLISH_OK of a PUBLISH before",
                vec![Event::Publish, Event::Wait(1), Event::Ok(TOKEN + 1, true)],
                vec![],
                false,
            ),
            (
                "the PUBLISH_OK of a PUBLISH that is ending",
                vec![
                    Event::Publish,
                    Event::Wait(1),
                    Event::End,
                    Event::Ok(TOKEN, true),
                ],
                vec![],
                false,
            ),
            (
                "what waited goes with a PUBLISH that is over",
                vec![
                    Event::Publish,
                    Event::Wait(1),
                    Event::Over(TOKEN, false),
                    Event::Publish,
                    Event::Ok(TOKEN, true),
                ],
                vec![],
                true,
            ),
        ];
        for (name, events, sent, live) in cases {
            let (publication, got) = after(&events, Instant::now());
            assert_eq!(got, sent, "{name}");
            assert_eq!(publication.live(), live, "{name}");
        }
    }

    #[test]
    fn a_publication_is_published_again_after_a_failure_and_its_pause() {
        let now = Instant::now();
        // (what happened; whether it is to be published now, and once the
        // pause is over)
        let cases: [(&str, Vec<Event>, bool, bool); 5] = [
            ("never published", vec![], true, true),
            ("published", vec![Event::Publish], false, false),
            (
                "over without a failure",
                vec![Event::Publish, Event::Over(TOKEN, false)],
                true,
                true,
            ),
            (
                "over with a failure",
                vec![Event::Publish, Event::Over(TOKEN, true)],
                false,
                true,
            ),
            (
                "the end of a PUBLISH before",
                vec![Event::Publish, Event::Over(TOKEN + 1, true)],
                false,
                false,
            ),
        ];
        for (name, events, at_once, later) in cases {
            let (publication, _) = after(&events, now);
            assert_eq!(publication.to_publish(now), at_once, "{name}");
            assert_eq!(
                publication.to_publish(now + PUBLISH_AGAIN_AFTER),
                later,
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn the_publish_done_waits_for_the_streams_being_sent() {
        let mut publication = Publication::<u32>::new(2);
        publication.published(TOKEN, ALIAS);
        publication.opened().fetch_add(3, Ordering::Relaxed);
        publication.sending().send_replace(1);
        let ending = publication.end(0x2).unwrap();
        assert_eq!(
            (ending.token, ending.status, ending.streams()),
            (TOKEN, 0x2, 3)
        );
        assert!(publication.end(0x2).is_none(), "it has no PUBLISH");
        let mut drained = Box::pin(ending.when_drained());
        assert!(
            poll_once(&mut drained).await.is_none(),
            "a stream is being sent"
        );
        publication.sending().send_replace(0);
        assert_eq!(drained.await, TOKEN);
    }

    /// Polls `future` once.
    async fn poll_once<F: Future + Unpin>(future: &mut F) -> Option<F::Output> {
        tokio::time::timeout(std::time::Duration::ZERO, future)
            .await
            .ok()
    }
}
