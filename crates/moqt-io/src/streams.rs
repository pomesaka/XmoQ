//! Reading and writing `MoQ` Transport streams on a [`Connection`].
//!
//! Section numbers refer to `draft-ietf-moq-transport-18`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio::time::Instant;
use xmoq_moqt::data::{
    SubgroupEvent, SubgroupHeader, SubgroupObject, SubgroupStreamDecoder, SubgroupStreamEncoder,
    UniStreamType, uni_stream_type,
};
use xmoq_moqt::fetch::{FetchHeader, FetchStreamEncoder, decode_fetch_header};
use xmoq_moqt::message::{ControlMessage, MessageDecoder};
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::{Error, ErrorScope, UsageError, stream_reset_code};

use crate::{Connection, ReadError, RecvStream, SendStream, WriteError};

/// Bytes asked of a stream in one read. Each stream that is being read holds a
/// buffer of this size.
const READ_CHUNK: usize = 16 * 1024;

/// Reads of a fetch stream that wait in the queue of its [`FetchBody`]. A
/// fetch stream nobody reads holds one more read than this, the one that
/// waits for room in the queue: five times [`READ_CHUNK`] at most, on top of
/// what the transport holds in the stream's receive window. Its sender is
/// held back by flow control.
const FETCH_BODY_CHUNKS: usize = 4;

/// The longest `FETCH_HEADER`: two variable-length integers of 9 bytes.
const FETCH_HEADER_MAX: usize = 18;

/// Which way the bytes of a trace line travelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Written by this endpoint.
    Send,
    /// Read by this endpoint.
    Recv,
}

type TraceFn = dyn Fn(Direction, &str, &[u8]) + Send + Sync;

/// Where the raw bytes of every write and read go, when someone wants them.
/// The byte fixtures in `fixtures/moqt-18/` were captured this way.
#[derive(Clone, Default)]
pub struct Trace(Option<Arc<TraceFn>>);

impl Trace {
    /// Passes every chunk to `sink` with its direction and the label of its
    /// stream.
    pub fn new(sink: impl Fn(Direction, &str, &[u8]) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(sink)))
    }

    /// Reports one write or one read. The functions of this crate call it for
    /// what they move; a caller that writes a stream itself calls it too, so
    /// that the trace shows every byte of the session.
    pub fn bytes(&self, direction: Direction, label: &str, bytes: &[u8]) {
        if let Some(sink) = &self.0 {
            sink(direction, label, bytes);
        }
    }
}

/// Limits and deadlines for the streams of one session.
///
/// Every value is provisional: the draft sets none of them, and `XmoQ` has not
/// decided any.
#[derive(Clone)]
pub struct StreamConfig {
    /// Unidirectional streams of the peer read at once. Further streams wait
    /// unaccepted, where QUIC flow control holds the peer back.
    pub max_uni_streams: usize,
    /// Request streams of the peer read at once.
    pub max_request_streams: usize,
    /// Bytes one subgroup stream may hold undecoded, which caps the Object size
    /// this endpoint accepts.
    ///
    /// FIXME: replace with the Object size limit once `XmoQ` decides one.
    pub subgroup_buffer_limit: usize,
    /// Undecoded bytes all subgroup streams of the session may hold together.
    /// A stream that would take the session over it is stopped.
    ///
    /// WHY on top of the limit above: that one allows every stream its own
    /// megabyte, 64 MiB for a session whose streams all hold a large, unfinished
    /// Object.
    pub session_buffer_limit: usize,
    /// How long a stream of the peer may take to say what it is: its stream
    /// type and Subgroup Header, or its first message.
    ///
    /// WHY: a stream that never says anything would hold one of the slots
    /// above for as long as the session lasts, and 64 of them would keep every
    /// later stream of the peer from being read.
    pub first_bytes_timeout: Duration,
    /// Where the raw bytes go.
    pub trace: Trace,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            max_uni_streams: 64,
            max_request_streams: 64,
            subgroup_buffer_limit: 1024 * 1024,
            session_buffer_limit: 4 * 1024 * 1024,
            first_bytes_timeout: Duration::from_secs(10),
            trace: Trace::default(),
        }
    }
}

/// What went wrong while sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// The message cannot be encoded: a bug of the caller.
    Usage(UsageError),
    /// The stream or the connection did not take it.
    Write(WriteError),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(error) => error.fmt(f),
            Self::Write(WriteError::Stopped(code)) => {
                write!(f, "the peer stopped the stream, code {code:?}")
            }
            Self::Write(WriteError::Lost(why)) => write!(f, "connection lost: {why}"),
        }
    }
}

impl std::error::Error for SendError {}

/// Encodes one control message and writes it.
///
/// # Errors
///
/// Fails when the message cannot be encoded or the stream does not take it.
pub async fn send_message<S: SendStream>(
    stream: &mut S,
    message: &ControlMessage,
    trace: &Trace,
    label: &str,
) -> Result<(), SendError> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes).map_err(SendError::Usage)?;
    trace.bytes(Direction::Send, label, &bytes);
    stream.write_all(&bytes).await.map_err(SendError::Write)
}

/// A fetch stream this endpoint writes (§11.4.4), after its `FETCH_HEADER`.
///
/// WHY a type of its own around the sending half: whoever writes a fetch
/// stream waits on its peer, and gives up after a while; the wait is a future
/// that is dropped then, and the half with it. What a peer sees of a half
/// that is merely dropped is up to the transport ([`SendStream`]), and with
/// quinn it is a FIN: a fetch stream cut short would read as one that came
/// to its end, with every Object after the cut "not existing" (§10.12.3).
/// This type resets the stream when it is dropped without having been
/// finished or reset.
pub struct FetchStreamWriter<S: SendStream> {
    /// `None` once the stream was finished or reset.
    send: Option<S>,
    /// What the stream is reset with when the writer is dropped.
    dropped_with: u32,
    trace: Trace,
    label: String,
}

impl<S: SendStream> FetchStreamWriter<S> {
    /// Writes bytes of the stream: Objects and End of Range indicators as
    /// §11.4.4 has them, or what another fetch stream carried. Waits while
    /// the peer does not take them.
    ///
    /// # Errors
    ///
    /// As [`SendStream::write_all`].
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
        let Some(send) = &mut self.send else {
            return Err(WriteError::Lost("the stream was ended".to_string()));
        };
        self.trace.bytes(Direction::Send, &self.label, bytes);
        send.write_all(bytes).await
    }

    /// Ends the stream with a FIN: everything of the fetch was written.
    pub fn finish(mut self) {
        if let Some(send) = self.send.take() {
            send.finish();
        }
    }

    /// Resets the stream with a Stream Reset Error Code (§3.3.3).
    pub fn reset(mut self, code: u32) {
        if let Some(send) = self.send.take() {
            send.reset(code);
        }
    }

    /// Completes when the peer sent `STOP_SENDING` or the connection is
    /// gone, as [`SendStream::stopped`].
    pub async fn stopped(&mut self) -> WriteError {
        match &mut self.send {
            Some(send) => send.stopped().await,
            None => std::future::pending().await,
        }
    }
}

impl<S: SendStream> Drop for FetchStreamWriter<S> {
    fn drop(&mut self) {
        if let Some(send) = self.send.take() {
            send.reset(self.dropped_with);
        }
    }
}

/// Opens a unidirectional stream for a fetch stream and writes nothing on
/// it: the caller writes the `FETCH_HEADER` first (§11.4.4). For a caller
/// that has to end the stream with a code it learns while the header waits
/// to be written, which [`open_fetch_stream`] cannot take. Dropping the call
/// leaves no stream behind; a writer that is dropped resets with
/// `CANCELLED`.
///
/// # Errors
///
/// Fails when the connection is gone.
pub async fn open_fetch_stream_without_header<C: Connection>(
    connection: &C,
    trace: &Trace,
    label: &str,
) -> Result<FetchStreamWriter<C::Send>, WriteError> {
    let send = connection
        .open_uni()
        .await
        .map_err(|lost| WriteError::Lost(lost.0))?;
    Ok(FetchStreamWriter {
        send: Some(send),
        dropped_with: stream_reset_code::CANCELLED,
        trace: trace.clone(),
        label: label.to_string(),
    })
}

/// Opens a unidirectional stream and writes the `FETCH_HEADER` that begins a
/// fetch stream (§11.4.4). Dropping the call while it waits resets what it
/// had opened, with `given_up`: the caller knows why it stopped waiting, as
/// for a peer that takes nothing. A writer that is dropped after the call
/// returned resets with `CANCELLED`.
///
/// # Errors
///
/// Fails when the connection is gone, and when the peer stops the stream
/// before the header is written.
pub async fn open_fetch_stream<C: Connection>(
    connection: &C,
    header: FetchHeader,
    trace: &Trace,
    label: &str,
    given_up: u32,
) -> Result<FetchStreamWriter<C::Send>, WriteError> {
    let mut writer = open_fetch_stream_without_header(connection, trace, label).await?;
    writer.dropped_with = given_up;
    // The Group Order has no part in the header.
    let bytes = FetchStreamEncoder::new(header, GroupOrder::Ascending).take_bytes();
    writer.write(&bytes).await?;
    writer.dropped_with = stream_reset_code::CANCELLED;
    Ok(writer)
}

/// Tells readers of this crate to stop reading a stream, by the number the
/// reader announced the stream with, and holds what a fetch stream carries
/// until someone takes it.
///
/// WHY a registry and not a list of unwanted Track Aliases: an entry lives
/// exactly as long as its reader, so nothing accumulates over a session, and
/// an alias that the publisher reuses is not caught by a stop meant for its
/// earlier use.
#[derive(Clone, Default)]
pub struct StreamStops {
    inner: Arc<Mutex<StopsInner>>,
}

#[derive(Default)]
struct StopsInner {
    next_stream: u64,
    signals: HashMap<u64, oneshot::Sender<u32>>,
    /// The bodies of fetch streams that were announced and not taken yet,
    /// each with what tells its reader that it left here.
    fetch_bodies: HashMap<u64, (FetchBody, oneshot::Sender<()>)>,
}

/// One reader's entry in a [`StreamStops`], removed when the reader ends.
struct Registration {
    stops: StreamStops,
    stream: u64,
    signal: oneshot::Receiver<u32>,
}

impl StreamStops {
    /// Sends `STOP_SENDING` with `code` on the stream, if it is still being
    /// read. Does nothing otherwise.
    pub fn stop(&self, stream: u64, code: u32) {
        let mut inner = self.lock();
        // Nobody is going to read it.
        inner.fetch_bodies.remove(&stream);
        if let Some(signal) = inner.signals.remove(&stream) {
            // The reader may have just ended; then nobody needs the signal.
            let _ = signal.send(code);
        }
    }

    /// What the fetch stream announced with [`UniEvent::FetchOpened`]
    /// carries after its header. `None` when it was taken before or the
    /// stream was stopped.
    ///
    /// Whoever gets a `FetchOpened` has to take the body or stop the stream:
    /// until then the stream holds one of the session's stream slots
    /// ([`StreamConfig::max_uni_streams`]), also after its last byte was
    /// read.
    #[must_use]
    pub fn take_fetch_body(&self, stream: u64) -> Option<FetchBody> {
        let (body, taken) = self.lock().fetch_bodies.remove(&stream)?;
        // The reader may have ended with a stop; then nobody waits for this.
        let _ = taken.send(());
        Some(body)
    }

    /// Streams that are being read, for tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().signals.len()
    }

    /// Whether no stream is being read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn register(&self) -> Registration {
        let (sender, signal) = oneshot::channel();
        let mut inner = self.lock();
        let stream = inner.next_stream;
        inner.next_stream = stream.saturating_add(1);
        inner.signals.insert(stream, sender);
        Registration {
            stops: self.clone(),
            stream,
            signal,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StopsInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut inner = self.stops.lock();
        inner.signals.remove(&self.stream);
        inner.fetch_bodies.remove(&self.stream);
    }
}

/// One piece of what a fetch stream carries after its `FETCH_HEADER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchChunk {
    /// Bytes of the stream, in order, cut wherever a read ended: Objects and
    /// End of Range indicators as §11.4.4 has them, not decoded.
    Bytes(Vec<u8>),
    /// The peer ended the stream, or the connection did.
    End(StreamEnd),
    /// This endpoint stopped reading the stream: with
    /// [`StreamStops::stop`], or by dropping [`IncomingStreams`]. Told apart
    /// from [`FetchChunk::End`] so that whoever passes the stream on can
    /// give the peer's reset code on where there was one, and its own where
    /// it stopped the stream itself.
    Stopped,
}

/// What a fetch stream of the peer carries after its header, read at the
/// pace of whoever holds this.
///
/// WHY bytes that are pulled, where a subgroup stream is decoded and its
/// Objects are put in the session's queue: a relay passes a fetch stream on
/// as it is, and may read it only as fast as the one it writes to takes it;
/// and decoding needs the Group Order of the FETCH, which the one who sent
/// the FETCH knows and this reader does not.
#[derive(Debug)]
pub struct FetchBody {
    chunks: mpsc::Receiver<FetchChunk>,
}

impl FetchBody {
    /// The next piece. After [`FetchChunk::End`] every call returns
    /// [`FetchChunk::Stopped`].
    ///
    /// Dropping the body stops the stream with `CANCELLED` and frees its
    /// stream slot. [`StreamStops::stop`] does the same with a code of the
    /// caller's.
    pub async fn next(&mut self) -> FetchChunk {
        self.chunks.recv().await.unwrap_or(FetchChunk::Stopped)
    }
}

/// What one read of a stream produced.
enum Step {
    Bytes(usize),
    Fin,
    Reset(Option<u32>),
    Lost,
    /// The owner of the stream asked to stop it, with this code.
    Stop(u32),
    /// The stream did not say what it is in time.
    TimedOut,
}

/// The receiving half of a stream with its read buffer, its trace label and
/// the signals that end reading early.
struct Reader<R> {
    recv: R,
    chunk: Vec<u8>,
    trace: Trace,
    label: String,
    registration: Registration,
    /// Set while the stream has not yet said what it is.
    deadline: Option<Instant>,
}

impl<R: RecvStream> Reader<R> {
    fn new(recv: R, config: &StreamConfig, stops: &StreamStops, kind: &str) -> Self {
        let registration = stops.register();
        Self {
            recv,
            chunk: vec![0; READ_CHUNK],
            trace: config.trace.clone(),
            label: format!("{kind}#{}", registration.stream),
            registration,
            deadline: Some(Instant::now() + config.first_bytes_timeout),
        }
    }

    fn stream(&self) -> u64 {
        self.registration.stream
    }

    async fn step(&mut self) -> Step {
        let deadline = self.deadline;
        let timed_out = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            code = &mut self.registration.signal => match code {
                Ok(code) => Step::Stop(code),
                // Does not happen: the sending end of the signal lives in the
                // registry, which this reader's registration keeps alive.
                Err(_) => Step::Lost,
            },
            read = self.recv.read(&mut self.chunk) => match read {
                Ok(Some(len)) => Step::Bytes(len),
                Ok(None) => Step::Fin,
                Err(ReadError::Reset(code)) => Step::Reset(code),
                Err(ReadError::Lost(_)) => Step::Lost,
            },
            () = timed_out => Step::TimedOut,
        }
    }

    /// The bytes of the last [`Step::Bytes`].
    fn received(&self, len: usize) -> &[u8] {
        // `read` reports at most the length of the buffer it was given.
        let received = self.chunk.get(..len).unwrap_or(&self.chunk);
        self.trace.bytes(Direction::Recv, &self.label, received);
        received
    }
}

/// What a request stream produced next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestStreamEvent {
    /// A message.
    Message(ControlMessage),
    /// The stream is over for this request (§3.3.2).
    Closed(RequestClose),
}

/// How a request stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestClose {
    /// The peer finished its direction.
    Fin,
    /// The peer reset its direction, with this code if it gave one.
    Reset(Option<u32>),
    /// The peer sent `STOP_SENDING` for this endpoint's direction, with this
    /// code if it gave one. [`RequestReader`] never reports it; whoever
    /// watches the sending half does.
    Stopped(Option<u32>),
}

impl std::fmt::Display for RequestClose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (what, code) = match self {
            Self::Fin => return f.write_str("FIN"),
            Self::Reset(code) => ("reset", code),
            Self::Stopped(code) => ("STOP_SENDING", code),
        };
        match code {
            Some(code) => write!(f, "{what}, code {code:#x}"),
            None => write!(f, "{what}, without an application code"),
        }
    }
}

/// Why a request stream could not be read further.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestReadError {
    /// The bytes break the protocol; the session has to be closed.
    Peer(Error),
    /// The connection is gone.
    Lost,
}

impl std::fmt::Display for RequestReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Peer(error) => error.fmt(f),
            Self::Lost => f.write_str("connection lost"),
        }
    }
}

impl std::error::Error for RequestReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Peer(error) => Some(error),
            Self::Lost => None,
        }
    }
}

/// The receiving half of a request stream this endpoint opened.
///
/// To cancel the request, call [`Self::stop`] and reset the sending half.
/// Merely dropping the two halves does not reset anything: with quinn 0.11.12,
/// which `web-transport-quinn` 0.13.2 wraps, a dropped sending half ends with a FIN and a
/// dropped receiving half sends `STOP_SENDING` with code 0 (`impl Drop for
/// SendStream` and `for RecvStream`; read 2026-10-10).
pub struct RequestReader<R> {
    reader: Reader<R>,
    decoder: MessageDecoder,
}

impl<R: RecvStream> RequestReader<R> {
    /// Wraps the receiving half of a request stream. `label` names it in the
    /// trace.
    pub fn new(recv: R, trace: &Trace, label: &str) -> Self {
        let config = StreamConfig {
            trace: trace.clone(),
            ..StreamConfig::default()
        };
        let mut reader = Reader::new(recv, &config, &StreamStops::default(), "request");
        reader.label = label.to_string();
        // How long to wait for a response is the caller's to decide.
        reader.deadline = None;
        Self {
            reader,
            decoder: MessageDecoder::new(),
        }
    }

    /// The next message, or how the peer ended the stream. Safe to drop while
    /// waiting, as `tokio::select!` does: received bytes stay in the decoder.
    ///
    /// # Errors
    ///
    /// A protocol error in the bytes, or a lost connection.
    pub async fn next_event(&mut self) -> Result<RequestStreamEvent, RequestReadError> {
        loop {
            if let Some(message) = self
                .decoder
                .next_message()
                .map_err(RequestReadError::Peer)?
            {
                return Ok(RequestStreamEvent::Message(message));
            }
            match self.reader.step().await {
                Step::Bytes(len) => {
                    let Self { reader, decoder } = self;
                    decoder
                        .push(reader.received(len))
                        .map_err(RequestReadError::Peer)?;
                }
                Step::Fin => {
                    self.decoder.finish().map_err(RequestReadError::Peer)?;
                    return Ok(RequestStreamEvent::Closed(RequestClose::Fin));
                }
                Step::Reset(code) => {
                    return Ok(RequestStreamEvent::Closed(RequestClose::Reset(code)));
                }
                // No deadline is set, and nobody else holds the stop registry.
                Step::Lost | Step::Stop(_) | Step::TimedOut => return Err(RequestReadError::Lost),
            }
        }
    }

    /// Tells the peer to stop sending on the stream.
    pub fn stop(self, code: u32) {
        self.reader.recv.stop(code);
    }
}

/// How a subgroup stream of the peer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// With a FIN.
    Fin,
    /// Without one: reset by the peer with this code, or stopped by this
    /// endpoint.
    Aborted(Option<u32>),
    /// Cut off by the end of the connection. The peer said nothing about the
    /// stream itself.
    Lost,
}

/// What the peer's unidirectional streams produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniEvent {
    /// A message on the peer's control stream.
    Control(ControlMessage),
    /// The Subgroup Header of a stream. `stream` names the stream in the later
    /// events and in [`StreamStops::stop`].
    SubgroupOpened {
        /// The stream.
        stream: u64,
        /// Its header.
        header: SubgroupHeader,
    },
    /// An Object.
    Object {
        /// The stream.
        stream: u64,
        /// The header of the stream.
        header: SubgroupHeader,
        /// The Object.
        object: SubgroupObject,
    },
    /// The track of a stream broke the object model (§2.4.2). The stream was
    /// stopped; [`UniEvent::SubgroupClosed`] follows. The subscription has to
    /// be cancelled; the session stays open.
    TrackMalformed {
        /// The stream.
        stream: u64,
        /// The header of the stream.
        header: SubgroupHeader,
        /// What was wrong.
        error: Error,
    },
    /// A subgroup stream ended. Sent for every stream that was announced with
    /// [`UniEvent::SubgroupOpened`].
    SubgroupClosed {
        /// The stream.
        stream: u64,
        /// The header of the stream.
        header: SubgroupHeader,
        /// How it ended.
        end: StreamEnd,
    },
    /// A fetch stream, after its `FETCH_HEADER`. What follows the header is
    /// not put in the queue: the consumer takes it with
    /// [`StreamStops::take_fetch_body`], or stops the stream.
    FetchOpened {
        /// The stream.
        stream: u64,
        /// Its header.
        header: FetchHeader,
    },
    /// A stream held something the session cannot go on after. The session has
    /// to be closed with [`Error::termination_code`].
    Failed(Error),
    /// A reader of this crate panicked and lost its stream. The session cannot
    /// go on with a hole in it; the fault is on this side (`INTERNAL_ERROR`).
    ReaderPanicked,
}

/// Readers of the peer's streams. Dropping it stops them.
pub struct IncomingStreams(JoinHandle<()>);

impl IncomingStreams {
    /// Completes when the readers are done: the connection is gone, and what
    /// had arrived on it before was read and put in the queue.
    pub async fn done(&mut self) {
        // An error means the task was aborted or panicked; either way it is
        // over.
        let _ = (&mut self.0).await;
    }
}

impl Drop for IncomingStreams {
    fn drop(&mut self) {
        // The accept task owns the per-stream tasks in a JoinSet, which aborts
        // them when it is dropped.
        self.0.abort();
    }
}

/// Where readers put what they read. `map` turns an event into the item of the
/// consumer's queue, so that one queue can take events of several kinds.
struct Sink<E, T> {
    queue: mpsc::Sender<T>,
    map: Arc<dyn Fn(E) -> T + Send + Sync>,
}

impl<E, T> Clone for Sink<E, T> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            map: Arc::clone(&self.map),
        }
    }
}

impl<E, T> Sink<E, T> {
    /// Waits for room in the queue, which is what holds a fast peer back.
    /// `false` when the consumer is gone.
    async fn put(&self, event: E) -> bool {
        self.queue.send((self.map)(event)).await.is_ok()
    }
}

/// Starts reading the unidirectional streams the peer opens: its control
/// stream, subgroup streams and padding streams.
///
/// `queue` is the consumer's bounded queue. Every reader waits on it, so a
/// peer that sends faster than the consumer takes is slowed by flow control
/// instead of growing memory.
///
/// FIXME: give connection flow control to the request streams before the
/// data streams (§11.4.2, MUST, "to prevent deadlocks"). With quinn's defaults
/// only per-stream windows fill up, because the connection-level receive
/// window is unlimited (`receive_window: VarInt::MAX` in
/// `TransportConfig::default`, quinn-proto 0.11.19; read 2026-10-10). A
/// server that bounds that window, as the relay does, can have it taken up by
/// subgroup streams whose readers wait here, and the request streams of the
/// same session then wait as well. The deadlock the draft names does not
/// arise here: a stream whose Track Alias is unknown is read and held within
/// limits, not left unread until its PUBLISH comes.
pub fn read_uni_streams<C: Connection, T: Send + 'static>(
    connection: Arc<C>,
    config: StreamConfig,
    stops: StreamStops,
    queue: mpsc::Sender<T>,
    map: impl Fn(UniEvent) -> T + Send + Sync + 'static,
) -> IncomingStreams {
    let sink = Sink {
        queue,
        map: Arc::new(map),
    };
    let limit = config.max_uni_streams;
    let budget = Arc::new(ReceiveBudget {
        limit: config.session_buffer_limit,
        used: AtomicUsize::new(0),
    });
    accept_streams(
        limit,
        sink.clone(),
        || UniEvent::ReaderPanicked,
        move |permit| {
            let connection = Arc::clone(&connection);
            let config = config.clone();
            let stops = stops.clone();
            let sink = sink.clone();
            let budget = Arc::clone(&budget);
            async move {
                let recv = connection.accept_uni().await.ok()?;
                let reader = Reader::new(recv, &config, &stops, "uni");
                let read: ReaderTask = Box::pin(async move {
                    let _permit = permit;
                    read_uni_stream(reader, &config, &budget, &sink).await;
                });
                Some(read)
            }
        },
    )
}

/// How the reading of a request stream the peer opened ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerRequestClose {
    /// With a FIN, after whole messages. A requester that only finishes its
    /// direction has not cancelled the request (§3.3.2 names a reset and
    /// `STOP_SENDING` for that).
    Fin,
    /// The peer reset its direction, with this code when it gave one that
    /// can be read.
    Reset(Option<u32>),
    /// This endpoint stopped the stream, or the connection is gone.
    Abandoned,
}

impl PeerRequestClose {
    /// Whether it was a FIN.
    #[must_use]
    pub fn is_fin(self) -> bool {
        self == Self::Fin
    }
}

/// What the request streams the peer opens produced.
pub enum PeerRequestStreamEvent<S> {
    /// The first message of a new stream, with the half to answer on. `stream`
    /// names the stream in the later events and in [`StreamStops::stop`].
    Opened {
        /// The stream.
        stream: u64,
        /// The sending half.
        send: S,
        /// The message.
        first: ControlMessage,
    },
    /// A later message.
    Message {
        /// The stream.
        stream: u64,
        /// The message.
        message: ControlMessage,
    },
    /// Nothing more is read from the stream: the peer ended its direction, by
    /// FIN or by reset, this endpoint stopped it, or the connection is gone.
    /// Sent for every stream that was announced with `Opened`.
    Closed {
        /// The stream.
        stream: u64,
        /// How.
        how: PeerRequestClose,
    },
    /// A stream held something the session cannot go on after.
    Failed(Error),
    /// A reader of this crate panicked and lost its stream; see
    /// [`UniEvent::ReaderPanicked`].
    ReaderPanicked,
}

/// Starts reading the bidirectional streams the peer opens, which carry its
/// requests (§3.3).
pub fn read_request_streams<C: Connection, T: Send + 'static>(
    connection: Arc<C>,
    config: StreamConfig,
    stops: StreamStops,
    queue: mpsc::Sender<T>,
    map: impl Fn(PeerRequestStreamEvent<C::Send>) -> T + Send + Sync + 'static,
) -> IncomingStreams {
    let sink = Sink {
        queue,
        map: Arc::new(map),
    };
    let limit = config.max_request_streams;
    accept_streams(
        limit,
        sink.clone(),
        || PeerRequestStreamEvent::ReaderPanicked,
        move |permit| {
            let connection = Arc::clone(&connection);
            let config = config.clone();
            let stops = stops.clone();
            let sink = sink.clone();
            async move {
                let (send, recv) = connection.accept_bi().await.ok()?;
                let reader = Reader::new(recv, &config, &stops, "bi");
                let read: ReaderTask = Box::pin(async move {
                    let _permit = permit;
                    read_request_stream(reader, send, &sink).await;
                });
                Some(read)
            }
        },
    )
}

/// The work of reading one accepted stream to its end.
type ReaderTask = std::pin::Pin<Box<dyn Future<Output = ()> + Send>>;

/// Accepts streams until the connection is gone, at most `limit` at a time.
/// `accept` takes one stream and returns the task that reads it, or `None`
/// when the connection is gone.
fn accept_streams<E, T, A, F>(
    limit: usize,
    sink: Sink<E, T>,
    panicked: fn() -> E,
    mut accept: A,
) -> IncomingStreams
where
    E: Send + 'static,
    T: Send + 'static,
    A: FnMut(tokio::sync::OwnedSemaphorePermit) -> F + Send + 'static,
    F: Future<Output = Option<ReaderTask>> + Send,
{
    IncomingStreams(tokio::spawn(async move {
        let mut readers = JoinSet::new();
        let slots = Arc::new(Semaphore::new(limit));
        loop {
            let Ok(permit) = Arc::clone(&slots).acquire_owned().await else {
                break;
            };
            let Some(read) = accept(permit).await else {
                break;
            };
            readers.spawn(read);
            while let Some(joined) = readers.try_join_next() {
                report_panic(joined, &sink, panicked).await;
            }
        }
        // No further stream will come. The readers still hold what they already
        // read, so they get to finish and deliver it before this task ends.
        while let Some(joined) = readers.join_next().await {
            report_panic(joined, &sink, panicked).await;
        }
    }))
}

/// A reader that panicked has lost a stream without saying so; the session is
/// told instead of carrying on with a hole in it.
async fn report_panic<E, T>(joined: Result<(), JoinError>, sink: &Sink<E, T>, panicked: fn() -> E) {
    if joined.is_err_and(|error| error.is_panic()) {
        sink.put(panicked()).await;
    }
}

/// The undecoded bytes the subgroup streams of one session hold together.
struct ReceiveBudget {
    limit: usize,
    used: AtomicUsize,
}

/// What one stream holds of its session's [`ReceiveBudget`]. Gives it back when
/// dropped.
struct Held<'a> {
    budget: &'a ReceiveBudget,
    bytes: usize,
}

impl Held<'_> {
    /// Records that the stream now holds `bytes`. `false` when that takes the
    /// session over its limit.
    fn set(&mut self, bytes: usize) -> bool {
        // Relaxed: a budget, not a lock.
        let used = if bytes >= self.bytes {
            let more = bytes - self.bytes;
            self.budget.used.fetch_add(more, Ordering::Relaxed) + more
        } else {
            let less = self.bytes - bytes;
            self.budget.used.fetch_sub(less, Ordering::Relaxed) - less
        };
        self.bytes = bytes;
        used <= self.budget.limit
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

async fn read_uni_stream<R: RecvStream, T>(
    mut reader: Reader<R>,
    config: &StreamConfig,
    budget: &ReceiveBudget,
    sink: &Sink<UniEvent, T>,
) {
    // The first varint says what the stream is (§3.4).
    let mut head = Vec::new();
    let stream_type = loop {
        match uni_stream_type(&head) {
            Ok(Some(stream_type)) => break stream_type,
            Ok(None) => {}
            Err(error) => {
                sink.put(UniEvent::Failed(error)).await;
                return;
            }
        }
        match reader.step().await {
            Step::Bytes(len) => head.extend_from_slice(reader.received(len)),
            // The draft has no rule for a FIN inside the type; it is handled
            // like a stream that ends inside an Object (§11.4).
            Step::Fin if !head.is_empty() => {
                let error = Error::ProtocolViolation("stream ended inside its type");
                sink.put(UniEvent::Failed(error)).await;
                return;
            }
            // §11.4.1: a stream cancelled before it said what it is changes
            // nothing for the session. §10.11 counts on streams being reset
            // "before the SUBGROUP_HEADER". A lost connection is noticed where
            // streams are accepted.
            Step::Fin | Step::Reset(_) | Step::Lost => return,
            Step::Stop(code) => {
                reader.recv.stop(code);
                return;
            }
            Step::TimedOut => {
                reader.recv.stop(stream_reset_code::INTERNAL_ERROR);
                return;
            }
        }
    };

    match stream_type {
        UniStreamType::Control => read_control_stream(reader, &head, sink).await,
        UniStreamType::Subgroup => {
            read_subgroup_stream(reader, &head, config, budget, sink).await;
        }
        UniStreamType::Fetch => read_fetch_stream(reader, head, sink).await,
        // §11.5.1: everything on a padding stream is discarded, so that it
        // does not use up flow control, and either side may cancel it without
        // consequence.
        UniStreamType::Padding => {
            reader.deadline = None;
            while let Step::Bytes(_) = reader.step().await {}
        }
    }
}

async fn read_control_stream<R: RecvStream, T>(
    mut reader: Reader<R>,
    head: &[u8],
    sink: &Sink<UniEvent, T>,
) {
    // §3.3: the control stream stays open for the whole session.
    reader.deadline = None;
    let mut decoder = MessageDecoder::new();
    let mut pushed = decoder.push(head);
    loop {
        if let Err(error) = pushed {
            sink.put(UniEvent::Failed(error)).await;
            return;
        }
        loop {
            match decoder.next_message() {
                Ok(Some(message)) => {
                    if !sink.put(UniEvent::Control(message)).await {
                        return;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    sink.put(UniEvent::Failed(error)).await;
                    return;
                }
            }
        }
        pushed = match reader.step().await {
            Step::Bytes(len) => decoder.push(reader.received(len)),
            // §3.3: a control stream must not be closed during the session,
            // and closing it is a PROTOCOL_VIOLATION. A FIN and a reset both
            // close it.
            Step::Fin | Step::Reset(_) => Err(Error::ProtocolViolation(
                "the peer closed its control stream",
            )),
            // The connection itself is gone; there is no session left to close
            // with a code. Nobody stops a control stream, and it has no
            // deadline.
            Step::Lost | Step::Stop(_) | Step::TimedOut => return,
        };
    }
}

/// Reads the header of a fetch stream, announces the stream, and then moves
/// its bytes to whoever takes the [`FetchBody`], waiting for them.
async fn read_fetch_stream<R: RecvStream, T>(
    mut reader: Reader<R>,
    mut head: Vec<u8>,
    sink: &Sink<UniEvent, T>,
) {
    let (header, used) = loop {
        match decode_fetch_header(&head) {
            Ok(Some(decoded)) => break decoded,
            // Two variable-length integers; more bytes than they can take
            // without a header is not reached, and bounds `head` if it were.
            Ok(None) if head.len() < FETCH_HEADER_MAX => {}
            Ok(None) => return,
            Err(error) => {
                sink.put(UniEvent::Failed(error)).await;
                return;
            }
        }
        match reader.step().await {
            Step::Bytes(len) => head.extend_from_slice(reader.received(len)),
            // §10.12.3: a fetch without Objects still has "the FETCH_HEADER"
            // before its FIN.
            Step::Fin => {
                let error = Error::ProtocolViolation("stream ended before its FETCH_HEADER");
                sink.put(UniEvent::Failed(error)).await;
                return;
            }
            // As for a stream cancelled before it said what it is (§11.4.1).
            Step::Reset(_) | Step::Lost => return,
            Step::Stop(code) => {
                reader.recv.stop(code);
                return;
            }
            Step::TimedOut => {
                reader.recv.stop(stream_reset_code::INTERNAL_ERROR);
                return;
            }
        }
    };
    // The stream has said what it is; from here on, how long it stays open
    // is the business of the two endpoints.
    reader.deadline = None;
    let stream = reader.stream();
    let (chunks, body) = mpsc::channel(FETCH_BODY_CHUNKS);
    let (taken, left_the_registry) = oneshot::channel();
    reader
        .registration
        .stops
        .lock()
        .fetch_bodies
        .insert(stream, (FetchBody { chunks: body }, taken));
    if !sink.put(UniEvent::FetchOpened { stream, header }).await {
        return;
    }

    let rest = head.get(used..).unwrap_or_default().to_vec();
    let mut unsent = (!rest.is_empty()).then_some(rest);
    // How the stream ended, or `None` when a stop ended it: the one who asked
    // for the stop has given up the body with it.
    let end = loop {
        if let Some(bytes) = unsent.take() {
            // Waits for room, which is what holds the sender of the stream
            // back, and for a stop, which ends the wait.
            let sent = tokio::select! {
                biased;
                code = &mut reader.registration.signal => Err(code.ok()),
                sent = chunks.send(FetchChunk::Bytes(bytes)) => match sent {
                    Ok(()) => Ok(()),
                    // The body was dropped: nobody wants the rest (§5.2 lets
                    // a subscriber stop the data stream of a FETCH it
                    // cancels).
                    Err(_) => Err(Some(stream_reset_code::CANCELLED)),
                },
            };
            if let Err(code) = sent {
                if let Some(code) = code {
                    reader.recv.stop(code);
                }
                return;
            }
        }
        // WHY the body is watched while the stream is: a body that is
        // dropped while the peer sends nothing would otherwise keep the
        // stream, and one of the session's stream slots, until the peer's
        // next byte, which the peer need never send. WHY after the step in
        // the order: a stop takes the body out of the registry and drops it
        // too, and the stop's code is the one to use then. Dropping a read
        // loses nothing (contract of `RecvStream`).
        let step = tokio::select! {
            biased;
            step = reader.step() => step,
            () = chunks.closed() => Step::Stop(stream_reset_code::CANCELLED),
        };
        match step {
            Step::Bytes(len) => unsent = Some(reader.received(len).to_vec()),
            Step::Fin => break StreamEnd::Fin,
            Step::Reset(code) => break StreamEnd::Aborted(code),
            Step::Lost => break StreamEnd::Lost,
            Step::Stop(code) => {
                reader.recv.stop(code);
                return;
            }
            // The deadline was cleared above.
            Step::TimedOut => return,
        }
    };
    // WHY wait once the end is passed on: the body may not be taken yet, and
    // it is in the registry only as long as this reader's registration is.
    // A stop takes it out of the registry as well, which ends the wait.
    tokio::select! {
        biased;
        _ = &mut reader.registration.signal => {}
        () = async {
            if chunks.send(FetchChunk::End(end)).await.is_ok() {
                let _ = left_the_registry.await;
            }
        } => {}
    }
}

/// Reads one subgroup stream to its end and reports how it ended. Only a
/// protocol error ends the session: a reset is how streams are cancelled
/// (§11.4.1), and an oversized Object or a malformed track stops this stream.
async fn read_subgroup_stream<R: RecvStream, T>(
    mut reader: Reader<R>,
    head: &[u8],
    config: &StreamConfig,
    budget: &ReceiveBudget,
    sink: &Sink<UniEvent, T>,
) {
    let stream = reader.stream();
    let mut decoder = SubgroupStreamDecoder::new(config.subgroup_buffer_limit);
    let mut undecoded = Held { budget, bytes: 0 };
    let mut pushed = decoder.push(head);
    // What ends the stream without a FIN, and the code to stop it with.
    let aborted = loop {
        let delivered = match pushed {
            Ok(()) => deliver_decoded(&mut decoder, &mut reader, sink).await,
            Err(error) => Err(Some(error)),
        };
        match delivered {
            Ok(()) => {}
            // The consumer is gone.
            Err(None) => return,
            Err(Some(error)) => break Some(error),
        }
        // What is left in the decoder is the start of an Object that has not
        // arrived in full. The session as a whole may hold only so much of
        // that; the stream that goes over is stopped like one whose Object is
        // too large.
        if !undecoded.set(decoder.buffered_len()) {
            break Some(Error::ObjectTooLarge {
                limit: budget.limit,
            });
        }
        pushed = match reader.step().await {
            Step::Bytes(len) => decoder.push(reader.received(len)),
            Step::Fin => match decoder.finish() {
                Ok(()) => {
                    close_subgroup(&decoder, stream, StreamEnd::Fin, sink).await;
                    return;
                }
                Err(error) => Err(error),
            },
            Step::Reset(code) => {
                close_subgroup(&decoder, stream, StreamEnd::Aborted(code), sink).await;
                return;
            }
            Step::Lost => {
                close_subgroup(&decoder, stream, StreamEnd::Lost, sink).await;
                return;
            }
            Step::Stop(code) => {
                reader.recv.stop(code);
                close_subgroup(&decoder, stream, StreamEnd::Aborted(None), sink).await;
                return;
            }
            Step::TimedOut => {
                reader.recv.stop(stream_reset_code::INTERNAL_ERROR);
                return;
            }
        };
    };

    let Some(error) = aborted else { return };
    match error.scope() {
        ErrorScope::Stream { reset_code } => reader.recv.stop(reset_code),
        ErrorScope::Subscription { reset_code } => {
            reader.recv.stop(reset_code);
            if let Some(header) = decoder.header().copied() {
                let malformed = UniEvent::TrackMalformed {
                    stream,
                    header,
                    error,
                };
                if !sink.put(malformed).await {
                    return;
                }
            }
        }
        ErrorScope::Session(_) => {
            sink.put(UniEvent::Failed(error)).await;
            return;
        }
    }
    close_subgroup(&decoder, stream, StreamEnd::Aborted(None), sink).await;
}

/// Passes on what the decoder holds. `Err(None)` means the consumer is gone,
/// `Err(Some(_))` is what the decoder made of the bytes.
async fn deliver_decoded<R: RecvStream, T>(
    decoder: &mut SubgroupStreamDecoder,
    reader: &mut Reader<R>,
    sink: &Sink<UniEvent, T>,
) -> Result<(), Option<Error>> {
    let stream = reader.stream();
    loop {
        let event = match decoder.next_event().map_err(Some)? {
            Some(SubgroupEvent::Header(header)) => {
                // The stream has said what it is; from here on, how long it
                // stays open is the publisher's business.
                reader.deadline = None;
                UniEvent::SubgroupOpened { stream, header }
            }
            Some(SubgroupEvent::Object(object)) => {
                // The decoder yields the header before any Object.
                let Some(header) = decoder.header().copied() else {
                    return Ok(());
                };
                UniEvent::Object {
                    stream,
                    header,
                    object,
                }
            }
            None => return Ok(()),
        };
        if !sink.put(event).await {
            return Err(None);
        }
    }
}

/// Reports the end of a stream that was announced, which is one whose header
/// was decoded.
async fn close_subgroup<T>(
    decoder: &SubgroupStreamDecoder,
    stream: u64,
    end: StreamEnd,
    sink: &Sink<UniEvent, T>,
) {
    if let Some(header) = decoder.header().copied() {
        sink.put(UniEvent::SubgroupClosed {
            stream,
            header,
            end,
        })
        .await;
    }
}

async fn read_request_stream<R: RecvStream, S: SendStream, T>(
    mut reader: Reader<R>,
    send: S,
    sink: &Sink<PeerRequestStreamEvent<S>, T>,
) {
    let stream = reader.stream();
    let mut decoder = MessageDecoder::new();
    // Handed to the consumer together with the first message.
    let mut send = Some(send);
    let mut how = PeerRequestClose::Abandoned;
    let failed = loop {
        loop {
            let message = match decoder.next_message() {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) => {
                    sink.put(PeerRequestStreamEvent::Failed(error)).await;
                    return;
                }
            };
            let event = match send.take() {
                Some(send) => {
                    // The request has begun; how long it stays open is the
                    // business of the two endpoints.
                    reader.deadline = None;
                    PeerRequestStreamEvent::Opened {
                        stream,
                        send,
                        first: message,
                    }
                }
                None => PeerRequestStreamEvent::Message { stream, message },
            };
            if !sink.put(event).await {
                return;
            }
        }
        let pushed = match reader.step().await {
            Step::Bytes(len) => decoder.push(reader.received(len)),
            Step::Fin => match decoder.finish() {
                Ok(()) => {
                    how = PeerRequestClose::Fin;
                    break None;
                }
                Err(error) => Err(error),
            },
            Step::Reset(code) => {
                how = PeerRequestClose::Reset(code);
                break None;
            }
            Step::Lost => break None,
            Step::Stop(code) => {
                reader.recv.stop(code);
                break None;
            }
            Step::TimedOut => {
                // A stream that never sent its request is cancelled (§3.3.2).
                reader.recv.stop(stream_reset_code::CANCELLED);
                if let Some(send) = send.take() {
                    send.reset(stream_reset_code::CANCELLED);
                }
                return;
            }
        };
        if let Err(error) = pushed {
            break Some(error);
        }
    };
    match failed {
        Some(error) => {
            sink.put(PeerRequestStreamEvent::Failed(error)).await;
        }
        // Only a stream that was announced is reported closed.
        None if send.is_none() => {
            sink.put(PeerRequestStreamEvent::Closed { stream, how })
                .await;
        }
        None => {}
    }
}

/// The send budget of a receiver is used up: it does not keep up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

/// How much may wait to be written to one receiver, over all its subgroup
/// streams.
///
/// WHY one budget for the receiver and not one for each stream: a receiver
/// that stops reading while the publisher keeps starting short Subgroups (a
/// Group for each key frame, say) never fills a single stream, and a budget
/// for each stream would let the streams pile up without end.
/// WHY bytes and not only a count of Objects: Objects arrive in bursts. One
/// read of a publisher's stream can decode hundreds of small Objects at once,
/// which a small count would mistake for a receiver that fell behind, while a
/// large count would let large Objects pile up.
///
/// Every value is provisional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendLimits {
    /// Payload bytes that are not written yet. One Object is always taken,
    /// whatever its size.
    pub bytes: usize,
    /// Objects that are not written yet.
    pub objects: usize,
    /// Subgroup streams that are not done yet: not opened, not written out, or
    /// waiting for the peer's stream limit.
    pub streams: usize,
}

impl Default for SendLimits {
    fn default() -> Self {
        Self {
            bytes: 4 * 1024 * 1024,
            objects: 4096,
            streams: 256,
        }
    }
}

/// What waits to be written to one receiver. Shared by all its
/// [`OutgoingSubgroup`]s.
#[derive(Debug)]
pub struct SendBudget {
    limits: SendLimits,
    bytes: AtomicUsize,
    objects: AtomicUsize,
    streams: AtomicUsize,
}

impl SendBudget {
    /// An unused budget.
    #[must_use]
    pub fn new(limits: SendLimits) -> Arc<Self> {
        Arc::new(Self {
            limits,
            bytes: AtomicUsize::new(0),
            objects: AtomicUsize::new(0),
            streams: AtomicUsize::new(0),
        })
    }

    /// `(bytes, objects, streams)` that wait, for tests and logs.
    #[must_use]
    pub fn in_use(&self) -> (usize, usize, usize) {
        // Relaxed throughout this type: the counters are a budget, not a lock.
        (
            self.bytes.load(Ordering::Relaxed),
            self.objects.load(Ordering::Relaxed),
            self.streams.load(Ordering::Relaxed),
        )
    }
}

/// What one writer holds of its receiver's budget.
struct Share {
    budget: Arc<SendBudget>,
    bytes: AtomicUsize,
    objects: AtomicUsize,
    /// The writer is done and gave everything back. Its handle may live on;
    /// what is sent through it after that is dropped and counts for nothing.
    released: AtomicBool,
}

impl Share {
    fn take(&self, bytes: usize) {
        if self.released.load(Ordering::Relaxed) {
            return;
        }
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.objects.fetch_add(1, Ordering::Relaxed);
        self.budget.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.budget.objects.fetch_add(1, Ordering::Relaxed);
    }

    fn give_back(&self, bytes: usize) {
        self.bytes.fetch_sub(bytes, Ordering::Relaxed);
        self.objects.fetch_sub(1, Ordering::Relaxed);
        self.budget.bytes.fetch_sub(bytes, Ordering::Relaxed);
        self.budget.objects.fetch_sub(1, Ordering::Relaxed);
    }

    /// Gives back whatever the writer still holds, and its stream. Called
    /// once, when the writer ends, however it ends.
    fn release(&self) {
        self.released.store(true, Ordering::Relaxed);
        let budget = &self.budget;
        budget
            .bytes
            .fetch_sub(self.bytes.swap(0, Ordering::Relaxed), Ordering::Relaxed);
        budget
            .objects
            .fetch_sub(self.objects.swap(0, Ordering::Relaxed), Ordering::Relaxed);
        budget.streams.fetch_sub(1, Ordering::Relaxed);
    }
}

/// What became of a subgroup stream this endpoint writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutgoingEvent {
    /// The stream was opened. From here on it counts as a stream of the
    /// subscription (§10.11, Stream Count).
    Opened,
    /// The peer sent `STOP_SENDING` with this code, which a write ran into.
    /// The writer resets the stream in answer (§11.4.3). A stop that arrives
    /// while nothing is being written is not seen until the next write.
    Stopped(Option<u32>),
    /// The writer is done: the stream was finished, reset, or never opened.
    /// Reported once for every writer, and last.
    Closed,
}

enum Command {
    Object(Arc<SubgroupObject>),
    Finish,
}

/// A subgroup stream this endpoint writes, fed through a queue.
///
/// WHY a queue: the one who decides what to send must not wait for a slow
/// receiver. When the receiver's [`SendBudget`] is used up it is told so, and
/// decides what to do about the subscription.
pub struct OutgoingSubgroup {
    queue: mpsc::UnboundedSender<Command>,
    share: Arc<Share>,
    reset: watch::Sender<Option<u32>>,
}

impl OutgoingSubgroup {
    /// Opens a unidirectional stream, writes `header` and then what is queued.
    /// `report` is called from the writer with what became of the stream; it
    /// may wait. `label` names the stream in the trace.
    ///
    /// # Errors
    ///
    /// [`QueueFull`] when the receiver already has as many unfinished streams
    /// as its budget allows. Nothing was started, and `report` is not called.
    pub fn spawn<C, R, F>(
        connection: Arc<C>,
        header: SubgroupHeader,
        budget: &Arc<SendBudget>,
        report: R,
        trace: Trace,
        label: String,
    ) -> Result<Self, QueueFull>
    where
        C: Connection,
        R: Fn(OutgoingEvent) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send,
    {
        let streams = budget.streams.fetch_add(1, Ordering::Relaxed);
        if streams >= budget.limits.streams {
            budget.streams.fetch_sub(1, Ordering::Relaxed);
            return Err(QueueFull);
        }
        let share = Arc::new(Share {
            budget: Arc::clone(budget),
            bytes: AtomicUsize::new(0),
            objects: AtomicUsize::new(0),
            released: AtomicBool::new(false),
        });
        // The channel has no bound of its own; `send` keeps the budget.
        let (queue, commands) = mpsc::unbounded_channel();
        let (reset, reset_signal) = watch::channel(None);
        let writer = Writer {
            share: Arc::clone(&share),
            commands,
            reset: reset_signal,
            trace,
            label,
        };
        let done = Arc::clone(&share);
        tokio::spawn(async move {
            writer.run(connection, header, &report).await;
            done.release();
            report(OutgoingEvent::Closed).await;
        });
        Ok(Self {
            queue,
            share,
            reset,
        })
    }

    /// Queues an Object.
    ///
    /// # Errors
    ///
    /// [`QueueFull`] when the Object would take the receiver over its budget.
    /// The Object is not queued.
    pub fn send(&self, object: Arc<SubgroupObject>) -> Result<(), QueueFull> {
        let budget = &self.share.budget;
        let (bytes, objects, _) = budget.in_use();
        let len = object.payload.len();
        let over = objects >= budget.limits.objects
            || (objects > 0 && bytes.saturating_add(len) > budget.limits.bytes);
        if over {
            return Err(QueueFull);
        }
        self.share.take(len);
        // An error means the writer is gone, with its share of the budget.
        let _ = self.queue.send(Command::Object(object));
        Ok(())
    }

    /// Ends the stream with a FIN after what is queued (§11.4.3).
    pub fn finish(&self) {
        let _ = self.queue.send(Command::Finish);
    }

    /// Resets the stream with a Stream Reset Error Code, dropping what is
    /// queued. Works after [`Self::finish`] too, for as long as the writer has
    /// not written everything out.
    pub fn reset(&self, code: u32) {
        self.reset.send_replace(Some(code));
    }
}

struct Writer {
    share: Arc<Share>,
    commands: mpsc::UnboundedReceiver<Command>,
    reset: watch::Receiver<Option<u32>>,
    trace: Trace,
    label: String,
}

impl Writer {
    /// Completes with the code once a reset is asked for.
    ///
    /// WHY never when the handle is merely dropped: a handle may be dropped
    /// right after `finish`, with Objects still queued. Those have to go out.
    /// A handle dropped without `finish` shows as the end of the queue instead.
    fn reset_asked(&self) -> impl Future<Output = u32> + Send + use<> {
        let mut reset = self.reset.clone();
        async move {
            let asked = reset
                .wait_for(Option::is_some)
                .await
                .ok()
                .and_then(|code| *code);
            match asked {
                Some(code) => code,
                None => std::future::pending().await,
            }
        }
    }

    async fn run<C, R, F>(mut self, connection: Arc<C>, header: SubgroupHeader, report: &R)
    where
        C: Connection,
        R: Fn(OutgoingEvent) -> F + Send + Sync,
        F: Future<Output = ()> + Send,
    {
        let stream = tokio::select! {
            biased;
            _ = self.reset_asked() => return,
            stream = connection.open_uni() => stream,
        };
        let Ok(send) = stream else { return };
        report(OutgoingEvent::Opened).await;

        let mut encoder = SubgroupStreamEncoder::new(&header);
        // The payload lengths of the Objects in `encoder` that are not written.
        let mut unwritten = Vec::new();
        // `None` once a write failed: the peer has stopped the stream or is
        // gone. What is queued after that is taken and dropped, so that it
        // does not count against the receiver's budget and get it cut off as
        // too slow.
        let mut send = Some(send);
        loop {
            let bytes = encoder.take_bytes();
            if let Some(open) = &mut send
                && !bytes.is_empty()
            {
                self.trace.bytes(Direction::Send, &self.label, &bytes);
                let cut_short = tokio::select! {
                    biased;
                    code = self.reset_asked() => Some(code),
                    written = open.write_all(&bytes) => match written {
                        Ok(()) => None,
                        // §11.4.3: "The publisher SHOULD respond with a reset"
                        // to a STOP_SENDING.
                        Err(WriteError::Stopped(code)) => {
                            report(OutgoingEvent::Stopped(code)).await;
                            Some(stream_reset_code::CANCELLED)
                        }
                        // Nothing more can be written or reset, and nobody
                        // may be left to end this writer through its handle.
                        Err(WriteError::Lost(_)) => return,
                    },
                };
                if let Some(code) = cut_short
                    && let Some(open) = send.take()
                {
                    open.reset(code);
                }
            }
            for len in unwritten.drain(..) {
                self.share.give_back(len);
            }
            let command = tokio::select! {
                biased;
                code = self.reset_asked() => {
                    if let Some(open) = send {
                        open.reset(code);
                    }
                    return;
                }
                command = self.commands.recv() => command,
            };
            match command {
                Some(Command::Object(object)) => {
                    let len = object.payload.len();
                    if send.is_some() && encoder.object(&object).is_ok() {
                        unwritten.push(len);
                        continue;
                    }
                    self.share.give_back(len);
                    // An Object the encoder refuses cannot follow the ones
                    // already written; the Subgroup is cut short, which calls
                    // for a reset (§11.4.3).
                    if let Some(open) = send.take() {
                        open.reset(stream_reset_code::INTERNAL_ERROR);
                    }
                }
                Some(Command::Finish) => {
                    if let Some(open) = send {
                        open.finish();
                    }
                    return;
                }
                // The handle is gone without `finish`: the Subgroup is cut
                // short.
                None => {
                    if let Some(open) = send {
                        open.reset(stream_reset_code::CANCELLED);
                    }
                    return;
                }
            }
        }
    }
}
