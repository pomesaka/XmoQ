//! The MSF catalog of a broadcast: which tracks it has. The catalog is one
//! Object that was published before the viewer came, so a SUBSCRIBE alone
//! does not bring it: a Joining FETCH asks for the latest Group, as
//! `followCatalog` of `@xmoq/msf` does.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::message::FetchRange;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::Session;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{Connection, FetchChunk, RequestReader, RequestStreamEvent, StreamStops, Trace, UniEvent, send_message};
use xmoq_msf::CatalogTrack;

/// The name of the catalog track (MSF §5).
const CATALOG: &str = "catalog";
/// How long the catalog has to come, as the watching page waits.
const WAIT: Duration = Duration::from_secs(5);
/// The most a catalog may be.
const LIMIT: usize = 1024 * 1024;

/// What has to stay open for as long as the session: the subscription to
/// the catalog track and the request stream of the FETCH.
pub struct Kept {
    _subscription: crate::Subscription,
    _fetch: <WtConnection as Connection>::Send,
}

/// Reads the catalog of the namespace: its tracks.
pub async fn read(
    connection: &WtConnection,
    session: &mut Session,
    events: &mut mpsc::Receiver<UniEvent>,
    stops: &StreamStops,
    namespace: &TrackNamespace,
    token: Option<&[u8]>,
) -> Result<(Vec<CatalogTrack>, Kept)> {
    let subscription = crate::subscribe(connection, session, namespace, CATALOG, token).await?;
    // §10.12.2: the latest Group of the track the SUBSCRIBE is for.
    let range = FetchRange::RelativeJoining { joining_request_id: subscription.request.value(), joining_start: 0 };
    let parameters = token.map(|token| Parameter::AuthorizationToken(token.to_vec())).into_iter().collect();
    let (fetch, message) = session.fetch(range, parameters)?;
    let (mut send, recv) = connection.open_bi().await?;
    let trace = Trace::default();
    send_message(&mut send, &message, &trace, "fetch").await?;
    let mut answers = Some(RequestReader::new(recv, &trace, "fetch"));

    let payload = async {
        loop {
            tokio::select! {
                answer = async {
                    match answers.as_mut() {
                        Some(answers) => answers.next_event().await,
                        None => std::future::pending().await,
                    }
                } => match answer? {
                    // FETCH_OK, or a refusal: then the catalog can still come on the subscription.
                    RequestStreamEvent::Message(message) => {
                        session.on_request_message(fetch, message)?;
                    }
                    RequestStreamEvent::Closed(_) => answers = None,
                },
                event = events.recv() => match event.ok_or_else(|| anyhow!("the connection is gone"))? {
                    UniEvent::Object { header, object, .. } if header.track_alias == subscription.track_alias => {
                        return Ok::<_, anyhow::Error>(object.payload);
                    }
                    UniEvent::FetchOpened { stream, header } => {
                        session.on_fetch_stream(header.request_id)?;
                        let Some(mut body) = stops.take_fetch_body(stream) else { continue };
                        let mut decoder = FetchStreamDecoder::new(LIMIT, GroupOrder::Ascending);
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
                    UniEvent::SubgroupClosed { header, .. } => session.on_data_stream_closed(header.track_alias),
                    UniEvent::Failed(error) => return Err(error.into()),
                    _ => {}
                },
            }
        }
    };
    let payload = tokio::time::timeout(WAIT, payload).await.map_err(|_| anyhow!("no catalog came within 5 s"))??;
    let tracks = xmoq_msf::read_catalog(&payload).context("the catalog")?;
    if tracks.is_empty() {
        bail!("the catalog has no tracks");
    }
    Ok((tracks, Kept { _subscription: subscription, _fetch: send }))
}
