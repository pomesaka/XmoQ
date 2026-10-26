//! What this client adds to `xmoq-moqt-io`: a request stream as one value, and
//! the split of incoming events into the ones that concern the session and the
//! ones that carry data.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::session::{PeerRequestId, Role, Session, Transport};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::refuse::{Refusal, refusal};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, IdleSubgroupStreamTimeout, IncomingStreams, PeerRequestStreamEvent, RequestClose,
    RequestReader, RequestStreamEvent, SendStream, StreamConfig, StreamStops, Trace, UniEvent,
    WriteError, read_request_streams, read_uni_streams, send_message,
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
    /// The peer finished its direction; nothing more is read from it.
    peer_finished: bool,
    trace: Trace,
    label: String,
}

impl RequestStream {
    pub async fn open(connection: &WtConnection, trace: &Trace, label: &str) -> Result<Self> {
        let (send, recv) = connection.open_bi().await?;
        Ok(Self {
            send,
            reader: RequestReader::new(recv, trace, label),
            peer_finished: false,
            trace: trace.clone(),
            label: label.to_string(),
        })
    }

    pub async fn send(&mut self, message: &ControlMessage) -> Result<()> {
        send_message(&mut self.send, message, &self.trace, &self.label).await?;
        Ok(())
    }

    /// Ends this endpoint's half of the stream with a FIN and waits for the
    /// peer to end its own, for at most `wait`. Returns how the peer did, or
    /// `None` when it did not in time. `late` gets what the peer still says
    /// on the stream: the session has to see it, since a `REQUEST_UPDATE`
    /// that crossed uses up a Request ID (§10.1).
    ///
    /// WHY wait for the peer and not only for the transport: after
    /// `PUBLISH_DONE`, an `XmoQ` relay finishes its direction once it has
    /// received as many subgroup streams as the Stream Count said, and resets
    /// it when it gave up waiting. That tells a publisher whether it can
    /// close the session without losing what it sent last. The transport
    /// alone cannot: a stream that arrived and was not accepted yet can be
    /// lost when the connection closes (`xmoq_moqt_io::Connection::accept_uni`).
    pub async fn finish_and_await_close(
        mut self,
        wait: Duration,
        mut late: impl FnMut(ControlMessage) -> Result<()>,
    ) -> Result<Option<RequestClose>> {
        self.send.finish();
        if self.peer_finished {
            return Ok(Some(RequestClose::Fin));
        }
        let closed = async {
            loop {
                match self.reader.next_event().await? {
                    RequestStreamEvent::Closed(how) => return Ok::<_, anyhow::Error>(how),
                    RequestStreamEvent::Message(message) => late(message)?,
                }
            }
        };
        if let Ok(how) = tokio::time::timeout(wait, closed).await {
            Ok(Some(how?))
        } else {
            self.reader.stop(stream_reset_code::CANCELLED);
            Ok(None)
        }
    }

    /// Ends this endpoint's half of the stream with a FIN and hands over
    /// the peer's: for a caller that waits for the peer to end it as
    /// [`Self::finish_and_await_close`] does, with other things to do
    /// meanwhile.
    pub fn finish_own(self) -> PeerHalf {
        self.send.finish();
        PeerHalf {
            reader: self.reader,
            finished: self.peer_finished,
        }
    }

    /// Ends this endpoint's half of the stream with a FIN and reads no more
    /// of the peer's.
    pub fn finish(self) {
        self.send.finish();
    }

    /// Cancels the request: resets the send half and stops the receive half,
    /// both with CANCELLED (§3.3.2).
    pub fn cancel(self) {
        self.cancel_with(stream_reset_code::CANCELLED);
    }

    /// Cancels the request with `code`: `MALFORMED_TRACK` where its Track is
    /// malformed (§2.4.2).
    pub fn cancel_with(self, code: u32) {
        self.send.reset(code);
        self.reader.stop(code);
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
            event = self.reader.next_event(), if !self.peer_finished => {
                let event = event?;
                if event == RequestStreamEvent::Closed(RequestClose::Fin) {
                    self.peer_finished = true;
                }
                Ok(event)
            }
            stopped = self.send.stopped() => match stopped {
                WriteError::Stopped(code) => {
                    Ok(RequestStreamEvent::Closed(RequestClose::Stopped(code)))
                }
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

/// The peer's half of a request stream whose other half this endpoint has
/// finished.
pub struct PeerHalf {
    reader: RequestReader<Recv>,
    /// The peer had finished it before; nothing more is read from it.
    pub finished: bool,
}

impl PeerHalf {
    /// As [`RequestStream::next_event`].
    pub async fn next_event(&mut self) -> Result<RequestStreamEvent> {
        Ok(self.reader.next_event().await?)
    }

    /// Stops the half, which the peer did not end in time, with CANCELLED.
    pub fn stop(self) {
        self.reader.stop(stream_reset_code::CANCELLED);
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

/// The requests the peer opens, for a client that serves them.
pub struct PeerRequests {
    events: mpsc::Receiver<PeerRequestStreamEvent<Send>>,
}

impl PeerRequests {
    pub async fn next_event(&mut self) -> Result<PeerRequestStreamEvent<Send>> {
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
    /// Absent when the client serves the peer's requests itself.
    refuser: Option<JoinHandle<()>>,
}

impl Drop for Readers {
    fn drop(&mut self) {
        self.router.abort();
        if let Some(refuser) = &self.refuser {
            refuser.abort();
        }
    }
}

/// Answers every request the peer opens with `REQUEST_ERROR` `NOT_SUPPORTED`
/// and a FIN (§3.3.2), after `delay`: this client serves no requests, and
/// leaving one unanswered would keep the peer waiting.
///
/// The requests go through a session of their own, which checks their
/// Request IDs (§10.1) and reads what still comes on a refused stream (a
/// `REQUEST_UPDATE` uses up one more); a fault there closes the session with
/// its code. WHY a session of its own, and not the client's: the client's
/// is held by the mode while it runs. WHY that is enough: in the modes that
/// refuse (`subscribe` and `fetch`), the peer's Request IDs come on nothing
/// but these streams, since this client publishes nothing the peer could
/// update.
fn refuse_requests(
    connection: Arc<WtConnection>,
    config: StreamConfig,
    stops: StreamStops,
    delay: Duration,
) -> (IncomingStreams, JoinHandle<()>) {
    let (queue, mut requests) = mpsc::channel(EVENT_QUEUE);
    let trace = config.trace.clone();
    let streams =
        read_request_streams(Arc::clone(&connection), config, stops, queue, |event| event);
    let refuser = tokio::spawn(async move {
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        // The requests the session keeps as refused, by the reader's name of
        // their stream.
        let mut refused: HashMap<u64, PeerRequestId> = HashMap::new();
        while let Some(event) = requests.recv().await {
            match refusal(&mut session, &mut refused, event) {
                Refusal::Answer {
                    stream,
                    mut send,
                    response,
                } => {
                    let label = format!("bi#{stream}");
                    tokio::time::sleep(delay).await;
                    // An error means the peer is gone or stopped the stream.
                    let _ = send_message(&mut send, &response, &trace, &label).await;
                    send.finish();
                }
                Refusal::Nothing => {}
                Refusal::Close { code, why, error } => {
                    if let Some(error) = &error {
                        crate::report::closed_for_a_request_of_the_peer(code, error);
                    }
                    connection.close(code, why.as_bytes());
                    return;
                }
            }
        }
    });
    (streams, refuser)
}

/// Starts reading the peer's streams. With `serve`, the requests the peer
/// opens are handed to the caller; without, each is answered with
/// `NOT_SUPPORTED` after `answer_delay`. The fetch streams the peer opens
/// come with the data, for the caller to ask its session what each belongs
/// to.
///
/// §3.3 lets Objects arrive before the control stream; they wait in the data
/// queue until asked for. A subgroup stream of the peer that says nothing
/// for `idle_subgroup_stream_timeout` after its header is stopped.
pub fn start_readers(
    connection: Arc<WtConnection>,
    trace: Trace,
    stops: StreamStops,
    serve: bool,
    answer_delay: Duration,
    idle_subgroup_stream_timeout: IdleSubgroupStreamTimeout,
) -> (SessionInbox, DataInbox, Option<PeerRequests>, Readers) {
    let (all, mut incoming) = mpsc::channel(EVENT_QUEUE);
    let (session, session_events) = mpsc::channel(EVENT_QUEUE);
    let (data, data_events) = mpsc::channel(EVENT_QUEUE);
    let config = StreamConfig {
        trace,
        idle_subgroup_stream_timeout,
        ..StreamConfig::default()
    };
    let (requests, refuser, peer_requests) = if serve {
        let (queue, events) = mpsc::channel(EVENT_QUEUE);
        let streams = read_request_streams(
            Arc::clone(&connection),
            config.clone(),
            stops.clone(),
            queue,
            |event| event,
        );
        (streams, None, Some(PeerRequests { events }))
    } else {
        let (streams, refuser) = refuse_requests(
            Arc::clone(&connection),
            config.clone(),
            stops.clone(),
            answer_delay,
        );
        (streams, Some(refuser), None)
    };
    let streams = read_uni_streams(connection, config, stops, all, |event| event);
    // WHY two queues: the client waits for a control message and for data in
    // different places, and neither wait may swallow what the other needs.
    let router = tokio::spawn(async move {
        while let Some(event) = incoming.recv().await {
            let queue = match &event {
                UniEvent::Control(_) | UniEvent::Failed(_) | UniEvent::ReaderPanicked => &session,
                UniEvent::FetchOpened { .. }
                | UniEvent::SubgroupOpened { .. }
                | UniEvent::Object { .. }
                | UniEvent::ObjectBegun { .. }
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
        peer_requests,
        Readers {
            _streams: streams,
            _requests: requests,
            router,
            refuser,
        },
    )
}
