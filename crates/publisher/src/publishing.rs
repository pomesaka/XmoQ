//! The tracks a client publishes on its session, one [`Publication`] for
//! each key `K`, and the steps of each PUBLISH: sending it, writing a Group
//! to it or having the Group wait for its `PUBLISH_OK`, ending it, and its
//! `PUBLISH_DONE` once its streams are closed (§10.11). The gatherer of the
//! chat publishes the tracks of each room so, and the transcriber the
//! transcript of each prefix. The caller tells what comes of each in its log.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use xmoq_moqt::UsageError;
use xmoq_moqt::message::{ControlMessage, ReasonPhrase};
use xmoq_moqt::name::{FullTrackName, Location};
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::Session;
use xmoq_moqt_io::Connection;

use crate::own::OwnRequests;
use crate::publication::{Ending, FIRST_TRACK_ALIAS, Publication};
use crate::streams::{GroupToSend, Sending, send_group};

/// The tracks a client publishes, by `K`, with the Objects `T` that wait
/// for a `PUBLISH_OK`.
pub struct Publishing<K, T> {
    publications: HashMap<K, Publication<T>>,
    next_track_alias: u64,
    /// The PUBLISH requests that end once their streams are closed, by token.
    ending: HashMap<u64, (K, Ending)>,
    /// Where the token of an ending PUBLISH is told once its streams are
    /// closed; the caller then calls [`Self::done`].
    drained: mpsc::Sender<u64>,
    /// The streams of Groups being sent.
    sends: JoinSet<()>,
    most_sends: usize,
}

/// What came of [`Publishing::publish`].
#[derive(Debug)]
pub enum Publish {
    /// The track is published, or waits to be published again.
    NotNow,
    /// The session refused the PUBLISH: the one before it waits for its
    /// streams to be closed, and it is published again once that is over.
    Refused(UsageError),
    /// The PUBLISH was sent, with this token.
    Sent(u64),
}

/// What came of [`Publishing::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// The track has no `PUBLISH_OK` with Forward State 1 ([`Publishing::live`]):
    /// nothing is sent.
    NotLive,
    /// As many streams are being sent as the client lets be: the Object is
    /// not sent.
    TooMany,
    /// Its stream is being sent.
    Sending,
}

/// A PUBLISH whose streams are closed, and what ends it.
pub struct Done<K> {
    /// The track it published.
    pub key: K,
    /// The Status Code of its `PUBLISH_DONE`.
    pub status: u64,
    /// The subgroup streams it opened, the Stream Count of its
    /// `PUBLISH_DONE`.
    pub streams: u64,
    /// Its `PUBLISH_DONE`, to be told to the task of its request, which then
    /// finishes; `None` where the request is gone.
    pub message: Option<ControlMessage>,
}

/// The parameters of a PUBLISH: the Largest Object, where the track has a
/// Group (§10.2.11: what the relay gives its subscribers in `SUBSCRIBE_OK`,
/// and where a Joining FETCH ends), and the token of the publisher.
#[must_use]
pub fn parameters(largest: Option<u64>, token: Option<Parameter>) -> Vec<Parameter> {
    largest
        .map(|group| Parameter::LargestObject(Location { group, object: 0 }))
        .into_iter()
        .chain(token)
        .collect()
}

impl<K: Clone + Eq + Hash, T> Publishing<K, T> {
    /// None yet. The tokens of ending PUBLISH requests whose streams are
    /// closed are told on `drained`; `most_sends` streams of Groups are sent
    /// at once at the most.
    #[must_use]
    pub fn new(drained: mpsc::Sender<u64>, most_sends: usize) -> Self {
        Self {
            publications: HashMap::new(),
            next_track_alias: FIRST_TRACK_ALIAS,
            ending: HashMap::new(),
            drained,
            sends: JoinSet::new(),
            most_sends,
        }
    }

    /// The track of `key`, which is made, not published, with no more than
    /// `most_waiting` Objects waiting for a `PUBLISH_OK`, where there is none.
    pub fn track(&mut self, key: K, most_waiting: usize) -> &mut Publication<T> {
        self.publications
            .entry(key)
            .or_insert_with(|| Publication::new(most_waiting))
    }

    /// The track of `key`, where there is one.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&Publication<T>> {
        self.publications.get(key)
    }

    /// The same, to change.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut Publication<T>> {
        self.publications.get_mut(key)
    }

    /// Forgets the track of `key`.
    pub fn forget(&mut self, key: &K) {
        self.publications.remove(key);
    }

    /// Whether the PUBLISH of `key` is the one of `token`.
    #[must_use]
    pub fn is_publish(&self, key: &K, token: u64) -> bool {
        self.get(key)
            .is_some_and(|entry| entry.publish() == Some(token))
    }

    /// Publishes the track of `key` as `track`, with `parameters`
    /// ([`parameters`]), for `purpose`, unless it is published or waits to
    /// be published again.
    pub fn publish<P, C: Connection>(
        &mut self,
        key: &K,
        track: FullTrackName,
        parameters: &[Parameter],
        session: &mut Session,
        own: &mut OwnRequests<P, C>,
        purpose: P,
    ) -> Publish {
        if !self
            .get(key)
            .is_some_and(|entry| entry.to_publish(Instant::now()))
        {
            return Publish::NotNow;
        }
        let track_alias = self.next_track_alias;
        self.next_track_alias += 1;
        let (request, message) =
            match session.publish(track, track_alias, parameters.to_vec(), vec![]) {
                Ok(published) => published,
                Err(error) => return Publish::Refused(error),
            };
        let token = own.open(purpose, request, message, parameters);
        if let Some(entry) = self.get_mut(key) {
            entry.published(token, track_alias);
        }
        Publish::Sent(token)
    }

    /// `object` waits for the `PUBLISH_OK` of the track of `key`, where it
    /// has a PUBLISH.
    pub fn wait(&mut self, key: &K, object: T) {
        if let Some(entry) = self.get_mut(key) {
            entry.wait(object);
        }
    }

    /// Whether Objects are sent on the track of `key`: where they are not,
    /// the caller publishes it and has the Object [`Self::wait`].
    #[must_use]
    pub fn live(&self, key: &K) -> bool {
        self.get(key).is_some_and(Publication::live)
    }

    /// Sends Group `group_id` of one Object of `payload` on the track of
    /// `key`; where its stream fails, `failed` is called with why, in its
    /// task.
    pub fn send<C: Connection>(
        &mut self,
        key: &K,
        connection: &Arc<C>,
        (group_id, payload): (u64, Vec<u8>),
        failed: impl FnOnce(String) + Send + 'static,
    ) -> Sent {
        let Some(entry) = self.get(key).filter(|entry| entry.live()) else {
            return Sent::NotLive;
        };
        if self.sends.len() >= self.most_sends {
            return Sent::TooMany;
        }
        let group = GroupToSend {
            track_alias: entry.track_alias(),
            group_id,
            payload,
            opened: Arc::clone(entry.opened()),
        };
        let counted = Sending::begin(entry.sending());
        let connection = Arc::clone(connection);
        self.sends.spawn(async move {
            let _counted = counted;
            if let Err(why) = send_group(connection, group).await {
                failed(why);
            }
        });
        Sent::Sending
    }

    /// Waits for a stream of a Group to be sent; for ever where none is.
    pub async fn sent(&mut self) {
        if self.sends.join_next().await.is_none() {
            std::future::pending::<()>().await;
        }
    }

    /// Ends the PUBLISH of `key` with `status`: nothing is sent on it from
    /// now on, and its `PUBLISH_DONE` goes once the streams being sent are
    /// closed ([`Self::done`]). Does nothing where it has none.
    pub fn end(&mut self, key: &K, status: u64) {
        let Some(ending) = self.get_mut(key).and_then(|entry| entry.end(status)) else {
            return;
        };
        let drained = self.drained.clone();
        let when_drained = ending.when_drained();
        tokio::spawn(async move {
            let _ = drained.send(when_drained.await).await;
        });
        self.ending.insert(ending.token, (key.clone(), ending));
    }

    /// The PUBLISH of `token` is over before its `PUBLISH_DONE`.
    pub fn over(&mut self, token: u64) {
        self.ending.remove(&token);
    }

    /// Every stream of the ending PUBLISH of `token` is closed: its
    /// `PUBLISH_DONE` with the number of streams it opened (§10.11), where
    /// `session` still has it.
    ///
    /// # Errors
    ///
    /// Where the session cannot make the `PUBLISH_DONE`.
    pub fn done<P, C: Connection>(
        &mut self,
        token: u64,
        session: &mut Session,
        own: &OwnRequests<P, C>,
    ) -> Result<Option<Done<K>>, UsageError> {
        let Some((key, ending)) = self.ending.remove(&token) else {
            return Ok(None);
        };
        let streams = ending.streams();
        let message = match own.get(token) {
            Some(own) => Some(session.publish_done(
                own.request,
                ending.status,
                streams,
                ReasonPhrase::default(),
            )?),
            None => None,
        };
        Ok(Some(Done {
            key,
            status: ending.status,
            streams,
            message,
        }))
    }
}
