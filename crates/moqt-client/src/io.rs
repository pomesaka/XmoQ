//! What this client adds to `xmoq-moqt-io`: a request stream as one value, and
//! the split of incoming events into the ones that concern the session and the
//! ones that carry data.
//!
//! FIXME: check the Request IDs of requests the peer opens (§10.1). This client
//! serves none: every request is answered with `REQUEST_ERROR` `NOT_SUPPORTED`
//! without going through the session, which is where the IDs are checked.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use xmoq_moqt::message::{ControlMessage, ReasonPhrase, RequestError, request_error_code};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, IncomingStreams, PeerRequestStreamEvent, RequestReader, RequestStreamEvent,
    SendStream, StreamConfig, StreamStops, Trace, UniEvent, WriteError, read_request_streams,
    read_uni_streams, send_message,
};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// Events decoded and not yet taken by the client. When a queue is full the
/// stream readers stop reading, which leaves the pressure to flow control.
/// Provisional.
const EVENT_QUEUE: usize = 16;

/// A request stream: the bidirectional stream one request and its responses
/// use.
///
/// To cancel the request, call [`Self::cancel`]. Merely dropping this value
/// does not reset anything; see `RequestReader`.
pub struct RequestStream {
    send: Send,
    reader: RequestReader<Recv>,
    trace: Trace,
    label: String,
}

impl RequestStream {
    pub async fn open(connection: &WtConnection, trace: &Trace, label: &str) -> Result<Self> {
        let (send, recv) = connection.open_bi().await?;
        Ok(Self {
            send,
            reader: RequestReader::new(recv, trace, label),
            trace: trace.clone(),
            label: label.to_string(),
        })
    }

    pub async fn send(&mut self, message: &ControlMessage) -> Result<()> {
        send_message(&mut self.send, message, &self.trace, &self.label).await?;
        Ok(())
    }

    /// Ends this endpoint's half of the stream with a FIN and lets go of the
    /// other half: nothing more is expected on it. Waits until what was sent
    /// has reached the peer, so that closing the session afterwards does not
    /// take it back.
    pub async fn finish_and_wait(self) -> Result<()> {
        let delivered = self.send.finish_and_wait().await;
        self.reader.stop(stream_reset_code::CANCELLED);
        match delivered {
            // The peer let go of the stream first; it has what it wanted.
            Ok(()) | Err(WriteError::Stopped(_)) => Ok(()),
            Err(WriteError::Lost(reason)) => Err(anyhow::anyhow!("connection lost: {reason}")),
        }
    }

    /// Cancels the request: resets the send half and stops the receive half,
    /// both with CANCELLED (§3.3.2).
    pub fn cancel(self) {
        self.send.reset(stream_reset_code::CANCELLED);
        self.reader.stop(stream_reset_code::CANCELLED);
    }

    /// Like [`Self::next_event`], and also reports the stream closed when the
    /// peer sent `STOP_SENDING` for this endpoint's half. That is how the
    /// subscriber of a subscription made with PUBLISH ends it (§5.1), so the
    /// stream of an established PUBLISH is read with this method.
    ///
    /// WHY not always: on the stream of a request that is not answered yet, or
    /// of a SUBSCRIBE that already got `PUBLISH_DONE`, a `STOP_SENDING` alone
    /// changes nothing; the request ends when the peer's own direction does.
    pub async fn next_event_or_stopped(&mut self) -> Result<RequestStreamEvent> {
        tokio::select! {
            event = self.reader.next_event() => Ok(event?),
            stopped = self.send.stopped() => match stopped {
                WriteError::Stopped(code) => Ok(RequestStreamEvent::Closed(match code {
                    Some(code) => format!("STOP_SENDING, code {code:#x}"),
                    None => "STOP_SENDING, without an application code".to_string(),
                })),
                // A lost connection is reported where the session is watched.
                WriteError::Lost(_) => std::future::pending().await,
            },
        }
    }

    /// The next message, or how the peer ended the stream. Safe to drop while
    /// waiting, as `tokio::select!` does.
    ///
    /// An error is a protocol error in the bytes, or a lost connection.
    pub async fn next_event(&mut self) -> Result<RequestStreamEvent> {
        Ok(self.reader.next_event().await?)
    }
}

/// Messages from the peer's control stream, and what ends the session when it
/// comes from one of its streams.
pub struct SessionInbox {
    events: mpsc::Receiver<UniEvent>,
}

impl SessionInbox {
    /// The next control message, or the error that ends the session.
    pub async fn next_control_message(&mut self) -> Result<ControlMessage> {
        match self.events.recv().await {
            Some(UniEvent::Control(message)) => Ok(message),
            Some(UniEvent::Failed(error)) => Err(error.into()),
            Some(UniEvent::ReaderPanicked) => Err(anyhow!("a stream reader panicked")),
            Some(other) => Err(anyhow!("misrouted event {other:?}")),
            None => Err(anyhow!("connection closed")),
        }
    }
}

/// What the peer's subgroup streams produced, in arrival order per stream.
pub struct DataInbox {
    events: mpsc::Receiver<UniEvent>,
}

impl DataInbox {
    pub async fn next_event(&mut self) -> Result<UniEvent> {
        self.events
            .recv()
            .await
            .ok_or_else(|| anyhow!("connection closed"))
    }
}

/// Stops the stream readers when dropped.
pub struct Readers {
    _streams: IncomingStreams,
    _requests: IncomingStreams,
    router: JoinHandle<()>,
    refuser: JoinHandle<()>,
}

impl Drop for Readers {
    fn drop(&mut self) {
        self.router.abort();
        self.refuser.abort();
    }
}

/// Answers every request the peer opens with `REQUEST_ERROR` `NOT_SUPPORTED`
/// and a FIN (§3.3.2): this client serves no requests, and leaving one
/// unanswered would keep the peer waiting.
fn refuse_requests(
    connection: Arc<WtConnection>,
    config: StreamConfig,
    stops: StreamStops,
) -> (IncomingStreams, JoinHandle<()>) {
    let (queue, mut requests) = mpsc::channel(EVENT_QUEUE);
    let trace = config.trace.clone();
    let streams = read_request_streams(connection, config, stops, queue, |event| event);
    let refuser = tokio::spawn(async move {
        let not_supported = ControlMessage::RequestError(RequestError {
            error_code: request_error_code::NOT_SUPPORTED,
            retry_interval: 0,
            error_reason: ReasonPhrase::default(),
            redirect: None,
        });
        while let Some(event) = requests.recv().await {
            // What follows on a refused stream, and how it ends, is of no
            // interest. A stream that breaks the protocol is left to the peer's
            // own streams to show: this task has no way to close the session.
            if let PeerRequestStreamEvent::Opened {
                stream, mut send, ..
            } = event
            {
                let label = format!("bi#{stream}");
                // An error means the peer is gone or stopped the stream.
                let _ = send_message(&mut send, &not_supported, &trace, &label).await;
                send.finish();
            }
        }
    });
    (streams, refuser)
}

/// Starts reading the peer's unidirectional streams.
///
/// §3.3 lets Objects arrive before the control stream; they wait in the data
/// queue until asked for.
pub fn start_readers(
    connection: Arc<WtConnection>,
    trace: Trace,
    stops: StreamStops,
) -> (SessionInbox, DataInbox, Readers) {
    let (all, mut incoming) = mpsc::channel(EVENT_QUEUE);
    let (session, session_events) = mpsc::channel(EVENT_QUEUE);
    let (data, data_events) = mpsc::channel(EVENT_QUEUE);
    let config = StreamConfig {
        trace,
        ..StreamConfig::default()
    };
    let (requests, refuser) =
        refuse_requests(Arc::clone(&connection), config.clone(), stops.clone());
    let streams = read_uni_streams(connection, config, stops, all, |event| event);
    // WHY two queues: the client waits for a control message and for data in
    // different places, and neither wait may swallow what the other needs.
    let router = tokio::spawn(async move {
        while let Some(event) = incoming.recv().await {
            let queue = match &event {
                UniEvent::Control(_) | UniEvent::Failed(_) | UniEvent::ReaderPanicked => &session,
                UniEvent::SubgroupOpened { .. }
                | UniEvent::Object { .. }
                | UniEvent::TrackMalformed { .. }
                | UniEvent::SubgroupClosed { .. } => &data,
            };
            if queue.send(event).await.is_err() {
                return;
            }
        }
    });
    (
        SessionInbox {
            events: session_events,
        },
        DataInbox {
            events: data_events,
        },
        Readers {
            _streams: streams,
            _requests: requests,
            router,
            refuser,
        },
    )
}
