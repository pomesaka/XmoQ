//! The requests of a viewer on its session: SUBSCRIBE, and the catalog of
//! MSF by a Joining FETCH.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::SubgroupObject;
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::kvp::Value;
use xmoq_moqt::message::FetchRange;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, RequestReader, RequestStreamEvent, StreamStops, Trace, UniEvent,
    send_message,
};
use xmoq_msf::CatalogTrack;

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// The name of the catalog track (MSF §5).
const CATALOG: &str = "catalog";
/// How long the catalog has to come, as the watching page waits.
const CATALOG_WAIT: Duration = Duration::from_secs(5);
/// The most a catalog may be.
const CATALOG_LIMIT: usize = 1024 * 1024;

/// A subscription: its request stream, kept open for as long as the track
/// is wanted.
pub struct Subscription {
    pub request: RequestId,
    pub track_alias: u64,
    pub reader: RequestReader<Recv>,
    _send: Send,
}

/// What has to stay open for as long as the session after the catalog was
/// read: the subscription to the catalog track and the request stream of
/// the FETCH.
pub struct CatalogKept {
    _subscription: Subscription,
    _fetch: Send,
}

/// What the requests of a session are sent with.
pub struct Requests<'a> {
    pub connection: &'a WtConnection,
    pub session: &'a mut Session,
    pub namespace: &'a TrackNamespace,
    /// The AUTHORIZATION TOKEN every request carries, where there is one.
    pub token: Option<&'a [u8]>,
}

impl Requests<'_> {
    /// Subscribes to a track of the namespace.
    pub async fn subscribe(&mut self, name: &str) -> Result<Subscription> {
        let full_name = FullTrackName::new(self.namespace.clone(), name.as_bytes().to_vec())?;
        let mut parameters = vec![
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        if let Some(token) = self.token {
            parameters.push(Parameter::AuthorizationToken(token.to_vec()));
        }
        let (request, message) = self.session.subscribe(full_name, parameters)?;
        let (mut send, recv) = self.connection.open_bi().await?;
        let trace = Trace::default();
        send_message(&mut send, &message, &trace, name).await?;
        let mut reader = RequestReader::new(recv, &trace, name);
        let RequestStreamEvent::Message(response) = reader.next_event().await? else {
            bail!("the relay closed the SUBSCRIBE of {name} without an answer");
        };
        match self.session.on_request_message(request, response)? {
            RequestEvent::SubscribeEstablished { track_alias, .. } => Ok(Subscription {
                request,
                track_alias,
                reader,
                _send: send,
            }),
            other => bail!("SUBSCRIBE of {name} was refused: {other:?}"),
        }
    }

    /// Reads the catalog of the namespace: its tracks.
    ///
    /// WHY a Joining FETCH: the catalog is one Object that was published
    /// before the viewer came, so a SUBSCRIBE alone does not bring it
    /// (§10.12.2: the latest Group of the track the SUBSCRIBE is for), as
    /// `followCatalog` of `@xmoq/msf` does.
    pub async fn catalog(
        &mut self,
        events: &mut mpsc::Receiver<UniEvent>,
        stops: &StreamStops,
    ) -> Result<(Vec<CatalogTrack>, CatalogKept)> {
        let subscription = self.subscribe(CATALOG).await?;
        let range = FetchRange::RelativeJoining {
            joining_request_id: subscription.request.value(),
            joining_start: 0,
        };
        let parameters = self
            .token
            .map(|token| Parameter::AuthorizationToken(token.to_vec()))
            .into_iter()
            .collect();
        let (fetch, message) = self.session.fetch(range, parameters)?;
        let (mut send, recv) = self.connection.open_bi().await?;
        let trace = Trace::default();
        send_message(&mut send, &message, &trace, "fetch").await?;
        let mut answers = Some(RequestReader::new(recv, &trace, "fetch"));
        let session = &mut *self.session;

        let payload = async {
            loop {
                tokio::select! {
                    answer = async {
                        match answers.as_mut() {
                            Some(answers) => answers.next_event().await,
                            None => std::future::pending().await,
                        }
                    } => match answer? {
                        // FETCH_OK, or a refusal: then the catalog can
                        // still come on the subscription.
                        RequestStreamEvent::Message(message) => {
                            session.on_request_message(fetch, message)?;
                        }
                        RequestStreamEvent::Closed(_) => answers = None,
                    },
                    event = events.recv() => match event.ok_or_else(|| anyhow!("the connection is gone"))? {
                        UniEvent::Object { header, object, .. }
                            if header.track_alias == subscription.track_alias =>
                        {
                            return Ok::<_, anyhow::Error>(object.payload);
                        }
                        UniEvent::FetchOpened { stream, header } => {
                            session.on_fetch_stream(header.request_id)?;
                            let Some(mut body) = stops.take_fetch_body(stream) else { continue };
                            let mut decoder = FetchStreamDecoder::new(CATALOG_LIMIT, GroupOrder::Ascending);
                            // The readers took the header off; the decoder wants to see one.
                            decoder.push(&FetchStreamEncoder::new(header, GroupOrder::Ascending).take_bytes())?;
                            // The last Object of the Group is the catalog as it is now.
                            let mut latest = None;
                            loop {
                                while let Some(event) = decoder.next_event()? {
                                    if let FetchEvent::Object(object) = event {
                                        latest = Some(object.payload);
                                    }
                                }
                                match body.next().await {
                                    FetchChunk::Bytes(bytes) => decoder.push(&bytes)?,
                                    _ => break,
                                }
                            }
                            if let Some(payload) = latest {
                                return Ok(payload);
                            }
                        }
                        UniEvent::SubgroupClosed { header, .. } => {
                            session.on_data_stream_closed(header.track_alias);
                        }
                        UniEvent::Failed(error) => return Err(error.into()),
                        _ => {}
                    },
                }
            }
        };
        let payload = tokio::time::timeout(CATALOG_WAIT, payload)
            .await
            .map_err(|_| anyhow!("no catalog came within 5 s"))??;
        let tracks = xmoq_msf::read_catalog(&payload).context("the catalog")?;
        Ok((
            tracks,
            CatalogKept {
                _subscription: subscription,
                _fetch: send,
            },
        ))
    }
}

/// The Timestamp of LOC of an Object, in microseconds. `None` for an
/// Object without one, with one that is invalid, or with a Timescale of
/// its own, which is not handled: the demo has none (wall-clock
/// microseconds).
pub fn timestamp_of(object: &SubgroupObject) -> Option<u64> {
    let timing = loc::read_timing(object.properties.iter().map(|pair| {
        let value = match pair.value() {
            Value::Varint(value) => Some(*value),
            Value::Bytes(_) => None,
        };
        (pair.ty(), value)
    }))
    .ok()??;
    timing.timescale.is_none().then_some(timing.timestamp)
}
