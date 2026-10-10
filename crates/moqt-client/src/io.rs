//! The WebTransport side of the client: moves bytes between `wtransport` streams
//! and the decoders of `xmoq-moqt`.
//!
//! FIXME: accept the bidirectional streams the peer opens (`accept_bi`) and hand
//! them to the responder side of the session once it exists. Today they are never
//! accepted, so a request a relay sends to this client gets no answer.
//!
//! FIXME: read datagrams. The client never does, so an `OBJECT_DATAGRAM` is lost
//! and an unknown datagram type does not close the session (§11).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::{JoinError, JoinHandle, JoinSet};
use wtransport::endpoint::ConnectOptions;
use wtransport::error::StreamReadError;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, RecvStream, SendStream, VarInt};
use xmoq_moqt::data::{
    SubgroupEvent, SubgroupHeader, SubgroupObject, SubgroupStreamDecoder, UniStreamType,
    uni_stream_type,
};
use xmoq_moqt::message::{ControlMessage, MessageDecoder};
use xmoq_moqt::{Error as MoqtError, ErrorScope, stream_reset_code};

use crate::report::{self, Direction};

const READ_CHUNK: usize = 64 * 1024;

/// Bytes one subgroup stream may hold undecoded, which caps the Object size this
/// client accepts.
///
/// FIXME: replace with the Object size limit once `XmoQ` decides one. 1 MiB is a
/// placeholder that is ample for the Objects this client is tested with.
const SUBGROUP_BUFFER_LIMIT: usize = 1024 * 1024;

/// Unidirectional streams of the peer read at once. Further streams wait
/// unaccepted, where QUIC flow control holds the peer back.
const MAX_UNI_STREAMS: usize = 64;

/// Objects decoded and not yet taken by the client. When the queue is full the
/// stream readers stop reading, which again leaves the pressure to flow control.
///
/// WHY this cannot starve the request stream of flow control (§11.4.2 asks that
/// control streams get it first): only per-stream windows fill up. The
/// connection-level receive window is unlimited by default
/// (`receive_window: VarInt::MAX` in `TransportConfig::default`, quinn-proto
/// 0.11.19, which wtransport 0.7.2 uses for `ClientConfig`; read 2026-10-10), so
/// unread data streams take nothing away from the stream that carries
/// `SUBSCRIBE_OK`.
const OBJECT_QUEUE: usize = 16;

/// Session events not yet taken. One SETUP is all a well-behaved peer sends.
const SESSION_QUEUE: usize = 4;

/// Prints the raw bytes of every stream when `--trace` is given. The fixtures in
/// `fixtures/moqt-18/` were captured this way.
#[derive(Clone, Copy)]
pub struct Trace {
    enabled: bool,
}

impl Trace {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    fn bytes(self, direction: Direction, stream: &str, bytes: &[u8]) {
        if self.enabled {
            report::trace(direction, stream, bytes);
        }
    }
}

pub async fn connect(url: &str, cert_sha256: [u8; 32]) -> Result<Arc<Connection>> {
    // WHY pin the hash: the relay under test runs on a self-signed certificate
    // that no root store trusts. Pinning keeps verification on.
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes([Sha256Digest::new(cert_sha256)])
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    // §3.1.3: the version is offered in WT-Available-Protocols, a structured
    // field list of strings, hence the quotes.
    let options = ConnectOptions::builder(url)
        .add_header(
            "wt-available-protocols",
            format!("\"{}\"", xmoq_moqt::PROTOCOL),
        )
        .build();
    let connection = Endpoint::client(config)?
        .connect(options)
        .await
        .context("WebTransport connect failed")?;
    Ok(Arc::new(connection))
}

pub async fn send_message(
    stream: &mut SendStream,
    message: &ControlMessage,
    trace: Trace,
    label: &str,
) -> Result<()> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes)?;
    trace.bytes(Direction::Send, label, &bytes);
    stream.write_all(&bytes).await?;
    Ok(())
}

/// How a read ended without bytes and without a FIN.
enum ReadFailure {
    /// The peer reset the stream, with this code.
    Reset(u64),
    /// The connection is gone, or the transport reported something it should not.
    Lost(anyhow::Error),
}

/// The receiving half of a stream, with its read buffer and its trace label.
struct TracedRecv {
    recv: RecvStream,
    chunk: Vec<u8>,
    trace: Trace,
    label: String,
}

impl TracedRecv {
    fn new(recv: RecvStream, trace: Trace, label: &str) -> Self {
        Self {
            recv,
            chunk: vec![0; READ_CHUNK],
            trace,
            label: label.to_string(),
        }
    }

    /// The next chunk, or `None` at a FIN.
    async fn read(&mut self) -> Result<Option<&[u8]>, ReadFailure> {
        let len = match self.recv.read(&mut self.chunk).await {
            Ok(Some(len)) => len,
            Ok(None) => return Ok(None),
            Err(StreamReadError::Reset(code)) => return Err(ReadFailure::Reset(code.into_inner())),
            Err(error) => return Err(ReadFailure::Lost(error.into())),
        };
        let Some(received) = self.chunk.get(..len) else {
            return Err(ReadFailure::Lost(anyhow!(
                "{}: a read reported {len} bytes for a buffer of {}",
                self.label,
                self.chunk.len()
            )));
        };
        self.trace.bytes(Direction::Recv, &self.label, received);
        Ok(Some(received))
    }

    /// Tells the peer to stop sending on this stream (`STOP_SENDING`).
    fn stop(self, code: u32) {
        self.recv.stop(VarInt::from_u32(code));
    }
}

/// What a request stream produced next.
pub enum RequestStreamEvent {
    Message(ControlMessage),
    /// The peer ended the stream, by FIN or by reset, which ends the request
    /// (§11.4.1). The text says how.
    Closed(String),
}

/// A request stream: the bidirectional stream one request and its responses use.
///
/// To cancel the request, call [`Self::cancel`]. Merely dropping this value does
/// not reset anything: the send half ends with a FIN and the receive half with
/// `STOP_SENDING` code 0 (`impl Drop for SendStream` and `for RecvStream` in
/// quinn 0.11.12, which wtransport 0.7.2 wraps; read 2026-10-10).
pub struct RequestStream {
    send: SendStream,
    recv: TracedRecv,
    decoder: MessageDecoder,
    trace: Trace,
}

impl RequestStream {
    pub async fn open(connection: &Connection, trace: Trace, label: &str) -> Result<Self> {
        let (send, recv) = connection.open_bi().await?.await?;
        Ok(Self {
            send,
            recv: TracedRecv::new(recv, trace, label),
            decoder: MessageDecoder::new(),
            trace,
        })
    }

    pub async fn send(&mut self, message: &ControlMessage) -> Result<()> {
        send_message(&mut self.send, message, self.trace, &self.recv.label).await
    }

    /// Ends this endpoint's half of the stream with a FIN.
    pub async fn finish(&mut self) -> Result<()> {
        self.send.finish().await?;
        Ok(())
    }

    /// Cancels the request: resets the send half and stops the receive half, both
    /// with CANCELLED (§3.3.2).
    pub fn cancel(mut self) {
        // An error only says the stream was already closed, which is fine.
        let _ = self
            .send
            .reset(VarInt::from_u32(stream_reset_code::CANCELLED));
        self.recv.stop(stream_reset_code::CANCELLED);
    }

    /// The next message, or how the peer ended the stream. Safe to drop while
    /// waiting, as `tokio::select!` does: received bytes stay in the decoder.
    ///
    /// An error is a protocol error in the bytes, or a lost connection.
    pub async fn next_event(&mut self) -> Result<RequestStreamEvent> {
        loop {
            if let Some(message) = self.decoder.next_message()? {
                return Ok(RequestStreamEvent::Message(message));
            }
            match self.recv.read().await {
                Ok(Some(received)) => self.decoder.push(received)?,
                Ok(None) => {
                    self.decoder.finish()?;
                    return Ok(RequestStreamEvent::Closed("FIN".to_string()));
                }
                Err(ReadFailure::Reset(code)) => {
                    return Ok(RequestStreamEvent::Closed(format!("reset, code {code:#x}")));
                }
                Err(ReadFailure::Lost(error)) => return Err(error),
            }
        }
    }
}

/// What ends the session when it comes from the peer's streams.
enum SessionEvent {
    /// A message on the peer's control stream.
    Control(ControlMessage),
    /// A stream reader hit an error that the session cannot go on after. The
    /// error keeps its type so the session can be closed with the matching
    /// termination code.
    Failed(anyhow::Error),
}

/// Messages from the peer's control stream, and fatal failures of any stream
/// reader.
pub struct SessionInbox {
    events: mpsc::Receiver<SessionEvent>,
}

impl SessionInbox {
    /// The next control message, or the error that ends the session.
    pub async fn next_control_message(&mut self) -> Result<ControlMessage> {
        match self.events.recv().await {
            Some(SessionEvent::Control(message)) => Ok(message),
            Some(SessionEvent::Failed(error)) => Err(error),
            None => Err(anyhow!("connection closed")),
        }
    }
}

/// What the peer's subgroup streams produced.
pub enum DataEvent {
    Object(SubgroupHeader, SubgroupObject),
    /// A subgroup stream of this Track Alias ended, by FIN, by reset or because
    /// this client stopped it. The session counts these (§10.11).
    StreamClosed {
        track_alias: u64,
    },
    /// A stream of this Track Alias broke the object model. The subscription has
    /// to be cancelled; the session stays open (§2.4.2).
    TrackMalformed {
        track_alias: u64,
        error: MoqtError,
    },
}

/// Events from the peer's subgroup streams, in arrival order per stream.
pub struct ObjectInbox {
    events: mpsc::Receiver<DataEvent>,
    ended_aliases: watch::Sender<Vec<u64>>,
}

impl ObjectInbox {
    /// Tells the stream readers that the subscription under `track_alias` is
    /// over: they send `STOP_SENDING` on its open subgroup streams and on any
    /// that still arrive (§10.11, "SHOULD send `STOP_SENDING` on all streams
    /// related to the subscription when it deletes subscription state").
    ///
    /// The alias stays stopped for the rest of the session. That is right for a
    /// client that subscribes once; one that subscribes again would have to
    /// lift the stop when the publisher reuses the alias (§11.1).
    pub fn stop_track(&self, track_alias: u64) {
        self.ended_aliases
            .send_modify(|aliases| aliases.push(track_alias));
    }

    pub async fn next_event(&mut self) -> Result<DataEvent> {
        self.events
            .recv()
            .await
            .ok_or_else(|| anyhow!("connection closed"))
    }
}

/// Stops the stream readers when dropped.
pub struct Readers(JoinHandle<()>);

impl Drop for Readers {
    fn drop(&mut self) {
        // The accept task owns the per-stream tasks in a JoinSet, which aborts
        // them when it is dropped.
        self.0.abort();
    }
}

/// Where a subgroup stream reader sends what it decodes, and where it learns
/// that nobody wants it any more.
#[derive(Clone)]
struct DataSink {
    events: mpsc::Sender<DataEvent>,
    ended_aliases: watch::Receiver<Vec<u64>>,
}

struct Sinks {
    session: mpsc::Sender<SessionEvent>,
    data: DataSink,
}

/// Starts reading the peer's unidirectional streams.
///
/// Both queues are bounded and every stream reader waits on them, so a peer that
/// sends faster than the client consumes is slowed by flow control instead of
/// growing memory. §3.3 lets Objects arrive before the control stream; they wait
/// in the Object queue until asked for.
pub fn start_readers(
    connection: Arc<Connection>,
    trace: Trace,
) -> (SessionInbox, ObjectInbox, Readers) {
    let (session_sender, session_events) = mpsc::channel(SESSION_QUEUE);
    let (data_sender, data_events) = mpsc::channel(OBJECT_QUEUE);
    let (ended_sender, ended_aliases) = watch::channel(Vec::new());
    let data_sink = DataSink {
        events: data_sender,
        ended_aliases,
    };

    let accept = tokio::spawn(async move {
        let mut readers = JoinSet::new();
        let open_streams = Arc::new(Semaphore::new(MAX_UNI_STREAMS));
        let mut stream_number = 0u64;
        loop {
            let Ok(permit) = open_streams.clone().acquire_owned().await else {
                break;
            };
            let Ok(stream) = connection.accept_uni().await else {
                break;
            };
            while let Some(joined) = readers.try_join_next() {
                report_reader_panic(joined, &session_sender).await;
            }

            let label = format!("uni#{stream_number}");
            stream_number = stream_number.saturating_add(1);
            let sinks = Sinks {
                session: session_sender.clone(),
                data: data_sink.clone(),
            };
            readers.spawn(async move {
                let _permit = permit;
                let stream = TracedRecv::new(stream, trace, &label);
                if let Err(error) = read_uni_stream(stream, &sinks).await {
                    let _ = sinks.session.send(SessionEvent::Failed(error)).await;
                }
            });
        }
        // No further stream will come. The readers still hold what they already
        // read, so they get to finish and deliver it before this task ends.
        while let Some(joined) = readers.join_next().await {
            report_reader_panic(joined, &session_sender).await;
        }
    });

    (
        SessionInbox {
            events: session_events,
        },
        ObjectInbox {
            events: data_events,
            ended_aliases: ended_sender,
        },
        Readers(accept),
    )
}

/// A reader that panicked has lost a stream without saying so; the session is
/// told instead of carrying on with a hole in it.
async fn report_reader_panic(joined: Result<(), JoinError>, sink: &mpsc::Sender<SessionEvent>) {
    if let Err(error) = joined
        && error.is_panic()
    {
        let failed = SessionEvent::Failed(anyhow!("a stream reader panicked: {error}"));
        let _ = sink.send(failed).await;
    }
}

async fn read_uni_stream(mut stream: TracedRecv, sinks: &Sinks) -> Result<()> {
    // The first varint says what the stream is (§3.4).
    let mut head = Vec::new();
    let stream_type = loop {
        if let Some(stream_type) = uni_stream_type(&head)? {
            break stream_type;
        }
        match stream.read().await {
            Ok(Some(received)) => head.extend_from_slice(received),
            Ok(None) if head.is_empty() => return Ok(()),
            // The draft has no rule for a FIN inside the type; it is handled like
            // a stream that ends inside an Object (§11.4).
            Ok(None) => {
                return Err(MoqtError::ProtocolViolation("stream ended inside its type").into());
            }
            // §11.4.1: a stream cancelled before it said what it is changes
            // nothing for the session. §10.11 counts on streams being reset
            // "before the SUBGROUP_HEADER".
            Err(ReadFailure::Reset(code)) => {
                report::stream_reset(&stream.label, code);
                return Ok(());
            }
            // A lost connection is noticed where streams are accepted.
            Err(ReadFailure::Lost(_)) => return Ok(()),
        }
    };

    match stream_type {
        UniStreamType::Control => read_control_stream(stream, &head, &sinks.session).await,
        UniStreamType::Subgroup => read_subgroup_stream(stream, &head, &sinks.data).await,
        UniStreamType::Padding => {
            discard_padding_stream(stream).await;
            Ok(())
        }
    }
}

async fn read_control_stream(
    mut stream: TracedRecv,
    head: &[u8],
    sink: &mpsc::Sender<SessionEvent>,
) -> Result<()> {
    // §3.3: a control stream must not be closed during the session, and closing
    // it is a PROTOCOL_VIOLATION. A FIN and a reset both close it.
    let closed = || MoqtError::ProtocolViolation("the peer closed its control stream");

    let mut decoder = MessageDecoder::new();
    decoder.push(head)?;
    loop {
        while let Some(message) = decoder.next_message()? {
            deliver(sink, SessionEvent::Control(message)).await?;
        }
        match stream.read().await {
            Ok(Some(received)) => decoder.push(received)?,
            Ok(None) | Err(ReadFailure::Reset(_)) => return Err(closed().into()),
            // The connection itself is gone; there is no session left to close
            // with a code.
            Err(ReadFailure::Lost(error)) => return Err(error),
        }
    }
}

/// Why reading a subgroup stream stopped.
enum StreamEnd {
    Fin,
    Reset(u64),
    /// The connection is gone.
    Lost,
    /// The subscription the stream belongs to is over (§10.11).
    Unwanted,
    /// What the decoder made of the bytes. [`MoqtError::scope`] says whether
    /// that ends the stream, the subscription or the session.
    Refused(MoqtError),
    /// The client stopped taking Objects. The session ends.
    Fatal(anyhow::Error),
}

/// Reads one subgroup stream to its end and reports how it ended. Only a
/// protocol error ends the session: a reset is how streams are cancelled
/// (§11.4.1), and an oversized Object or a malformed track stops this stream.
async fn read_subgroup_stream(mut stream: TracedRecv, head: &[u8], sink: &DataSink) -> Result<()> {
    let mut decoder = SubgroupStreamDecoder::new(SUBGROUP_BUFFER_LIMIT);
    let end = decode_subgroup_stream(&mut stream, &mut decoder, head, sink).await;
    let track_alias = decoder.header().map(|header| header.track_alias);
    let label = stream.label.clone();

    let mut malformed = None;
    let mut fatal = None;
    match end {
        StreamEnd::Fin | StreamEnd::Lost => {}
        StreamEnd::Reset(code) => report::stream_reset(&label, code),
        StreamEnd::Unwanted => {
            stream.stop(stream_reset_code::CANCELLED);
            report::stream_stopped(&label, "the subscription ended");
        }
        StreamEnd::Refused(error) => match error.scope() {
            ErrorScope::Stream { reset_code } => {
                stream.stop(reset_code);
                report::stream_stopped(&label, &error);
            }
            ErrorScope::Subscription { reset_code } => {
                stream.stop(reset_code);
                report::stream_stopped(&label, &error);
                malformed = Some(error);
            }
            ErrorScope::Session(_) => fatal = Some(error.into()),
        },
        StreamEnd::Fatal(error) => fatal = Some(error),
    }

    if let Some(track_alias) = track_alias {
        if let Some(error) = malformed {
            deliver(
                &sink.events,
                DataEvent::TrackMalformed { track_alias, error },
            )
            .await?;
        }
        deliver(&sink.events, DataEvent::StreamClosed { track_alias }).await?;
    }
    fatal.map_or(Ok(()), Err)
}

async fn decode_subgroup_stream(
    stream: &mut TracedRecv,
    decoder: &mut SubgroupStreamDecoder,
    head: &[u8],
    sink: &DataSink,
) -> StreamEnd {
    if let Err(error) = decoder.push(head) {
        return StreamEnd::Refused(error);
    }
    let mut ended_aliases = sink.ended_aliases.clone();
    loop {
        loop {
            match decoder.next_event() {
                Ok(Some(SubgroupEvent::Header(_))) => {}
                Ok(Some(SubgroupEvent::Object(object))) => {
                    // The decoder yields the header before any Object.
                    let Some(header) = decoder.header().copied() else {
                        return StreamEnd::Fatal(anyhow!("an object came before its header"));
                    };
                    if let Err(error) =
                        deliver(&sink.events, DataEvent::Object(header, object)).await
                    {
                        return StreamEnd::Fatal(error);
                    }
                }
                Ok(None) => break,
                Err(error) => return StreamEnd::Refused(error),
            }
        }
        let unwanted = |aliases: &[u64]| {
            decoder
                .header()
                .is_some_and(|header| aliases.contains(&header.track_alias))
        };
        if unwanted(&ended_aliases.borrow_and_update()) {
            return StreamEnd::Unwanted;
        }
        let read = tokio::select! {
            read = stream.read() => read,
            // The list changed, or its sender is gone with the client; either
            // way the check above runs again before more is read.
            changed = ended_aliases.changed() => match changed {
                Ok(()) => continue,
                Err(_) => return StreamEnd::Lost,
            },
        };
        match read {
            Ok(Some(received)) => {
                if let Err(error) = decoder.push(received) {
                    return StreamEnd::Refused(error);
                }
            }
            Ok(None) => {
                return match decoder.finish() {
                    Ok(()) => StreamEnd::Fin,
                    Err(error) => StreamEnd::Refused(error),
                };
            }
            Err(ReadFailure::Reset(code)) => return StreamEnd::Reset(code),
            Err(ReadFailure::Lost(_)) => return StreamEnd::Lost,
        }
    }
}

/// §11.5.1: everything on a padding stream is discarded, so that it does not use
/// up flow control, and either side may cancel it without consequence.
async fn discard_padding_stream(mut stream: TracedRecv) {
    while let Ok(Some(_)) = stream.read().await {}
}

async fn deliver<T>(sink: &mpsc::Sender<T>, item: T) -> Result<()> {
    sink.send(item)
        .await
        .map_err(|_| anyhow!("the client stopped reading"))
}
