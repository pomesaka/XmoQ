//! The stream handling, run on the in-memory connection. Time is paused, so
//! deadlines pass at once and "nothing happens" can be asserted exactly.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use xmoq_moqt::Error;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::fetch::FetchHeader;
use xmoq_moqt::message::{ControlMessage, RequestOk, Setup};

use crate::memory::{MemoryConnection, MemorySend, STREAM_WINDOW, pair};
use crate::{
    Connection, FetchBody, FetchChunk, IncomingStreams, OutgoingEvent, OutgoingSubgroup,
    PeerRequestClose, PeerRequestStreamEvent, QueueFull, ReadError, RecvStream, RequestReader,
    RequestStreamEvent, SendBudget, SendLimits, SendStream, StreamConfig, StreamEnd, StreamStops,
    Trace, UniEvent, WriteError, read_request_streams, read_uni_streams,
};

const SOON: Duration = Duration::from_secs(1);

fn header() -> SubgroupHeader {
    SubgroupHeader {
        track_alias: 7,
        group_id: 0,
        subgroup_id: SubgroupId::Zero,
        publisher_priority: Some(0),
        properties: false,
        end_of_group: false,
        first_object: true,
    }
}

fn object(object_id: u64, payload_len: usize) -> SubgroupObject {
    SubgroupObject {
        object_id,
        properties: vec![],
        status: ObjectStatus::Normal,
        payload: vec![0x61; payload_len],
    }
}

fn end_of_group(object_id: u64) -> SubgroupObject {
    SubgroupObject {
        status: ObjectStatus::EndOfGroup,
        ..object(object_id, 0)
    }
}

/// The bytes of a subgroup stream with these Objects.
fn subgroup_bytes(objects: &[SubgroupObject]) -> Vec<u8> {
    let mut encoder = SubgroupStreamEncoder::new(&header());
    for object in objects {
        encoder.object(object).unwrap();
    }
    encoder.take_bytes()
}

fn message_bytes(message: &ControlMessage) -> Vec<u8> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes).unwrap();
    bytes
}

fn setup() -> ControlMessage {
    ControlMessage::Setup(Setup::default())
}

fn request_ok() -> ControlMessage {
    ControlMessage::RequestOk(RequestOk::default())
}

/// One endpoint that reads what the other, `peer`, sends.
struct Reading {
    peer: MemoryConnection,
    stops: StreamStops,
    events: mpsc::Receiver<UniEvent>,
    _readers: IncomingStreams,
}

fn reading(config: StreamConfig, queue_len: usize) -> Reading {
    let (local, peer) = pair();
    let stops = StreamStops::default();
    let (queue, events) = mpsc::channel(queue_len);
    let readers = read_uni_streams(Arc::new(local), config, stops.clone(), queue, |event| event);
    Reading {
        peer,
        stops,
        events,
        _readers: readers,
    }
}

impl Reading {
    async fn next(&mut self) -> UniEvent {
        timeout(SOON, self.events.recv())
            .await
            .expect("an event in time")
            .expect("the readers are running")
    }

    /// Nothing more arrives, however long one waits.
    async fn assert_quiet(&mut self) {
        let waited = timeout(Duration::from_secs(3600), self.events.recv()).await;
        assert!(matches!(waited, Err(_) | Ok(None)), "{waited:?}");
    }

    async fn opened(&mut self) -> u64 {
        match self.next().await {
            UniEvent::SubgroupOpened { stream, header: h } => {
                assert_eq!(h, header());
                stream
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

async fn open_with(peer: &MemoryConnection, bytes: &[u8]) -> MemorySend {
    let mut send = peer.open_uni().await.unwrap();
    send.write_all(bytes).await.unwrap();
    send
}

#[tokio::test(start_paused = true)]
async fn control_messages_and_objects_are_delivered_in_order() {
    let mut reading = reading(StreamConfig::default(), 8);
    let control = open_with(&reading.peer, &message_bytes(&setup())).await;
    assert_eq!(reading.next().await, UniEvent::Control(setup()));

    let data = open_with(
        &reading.peer,
        &subgroup_bytes(&[object(0, 3), object(1, 3)]),
    )
    .await;
    let stream = reading.opened().await;
    for object_id in [0, 1] {
        assert_eq!(
            reading.next().await,
            UniEvent::Object {
                stream,
                header: header(),
                object: object(object_id, 3),
            }
        );
    }
    data.finish();
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream,
            header: header(),
            end: StreamEnd::Fin,
        }
    );
    // The slot of the stream is free again; the control stream keeps its own.
    assert_eq!(reading.stops.len(), 1);

    // §3.3: closing the control stream is a PROTOCOL_VIOLATION.
    control.finish();
    assert_eq!(
        reading.next().await,
        UniEvent::Failed(Error::ProtocolViolation(
            "the peer closed its control stream"
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn a_full_queue_holds_the_sender_back() {
    // One event fits in the queue; the reader waits with the next one in hand.
    let mut reading = reading(StreamConfig::default(), 1);
    let objects: Vec<_> = (0..40).map(|id| object(id, 30)).collect();
    let bytes = subgroup_bytes(&objects);
    assert!(bytes.len() > 10 * STREAM_WINDOW);

    let mut send = reading.peer.open_uni().await.unwrap();
    let write = tokio::spawn(async move {
        send.write_all(&bytes).await.unwrap();
        send.finish();
    });
    // Nobody takes events: the writer gets stuck on the stream's window.
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(!write.is_finished(), "the whole stream was taken");

    // Taking the events lets the writer through to the end.
    let mut objects = 0;
    loop {
        match reading.next().await {
            UniEvent::Object { .. } => objects += 1,
            UniEvent::SubgroupClosed { end, .. } => {
                assert_eq!(end, StreamEnd::Fin);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(objects, 40);
    write.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn at_most_the_configured_number_of_streams_is_read_at_once() {
    let config = StreamConfig {
        max_uni_streams: 1,
        ..StreamConfig::default()
    };
    let mut reading = reading(config, 8);
    let first = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    let _second = open_with(&reading.peer, &subgroup_bytes(&[])).await;

    let stream = reading.opened().await;
    // The second stream waits unaccepted while the first is open.
    reading.assert_quiet().await;

    first.finish();
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream,
            header: header(),
            end: StreamEnd::Fin,
        }
    );
    assert_ne!(reading.opened().await, stream);
}

#[tokio::test(start_paused = true)]
async fn a_stream_is_stopped_on_request() {
    let mut reading = reading(StreamConfig::default(), 8);
    let mut send = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    let stream = reading.opened().await;

    reading.stops.stop(stream, 0x5);
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream,
            header: header(),
            end: StreamEnd::Aborted(None),
        }
    );
    // The peer sees STOP_SENDING with the code.
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x5)))
    );
    assert!(reading.stops.is_empty());
    // Stopping a stream that is gone does nothing.
    reading.stops.stop(stream, 0x5);
}

#[tokio::test(start_paused = true)]
async fn the_ways_a_subgroup_stream_ends_are_told_apart() {
    // (what follows the header, the Stream Reset Error Code if any,
    //  the events after SubgroupOpened, the code the peer is stopped with)
    type Case = (
        &'static str,
        Vec<SubgroupObject>,
        Option<u32>,
        fn(u64) -> Vec<UniEvent>,
        Option<u32>,
    );
    let cases: [Case; 2] = [
        (
            "reset by the peer",
            vec![],
            Some(0x2),
            |stream| {
                vec![UniEvent::SubgroupClosed {
                    stream,
                    header: header(),
                    end: StreamEnd::Aborted(Some(0x2)),
                }]
            },
            None,
        ),
        // An Object after End of Group: a malformed track (§2.4.2). The
        // stream is stopped with MALFORMED_TRACK (0x12).
        (
            "malformed track",
            vec![end_of_group(0)],
            None,
            |stream| {
                vec![
                    UniEvent::Object {
                        stream,
                        header: header(),
                        object: end_of_group(0),
                    },
                    UniEvent::TrackMalformed {
                        stream,
                        header: header(),
                        error: Error::MalformedTrack("object after end of group or end of track"),
                    },
                    UniEvent::SubgroupClosed {
                        stream,
                        header: header(),
                        end: StreamEnd::Aborted(None),
                    },
                ]
            },
            Some(0x12),
        ),
    ];
    for (name, objects, reset, expected, stopped_with) in cases {
        let mut reading = reading(StreamConfig::default(), 8);
        let mut send = Some(open_with(&reading.peer, &subgroup_bytes(&objects)).await);
        let stream = reading.opened().await;
        match (reset, &mut send) {
            (Some(code), send) => {
                if let Some(send) = send.take() {
                    send.reset(code);
                }
            }
            // One more byte: the start of an Object that may not follow.
            (None, Some(send)) => send.write_all(&[0x00]).await.unwrap(),
            (None, None) => unreachable!(),
        }
        for event in expected(stream) {
            assert_eq!(reading.next().await, event, "{name}");
        }
        reading.assert_quiet().await;
        if let (Some(code), Some(send)) = (stopped_with, &mut send) {
            assert_eq!(
                send.write_all(&[0]).await,
                Err(WriteError::Stopped(Some(code))),
                "{name}"
            );
        }
        assert!(reading.stops.is_empty(), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_object_over_the_limit_stops_its_stream_only() {
    let config = StreamConfig {
        subgroup_buffer_limit: 16,
        ..StreamConfig::default()
    };
    let mut reading = reading(config, 8);
    let mut send = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    let stream = reading.opened().await;
    // Object 0 with a Payload Length of 17 (0x00, 0x11) and its payload.
    let mut oversized = vec![0x00, 0x11];
    oversized.extend([0x61; 17]);
    send.write_all(&oversized).await.unwrap();
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream,
            header: header(),
            end: StreamEnd::Aborted(None),
        }
    );
    // INTERNAL_ERROR (0x0): the limit is this endpoint's.
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x0)))
    );
    // The session goes on: another stream is read as usual.
    let _other = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    reading.opened().await;
}

#[tokio::test(start_paused = true)]
async fn streams_that_never_say_what_they_are() {
    // (bytes sent before the stream goes silent)
    let cases: [(&str, Vec<u8>); 2] = [
        ("nothing at all", vec![]),
        // The stream type of a subgroup stream, and no header after it.
        ("half a subgroup header", vec![0x10]),
    ];
    for (name, bytes) in cases {
        let mut reading = reading(StreamConfig::default(), 8);
        let mut send = open_with(&reading.peer, &bytes).await;
        // Nothing is reported, and after the deadline (10 s) the stream is
        // stopped with INTERNAL_ERROR, which frees its slot.
        reading.assert_quiet().await;
        assert_eq!(
            send.write_all(&[0]).await,
            Err(WriteError::Stopped(Some(0x0))),
            "{name}"
        );
        assert!(reading.stops.is_empty(), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_stream_cancelled_before_its_type_changes_nothing() {
    let mut reading = reading(StreamConfig::default(), 8);
    let send = reading.peer.open_uni().await.unwrap();
    send.reset(0x1);
    reading.assert_quiet().await;
    assert!(reading.stops.is_empty());

    // An unknown stream type, in contrast, ends the session.
    let _unknown = open_with(&reading.peer, &[0x20]).await;
    assert_eq!(
        reading.next().await,
        UniEvent::Failed(Error::UnknownStreamType(0x20))
    );
}

impl Reading {
    /// The next event is a fetch stream with Request ID 2.
    async fn fetch_opened(&mut self) -> u64 {
        match self.next().await {
            UniEvent::FetchOpened { stream, header } => {
                assert_eq!(header, FetchHeader { request_id: 2 });
                stream
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// Reads a body to its end: the bytes, and how the stream ended.
async fn read_body(body: &mut FetchBody) -> (Vec<u8>, StreamEnd) {
    let mut bytes = Vec::new();
    loop {
        match timeout(SOON, body.next()).await.expect("a chunk in time") {
            FetchChunk::Bytes(chunk) => bytes.extend_from_slice(&chunk),
            FetchChunk::End(end) => return (bytes, end),
            FetchChunk::Stopped => panic!("the stream was stopped"),
        }
    }
}

/// `FETCH_HEADER` with Request ID 2.
const FETCH_HEADER_BYTES: [u8; 2] = [0x05, 0x02];

#[tokio::test(start_paused = true)]
async fn a_fetch_stream_is_announced_and_its_bytes_are_handed_over_undecoded() {
    let mut reading = reading(StreamConfig::default(), 8);
    // The header and the first bytes in one write, the rest in another. The
    // bytes are no Objects: nothing here decodes them.
    let mut send = open_with(&reading.peer, &[0x05, 0x02, 0xaa, 0xbb]).await;
    let stream = reading.fetch_opened().await;
    let mut body = reading.stops.take_fetch_body(stream).expect("the body");
    assert!(reading.stops.take_fetch_body(stream).is_none());

    send.write_all(&[0xcc]).await.unwrap();
    send.finish();
    assert_eq!(
        read_body(&mut body).await,
        (vec![0xaa, 0xbb, 0xcc], StreamEnd::Fin)
    );
    // Nothing of the stream went into the queue but its opening.
    reading.assert_quiet().await;
    // The reader is gone, though the body is still held.
    assert!(reading.stops.is_empty());
    drop(body);
}

#[tokio::test(start_paused = true)]
async fn a_fetch_stream_that_ended_keeps_its_bytes_and_its_slot_until_they_are_taken() {
    let config = StreamConfig {
        max_uni_streams: 1,
        ..StreamConfig::default()
    };
    let mut reading = reading(config, 8);
    let first = open_with(&reading.peer, &[0x05, 0x02, 0xaa]).await;
    first.finish();
    let _second = open_with(&reading.peer, &subgroup_bytes(&[])).await;

    let stream = reading.fetch_opened().await;
    // The one slot is held, so the second stream waits unaccepted.
    reading.assert_quiet().await;
    assert_eq!(reading.stops.len(), 1);

    let mut body = reading.stops.take_fetch_body(stream).expect("the body");
    // Taking the body frees the slot; reading it can wait.
    reading.opened().await;
    assert_eq!(read_body(&mut body).await, (vec![0xaa], StreamEnd::Fin));
}

#[tokio::test(start_paused = true)]
async fn the_ways_a_fetch_stream_ends_are_told_apart() {
    // Reset by the peer, with a code.
    let mut reading = reading(StreamConfig::default(), 8);
    let send = open_with(&reading.peer, &FETCH_HEADER_BYTES).await;
    let stream = reading.fetch_opened().await;
    let mut body = reading.stops.take_fetch_body(stream).expect("the body");
    send.reset(0x6);
    assert_eq!(
        read_body(&mut body).await,
        (vec![], StreamEnd::Aborted(Some(0x6)))
    );

    // Stopped by this endpoint: the peer sees the code, and the body ends
    // without a FIN.
    let mut send = open_with(&reading.peer, &FETCH_HEADER_BYTES).await;
    let stream = reading.fetch_opened().await;
    let mut body = reading.stops.take_fetch_body(stream).expect("the body");
    reading.stops.stop(stream, 0x12);
    assert_eq!(body.next().await, FetchChunk::Stopped);
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x12)))
    );

    // Stopped before the body was taken: there is none to take.
    let mut send = open_with(&reading.peer, &FETCH_HEADER_BYTES).await;
    let stream = reading.fetch_opened().await;
    reading.stops.stop(stream, 0x1);
    assert!(reading.stops.take_fetch_body(stream).is_none());
    // The reader acts on the stop when it runs next.
    tokio::time::sleep(SOON).await;
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x1)))
    );

    // The body is dropped while the peer sends nothing: the stream is
    // stopped with CANCELLED (0x1) and its slot is free, without a byte
    // from the peer.
    let mut send = open_with(&reading.peer, &FETCH_HEADER_BYTES).await;
    let stream = reading.fetch_opened().await;
    drop(reading.stops.take_fetch_body(stream));
    tokio::time::sleep(SOON).await;
    assert!(reading.stops.is_empty());
    assert_eq!(send.stopped().await, WriteError::Stopped(Some(0x1)));

    // After the end of a stream, the body has nothing more to say.
    let send = open_with(&reading.peer, &FETCH_HEADER_BYTES).await;
    let stream = reading.fetch_opened().await;
    let mut body = reading.stops.take_fetch_body(stream).expect("the body");
    send.finish();
    assert_eq!(body.next().await, FetchChunk::End(StreamEnd::Fin));
    assert_eq!(body.next().await, FetchChunk::Stopped);
}

#[tokio::test(start_paused = true)]
async fn a_fetch_body_nobody_reads_holds_the_sender_back() {
    let mut reading = reading(StreamConfig::default(), 8);
    let mut send = open_with(&reading.peer, &FETCH_HEADER_BYTES).await;
    let stream = reading.fetch_opened().await;
    let mut body = reading.stops.take_fetch_body(stream).expect("the body");

    let bytes = vec![0x5a; 1024 * 1024];
    let expected = bytes.clone();
    let write = tokio::spawn(async move {
        send.write_all(&bytes).await.unwrap();
        send.finish();
    });
    // Nobody reads the body: the writer gets stuck on the stream's window.
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(!write.is_finished(), "the whole stream was taken");

    let (received, end) = read_body(&mut body).await;
    assert_eq!(end, StreamEnd::Fin);
    assert!(received == expected, "{} bytes", received.len());
    write.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_fetch_stream_without_its_header() {
    // Ends inside the header: the session cannot go on (§10.12.3 has the
    // header on every fetch stream).
    let mut reading = reading(StreamConfig::default(), 8);
    let send = open_with(&reading.peer, &[0x05]).await;
    send.finish();
    assert_eq!(
        reading.next().await,
        UniEvent::Failed(Error::ProtocolViolation(
            "stream ended before its FETCH_HEADER"
        ))
    );

    // Reset inside the header: as a stream cancelled before its type.
    let mut reading = reading_again();
    let send = open_with(&reading.peer, &[0x05]).await;
    send.reset(0x1);
    reading.assert_quiet().await;
    assert!(reading.stops.is_empty());

    // Never completes its header: stopped at the deadline, like a stream
    // that never says what it is.
    let mut reading = reading_again();
    let mut send = open_with(&reading.peer, &[0x05]).await;
    reading.assert_quiet().await;
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x0)))
    );
}

fn reading_again() -> Reading {
    reading(StreamConfig::default(), 8)
}

#[tokio::test(start_paused = true)]
async fn padding_streams_are_read_and_dropped() {
    let mut reading = reading(StreamConfig::default(), 8);
    // The stream type 0x132B3E28, then more bytes than the window holds.
    let mut send = open_with(&reading.peer, &[0xf0, 0x13, 0x2b, 0x3e, 0x28]).await;
    send.write_all(&vec![0; 4 * STREAM_WINDOW]).await.unwrap();
    send.finish();
    reading.assert_quiet().await;
    assert!(reading.stops.is_empty());
}

#[tokio::test(start_paused = true)]
async fn what_was_read_before_a_disconnect_is_still_delivered() {
    let mut reading = reading(StreamConfig::default(), 1);
    let send = open_with(&reading.peer, &subgroup_bytes(&[object(0, 3)])).await;
    send.finish();
    // The reader takes the bytes and waits with an event in hand: one fits in
    // the queue, and nobody takes it yet.
    tokio::time::sleep(SOON).await;
    reading.peer.close(0, b"");

    // What was decoded is not lost with the connection.
    let stream = reading.opened().await;
    assert_eq!(
        reading.next().await,
        UniEvent::Object {
            stream,
            header: header(),
            object: object(0, 3),
        }
    );
    // The FIN was not read before the connection went, and nothing is read
    // from a connection that is gone.
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream,
            header: header(),
            end: StreamEnd::Lost,
        }
    );
    // Then the readers are done.
    assert_eq!(timeout(SOON, reading.events.recv()).await, Ok(None));
}

#[tokio::test(start_paused = true)]
async fn a_disconnect_ends_open_streams_as_lost() {
    let mut reading = reading(StreamConfig::default(), 8);
    let _send = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    let stream = reading.opened().await;
    reading.peer.close(0x3, b"bye");
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream,
            header: header(),
            end: StreamEnd::Lost,
        }
    );
    assert_eq!(timeout(SOON, reading.events.recv()).await, Ok(None));
}

/// What a request stream event looks like without its sending half.
#[derive(Debug, PartialEq)]
enum Seen {
    Opened(u64, ControlMessage),
    Message(u64, ControlMessage),
    Closed(u64, PeerRequestClose),
    Failed(Error),
    ReaderPanicked,
}

struct Requests {
    peer: MemoryConnection,
    stops: StreamStops,
    events: mpsc::Receiver<PeerRequestStreamEvent<MemorySend>>,
    /// The sending halves that came with `Opened`, kept open.
    sends: Vec<MemorySend>,
    _readers: IncomingStreams,
}

fn requests() -> Requests {
    let (local, peer) = pair();
    let stops = StreamStops::default();
    let (queue, events) = mpsc::channel(8);
    let readers = read_request_streams(
        Arc::new(local),
        StreamConfig::default(),
        stops.clone(),
        queue,
        |event| event,
    );
    Requests {
        peer,
        stops,
        events,
        sends: Vec::new(),
        _readers: readers,
    }
}

impl Requests {
    async fn next(&mut self) -> Seen {
        let event = timeout(SOON, self.events.recv())
            .await
            .expect("an event in time")
            .expect("the readers are running");
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                send,
                first,
            } => {
                self.sends.push(send);
                Seen::Opened(stream, first)
            }
            PeerRequestStreamEvent::Message { stream, message } => Seen::Message(stream, message),
            PeerRequestStreamEvent::Closed { stream, how } => Seen::Closed(stream, how),
            PeerRequestStreamEvent::Failed(error) => Seen::Failed(error),
            PeerRequestStreamEvent::ReaderPanicked => Seen::ReaderPanicked,
        }
    }

    async fn assert_quiet(&mut self) {
        let waited = timeout(Duration::from_secs(3600), self.events.recv()).await;
        assert!(waited.is_err(), "an event arrived");
    }
}

#[tokio::test(start_paused = true)]
async fn request_streams_report_their_messages_and_their_end() {
    // (how the peer ends the stream: with this reset code, or with a FIN;
    //  what is reported)
    let endings = [
        ("FIN", None, PeerRequestClose::Fin),
        // CANCELLED (0x1) and MALFORMED_TRACK (0x12): the code is the peer's.
        ("reset", Some(0x1), PeerRequestClose::Reset(Some(0x1))),
        (
            "another reset",
            Some(0x12),
            PeerRequestClose::Reset(Some(0x12)),
        ),
    ];
    for (name, reset, reported) in endings {
        let mut requests = requests();
        let (mut send, _recv) = requests.peer.open_bi().await.unwrap();
        send.write_all(&message_bytes(&setup())).await.unwrap();
        let Seen::Opened(stream, first) = requests.next().await else {
            panic!("{name}: not opened");
        };
        assert_eq!(first, setup(), "{name}");

        send.write_all(&message_bytes(&request_ok())).await.unwrap();
        assert_eq!(
            requests.next().await,
            Seen::Message(stream, request_ok()),
            "{name}"
        );
        match reset {
            Some(code) => send.reset(code),
            None => send.finish(),
        }
        // Only a FIN is reported as one: a reset cancels the request.
        assert_eq!(
            requests.next().await,
            Seen::Closed(stream, reported),
            "{name}"
        );
        assert!(requests.stops.is_empty(), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_stream_is_stopped_on_request() {
    let mut requests = requests();
    let (mut send, _recv) = requests.peer.open_bi().await.unwrap();
    send.write_all(&message_bytes(&setup())).await.unwrap();
    let Seen::Opened(stream, _) = requests.next().await else {
        panic!("not opened");
    };
    requests.stops.stop(stream, 0x12);
    // Stopped by this endpoint: the peer said nothing about the stream.
    assert_eq!(
        requests.next().await,
        Seen::Closed(stream, PeerRequestClose::Abandoned)
    );
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x12)))
    );
}

#[tokio::test(start_paused = true)]
async fn request_streams_that_break_the_rules() {
    let mut requests = requests();
    // A stream that never sends its request is cancelled after the deadline,
    // in both directions, with CANCELLED (0x1), and never reported.
    let (mut send, mut recv) = requests.peer.open_bi().await.unwrap();
    requests.assert_quiet().await;
    assert_eq!(
        send.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x1)))
    );
    assert_eq!(
        recv.read(&mut [0; 8]).await,
        Err(ReadError::Reset(Some(0x1)))
    );
    assert!(requests.stops.is_empty());

    // Message type 0x3F does not exist: the session cannot go on.
    let (mut send, _recv) = requests.peer.open_bi().await.unwrap();
    send.write_all(&[0x3f, 0x00, 0x00]).await.unwrap();
    assert_eq!(
        requests.next().await,
        Seen::Failed(Error::UnknownMessageType(0x3f))
    );

    // A stream that ends inside a message.
    let (mut send, _recv) = requests.peer.open_bi().await.unwrap();
    send.write_all(&[0x07, 0x00]).await.unwrap();
    send.finish();
    assert_eq!(
        requests.next().await,
        Seen::Failed(Error::ProtocolViolation("stream ended inside a message"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_request_reader_yields_the_response_and_the_end() {
    // (how the peer ends the stream, what the reader says)
    let endings = [
        (None, crate::RequestClose::Fin, "FIN"),
        (
            Some(0x10),
            crate::RequestClose::Reset(Some(0x10)),
            "reset, code 0x10",
        ),
    ];
    for (reset, expected, text) in endings {
        assert_eq!(expected.to_string(), text);
        let (local, peer) = pair();
        let (_send, recv) = local.open_bi().await.unwrap();
        let (mut peer_send, _peer_recv) = peer.accept_bi().await.unwrap();
        let mut reader = RequestReader::new(recv, &Trace::default(), "subscribe");

        peer_send
            .write_all(&message_bytes(&request_ok()))
            .await
            .unwrap();
        assert_eq!(
            reader.next_event().await,
            Ok(RequestStreamEvent::Message(request_ok()))
        );
        match reset {
            Some(code) => peer_send.reset(code),
            None => peer_send.finish(),
        }
        assert_eq!(
            reader.next_event().await,
            Ok(RequestStreamEvent::Closed(expected))
        );
    }
}

/// Reads a stream until `len` bytes arrived.
async fn read_exactly(recv: &mut impl RecvStream, len: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 256];
    while bytes.len() < len {
        let read = timeout(SOON, recv.read(&mut buffer))
            .await
            .expect("bytes in time")
            .unwrap()
            .expect("the stream is open");
        bytes.extend_from_slice(&buffer[..read]);
    }
    bytes
}

/// One receiver, `peer`, and what is written to it.
struct Sending {
    local: Arc<MemoryConnection>,
    peer: MemoryConnection,
    budget: Arc<SendBudget>,
    /// What the writers reported, with the number each was started with.
    reports: mpsc::UnboundedReceiver<(u64, OutgoingEvent)>,
    report: mpsc::UnboundedSender<(u64, OutgoingEvent)>,
}

/// A receiver whose budget is 1024 bytes, `objects` Objects and `streams`
/// streams.
fn sending(objects: usize, streams: usize) -> Sending {
    let (local, peer) = pair();
    let (report, reports) = mpsc::unbounded_channel();
    Sending {
        local: Arc::new(local),
        peer,
        budget: SendBudget::new(SendLimits {
            bytes: 1024,
            objects,
            streams,
            delivery_patience: None,
        }),
        reports,
        report,
    }
}

impl Sending {
    fn open(&self, number: u64) -> Result<OutgoingSubgroup, QueueFull> {
        let report = self.report.clone();
        OutgoingSubgroup::spawn(
            Arc::clone(&self.local),
            header(),
            None,
            &self.budget,
            move |event| {
                let _ = report.send((number, event));
                std::future::ready(())
            },
            Trace::default(),
            format!("out#{number}"),
        )
    }

    async fn report(&mut self) -> (u64, OutgoingEvent) {
        timeout(SOON, self.reports.recv())
            .await
            .expect("a report in time")
            .expect("the channel is open")
    }
}

#[tokio::test(start_paused = true)]
async fn an_outgoing_subgroup_writes_what_is_queued_and_finishes() {
    let mut sending = sending(8, 8);
    let out = sending.open(0).unwrap();
    out.send(Arc::new(object(0, 3))).unwrap();
    out.send(Arc::new(object(1, 3))).unwrap();
    out.finish();
    // Letting go of the handle after `finish` must not cut the stream short.
    drop(out);

    let mut recv = sending.peer.accept_uni().await.unwrap();
    let expected = subgroup_bytes(&[object(0, 3), object(1, 3)]);
    assert_eq!(read_exactly(&mut recv, expected.len()).await, expected);
    assert_eq!(recv.read(&mut [0; 8]).await, Ok(None));

    assert_eq!(sending.report().await, (0, OutgoingEvent::Opened));
    assert_eq!(sending.report().await, (0, OutgoingEvent::Closed));
    // Everything was given back.
    assert_eq!(sending.budget.in_use(), (0, 0, 0));
}

#[tokio::test(start_paused = true)]
async fn a_receiver_that_does_not_read_uses_up_its_budget() {
    // (Objects the budget takes, payload length of each Object, how many are
    //  taken before the budget is used up)
    let cases = [
        // The count is the limit: 2 Objects. Each is larger than the window
        // of the stream, so the first one already cannot be written out.
        (2, 100, 2),
        // The 1024 bytes are the limit: 4 Objects of 256 bytes.
        (100, 256, 4),
    ];
    for (objects, payload_len, taken) in cases {
        let sending = sending(objects, 8);
        let out = sending.open(0).unwrap();
        let _recv = sending.peer.accept_uni().await.unwrap();
        for object_id in 0..taken {
            assert_eq!(
                out.send(Arc::new(object(object_id, payload_len))),
                Ok(()),
                "{objects} objects of {payload_len}: number {object_id}"
            );
        }
        // The receiver reads nothing, so nothing is given back, however long
        // one waits: what is stuck in the stream's window counts too.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(
            out.send(Arc::new(object(taken, payload_len))),
            Err(QueueFull),
            "{objects} objects of {payload_len}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_budget_is_shared_by_the_streams_of_a_receiver() {
    // Room for 4 Objects of 256 bytes and for 3 streams.
    let mut sending = sending(100, 3);
    let mut writers = Vec::new();
    // Short Subgroups, one Object each, all finished, none read.
    for number in 0..3 {
        let out = sending.open(number).unwrap();
        out.send(Arc::new(object(0, 256))).unwrap();
        out.finish();
        writers.push(out);
    }
    tokio::time::sleep(Duration::from_secs(3600)).await;
    // The receiver has three streams that are not written out. A fourth is
    // not started, although no single stream is anywhere near full.
    assert!(sending.open(3).is_err());
    assert_eq!(sending.budget.in_use(), (768, 3, 3));

    // Finished writers can still be reset, which gives everything back.
    for out in &writers {
        out.reset(0x5);
    }
    for _ in 0..3 {
        // Opened may or may not have been reported; Closed always is.
        while sending.report().await.1 != OutgoingEvent::Closed {}
    }
    assert_eq!(sending.budget.in_use(), (0, 0, 0));
    assert!(sending.open(4).is_ok());
}

#[tokio::test(start_paused = true)]
async fn one_object_is_taken_whatever_its_size() {
    let sending = sending(4, 8);
    let out = sending.open(0).unwrap();
    let mut recv = sending.peer.accept_uni().await.unwrap();
    // Far over the 1024 bytes of the budget.
    out.send(Arc::new(object(0, 5000))).unwrap();
    out.finish();
    let expected = subgroup_bytes(&[object(0, 5000)]);
    assert_eq!(read_exactly(&mut recv, expected.len()).await, expected);
}

#[tokio::test(start_paused = true)]
async fn a_reset_drops_what_is_queued() {
    let mut sending = sending(8, 8);
    let out = sending.open(0).unwrap();
    let mut recv = sending.peer.accept_uni().await.unwrap();
    out.send(Arc::new(object(0, 4 * STREAM_WINDOW))).unwrap();
    out.send(Arc::new(object(1, 1))).unwrap();
    tokio::time::sleep(SOON).await;

    // TOO_FAR_BEHIND (0x5).
    out.reset(0x5);
    tokio::time::sleep(SOON).await;
    assert_eq!(
        recv.read(&mut [0; 8]).await,
        Err(ReadError::Reset(Some(0x5)))
    );
    assert_eq!(sending.report().await, (0, OutgoingEvent::Opened));
    assert_eq!(sending.report().await, (0, OutgoingEvent::Closed));
    assert_eq!(sending.budget.in_use(), (0, 0, 0));
}

#[tokio::test(start_paused = true)]
async fn a_stream_reset_before_it_was_opened_is_never_reported_opened() {
    let mut sending = sending(8, 8);
    // The peer accepts nothing, and its queue of unaccepted streams (4) is
    // full: the fifth stream waits to be opened.
    let waiting: Vec<_> = (0..5).map(|number| sending.open(number).unwrap()).collect();
    tokio::time::sleep(SOON).await;
    waiting[4].reset(0x1);
    let mut reported = Vec::new();
    while reported.len() < 5 {
        reported.push(sending.report().await);
    }
    reported.sort_by_key(|(number, _)| *number);
    assert_eq!(
        reported,
        [
            (0, OutgoingEvent::Opened),
            (1, OutgoingEvent::Opened),
            (2, OutgoingEvent::Opened),
            (3, OutgoingEvent::Opened),
            (4, OutgoingEvent::Closed),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_stream_the_receiver_stopped_does_not_count_as_slow() {
    let mut sending = sending(2, 8);
    let out = sending.open(0).unwrap();
    let mut recv = sending.peer.accept_uni().await.unwrap();
    // The header arrives, then the receiver stops the stream.
    read_exactly(&mut recv, 1).await;
    // DELIVERY_TIMEOUT (0x2), a code the writer itself never uses.
    recv.stop(0x2);
    // Far more Objects than the budget holds: each is taken and dropped.
    for object_id in 0..20 {
        out.send(Arc::new(object(object_id, 4 * STREAM_WINDOW)))
            .unwrap();
        tokio::time::sleep(SOON).await;
    }
    out.finish();
    tokio::time::sleep(SOON).await;
    assert_eq!(sending.budget.in_use(), (0, 0, 0));
    // The receiver's code is reported, once, by the write that ran into it.
    assert_eq!(
        [
            sending.report().await,
            sending.report().await,
            sending.report().await,
        ],
        [
            (0, OutgoingEvent::Opened),
            (0, OutgoingEvent::Stopped(Some(0x2))),
            (0, OutgoingEvent::Closed),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_the_handle_resets_the_stream() {
    let sending = sending(8, 8);
    let out = sending.open(0).unwrap();
    let mut recv = sending.peer.accept_uni().await.unwrap();
    out.send(Arc::new(object(0, 1))).unwrap();
    tokio::time::sleep(SOON).await;
    drop(out);
    tokio::time::sleep(SOON).await;
    // CANCELLED (0x1): the Subgroup was cut short (§11.4.3).
    assert_eq!(
        recv.read(&mut [0; 64]).await,
        Err(ReadError::Reset(Some(0x1)))
    );
}

#[tokio::test(start_paused = true)]
async fn the_peer_stopping_a_send_stream_is_noticed_without_writing() {
    let (local, peer) = pair();
    let mut send = local.open_uni().await.unwrap();
    let recv = peer.accept_uni().await.unwrap();
    let stopped = timeout(SOON, send.stopped()).await;
    assert!(stopped.is_err(), "not stopped yet");
    recv.stop(0x1);
    assert_eq!(
        timeout(SOON, send.stopped()).await,
        Ok(WriteError::Stopped(Some(0x1)))
    );
}

#[tokio::test(start_paused = true)]
async fn a_half_that_is_dropped_unended_shows_as_an_abort_without_a_code() {
    let (local, peer) = pair();
    let (send, recv) = local.open_bi().await.unwrap();
    let (mut peer_send, mut peer_recv) = peer.accept_bi().await.unwrap();
    // The contract leaves this open; the in-memory connection makes it show.
    drop(send);
    drop(recv);
    assert_eq!(
        peer_recv.read(&mut [0; 8]).await,
        Err(ReadError::Reset(None))
    );
    assert_eq!(
        peer_send.write_all(&[0]).await,
        Err(WriteError::Stopped(None))
    );
}

#[tokio::test(start_paused = true)]
async fn the_session_as_a_whole_may_hold_only_so_much_undecoded() {
    // Every stream may hold 64 bytes of an unfinished Object, the session 100.
    let config = StreamConfig {
        subgroup_buffer_limit: 64,
        session_buffer_limit: 100,
        ..StreamConfig::default()
    };
    let mut reading = reading(config, 8);
    // Object 0 with a Payload Length of 60 (0x00, 0x3c), and 50 bytes of it.
    let mut partial = vec![0x00, 0x3c];
    partial.extend([0x61; 50]);

    let mut first = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    let first_stream = reading.opened().await;
    first.write_all(&partial).await.unwrap();
    let mut second = open_with(&reading.peer, &subgroup_bytes(&[])).await;
    let second_stream = reading.opened().await;
    second.write_all(&partial).await.unwrap();

    // The second stream takes the session over its limit and is stopped with
    // INTERNAL_ERROR (0x0); the first goes on and completes its Object.
    assert_eq!(
        reading.next().await,
        UniEvent::SubgroupClosed {
            stream: second_stream,
            header: header(),
            end: StreamEnd::Aborted(None),
        }
    );
    assert_eq!(
        second.write_all(&[0]).await,
        Err(WriteError::Stopped(Some(0x0)))
    );
    first.write_all(&[0x61; 10]).await.unwrap();
    assert_eq!(
        reading.next().await,
        UniEvent::Object {
            stream: first_stream,
            header: header(),
            object: object(0, 60),
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_lost_connection_gives_the_budget_back_and_is_reported() {
    let mut sending = sending(8, 8);
    let out = sending.open(0).unwrap();
    let _recv = sending.peer.accept_uni().await.unwrap();
    // More than the window takes: the writer is stuck in its write.
    out.send(Arc::new(object(0, 4 * STREAM_WINDOW))).unwrap();
    out.send(Arc::new(object(1, 4 * STREAM_WINDOW))).unwrap();
    tokio::time::sleep(SOON).await;
    assert_eq!(sending.report().await, (0, OutgoingEvent::Opened));
    assert_eq!(sending.budget.in_use(), (512, 2, 1));

    sending.peer.close(0, b"");
    assert_eq!(sending.report().await, (0, OutgoingEvent::Closed));
    assert_eq!(sending.budget.in_use(), (0, 0, 0));
}

/// One row of `fixtures/webtransport-close/reasons.tsv`; see its README.
#[derive(Debug, PartialEq, Eq)]
enum CloseReasonRow {
    /// A reason that is text, as its UTF-8 bytes, and the bytes a browser sent
    /// for it.
    Utf8 {
        name: String,
        reason: Vec<u8>,
        sent: Vec<u8>,
    },
    /// A reason that holds half of a surrogate pair. A Rust string cannot
    /// hold it, so the row is checked for its form and then left alone.
    Utf16,
}

/// Whole bytes in lower-case hexadecimal digits, or nothing at all: the same
/// as `HEX_BYTES` on the TypeScript side.
fn hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    let digit = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(digit) {
        return Err(format!(
            "not whole bytes in lower-case hexadecimal: {hex:.40}"
        ));
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).map_err(|error| error.to_string()))
        .collect()
}

/// The rows of the table as the README has them: each ends with one line
/// feed, the last one too. WHY not `str::lines`: it takes a carriage return
/// before the line feed away and lets an empty last line pass, where the
/// TypeScript side refuses both; a table that one side reads and the other
/// does not would be pinned by one of them only.
fn close_reason_lines(table: &str) -> Result<Vec<&str>, String> {
    let rows = table
        .strip_suffix('\n')
        .ok_or("the last row does not end with a line feed")?;
    Ok(rows.split('\n').collect())
}

/// Reads a row as strictly as the TypeScript side does
/// (`parseCloseReasonFixture`): a row that is not what the README describes
/// is an error, and never a row that is skipped.
fn close_reason_row(line: &str) -> Result<CloseReasonRow, String> {
    let columns: Vec<&str> = line.split('\t').collect();
    let [name, encoding, reason, sent] = columns[..] else {
        return Err(format!("{} columns, not four", columns.len()));
    };
    if name.is_empty() {
        return Err("no name".to_string());
    }
    let (reason, sent) = (hex_bytes(reason)?, hex_bytes(sent)?);
    match encoding {
        "utf8" => {
            std::str::from_utf8(&reason).map_err(|error| format!("{name}: {error}"))?;
            Ok(CloseReasonRow::Utf8 {
                name: name.to_string(),
                reason,
                sent,
            })
        }
        "utf16" if reason.len().is_multiple_of(2) => Ok(CloseReasonRow::Utf16),
        "utf16" => Err(format!("{name}: half of a UTF-16 code unit")),
        other => Err(format!("{name}: encoding {other}, neither utf8 nor utf16")),
    }
}

#[test]
fn a_close_reason_is_cut_as_a_browser_cuts_it() {
    let path = format!(
        "{}/../../fixtures/webtransport-close/reasons.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let table = std::fs::read_to_string(&path).expect("the fixture is there");
    let rows: Vec<CloseReasonRow> = close_reason_lines(&table)
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .into_iter()
        .map(|line| close_reason_row(line).unwrap_or_else(|error| panic!("{path}: {error}")))
        .collect();
    // WHY counted: a loop over no rows passes, and so does one that skips
    // every row. An emptied file, or one whose rows all turned into the kind
    // that is skipped, has to fail here.
    let skipped = rows
        .iter()
        .filter(|row| **row == CloseReasonRow::Utf16)
        .count();
    assert_eq!((rows.len(), skipped), (14, 2));
    for row in rows {
        if let CloseReasonRow::Utf8 { name, reason, sent } = row {
            assert_eq!(crate::close_reason(&reason).into_bytes(), sent, "{name}");
        }
    }
}

#[test]
fn a_row_of_the_close_reason_fixture_is_read_as_the_readme_has_it() {
    // (a row, what it is read as)
    let read = [
        (
            "short\tutf8\t627965\t6279",
            CloseReasonRow::Utf8 {
                name: "short".to_string(),
                reason: b"bye".to_vec(),
                sent: b"by".to_vec(),
            },
        ),
        (
            "empty\tutf8\t\t",
            CloseReasonRow::Utf8 {
                name: "empty".to_string(),
                reason: vec![],
                sent: vec![],
            },
        ),
        (
            "halves\tutf16\td83dd83d\tefbfbdefbfbd",
            CloseReasonRow::Utf16,
        ),
    ];
    for (row, expected) in read {
        assert_eq!(close_reason_row(row), Ok(expected), "{row:?}");
    }
    let refused = [
        // Three columns, five columns, and no row at all.
        "short\tutf8\t627965",
        "short\tutf8\t627965\t627965\t",
        "",
        "\tutf8\t627965\t627965",
        // Half a byte in either column, a digit that is none, and capitals.
        "short\tutf8\t62796\t627965",
        "short\tutf8\t627965\t62796",
        "short\tutf8\t627965\t6279zz",
        "short\tutf8\t6279AB\t627965",
        // A carriage return left at the end of the row.
        "short\tutf8\t627965\t627965\r",
        // An encoding the README does not have.
        "short\tlatin1\t627965\t627965",
        // Bytes that are not UTF-8 under `utf8`, and half a code unit.
        "broken\tutf8\tff\tefbfbd",
        "half\tutf16\td83dd8\tefbfbd",
    ];
    for row in refused {
        assert!(close_reason_row(row).is_err(), "{row:?}");
    }
}

#[test]
fn the_rows_of_the_close_reason_fixture_end_with_a_line_feed_each() {
    // (a table, its rows; `None`: refused)
    let cases: [(&str, Option<&[&str]>); 5] = [
        ("a\nb\n", Some(&["a", "b"])),
        // An empty row is a row, and is refused where it is read.
        ("a\n\nb\n", Some(&["a", "", "b"])),
        ("a\r\nb\r\n", Some(&["a\r", "b\r"])),
        ("a\nb", None),
        ("", None),
    ];
    for (table, rows) in cases {
        assert_eq!(
            close_reason_lines(table).ok(),
            rows.map(<[&str]>::to_vec),
            "{table:?}"
        );
    }
}

/// What the caller of `open_fetch_stream` says the stream is reset with when
/// it gives the call up reaches the peer: here while the second byte of the
/// `FETCH_HEADER` waits for a reader that takes nothing.
#[tokio::test(start_paused = true)]
async fn a_fetch_stream_given_up_while_its_header_waits_is_reset_with_the_callers_code() {
    let (local, peer) = crate::memory::pair_with_window(1);
    let header = FetchHeader { request_id: 2 };
    let trace = Trace::default();
    let opening = crate::open_fetch_stream(&local, header, &trace, "fetch", 0x5);
    assert!(
        timeout(SOON, opening).await.is_err(),
        "the header waits for the reader"
    );
    let mut recv = peer.accept_uni().await.unwrap();
    // The type of the stream came, 0x05; whether it is read before the
    // reset is seen is up to the connection.
    let mut buffer = [0; 8];
    let end = loop {
        match recv.read(&mut buffer).await {
            Ok(Some(_)) => {}
            other => break other,
        }
    };
    assert_eq!(end, Err(ReadError::Reset(Some(0x5))));
}

/// A writer that is dropped after `open_fetch_stream` returned says
/// CANCELLED (0x1), whatever code the call was given for giving up.
#[tokio::test(start_paused = true)]
async fn a_fetch_stream_writer_dropped_after_it_opened_says_cancelled() {
    let header = FetchHeader { request_id: 2 };
    let mut buffer = [0; 8];
    let (local, peer) = pair();
    let writer = crate::open_fetch_stream(&local, header, &Trace::default(), "fetch", 0x5)
        .await
        .unwrap();
    drop(writer);
    let mut recv = peer.accept_uni().await.unwrap();
    let end = loop {
        match recv.read(&mut buffer).await {
            Ok(Some(_)) => {}
            other => break other,
        }
    };
    assert_eq!(end, Err(ReadError::Reset(Some(0x1))));
}

/// One row of `fixtures/first-bytes/said.tsv`; see its README.
#[derive(Debug, PartialEq, Eq)]
struct FirstBytesRow {
    name: String,
    bidirectional: bool,
    /// What the stream has sent when its time runs out.
    bytes: Vec<u8>,
    /// Whether the stream has said what it is with those, and is left alone.
    said: bool,
}

fn first_bytes_rows(table: &str) -> Result<Vec<FirstBytesRow>, String> {
    let Some(body) = table.strip_suffix('\n') else {
        return Err("the last row does not end with a line feed".to_string());
    };
    let mut rows = Vec::new();
    for line in body.split('\n') {
        let columns: Vec<&str> = line.split('\t').collect();
        let [name, kind, hex, said] = columns.as_slice() else {
            return Err(format!("not four columns: {line:?}"));
        };
        let bidirectional = match *kind {
            "uni" => false,
            "bidi" => true,
            other => return Err(format!("neither uni nor bidi: {other:?}")),
        };
        let said = match *said {
            "said" => true,
            "not-yet" => false,
            other => return Err(format!("neither said nor not-yet: {other:?}")),
        };
        if hex.len() % 2 != 0
            || hex
                .bytes()
                .any(|digit| !b"0123456789abcdef".contains(&digit))
        {
            return Err(format!("not lower-case hexadecimal bytes: {hex:?}"));
        }
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|error| format!("{hex:?}: {error}"))?;
        let row = FirstBytesRow {
            name: (*name).to_string(),
            bidirectional,
            bytes,
            said,
        };
        if rows.iter().any(|other: &FirstBytesRow| {
            (&other.name, other.bidirectional) == (&row.name, row.bidirectional)
        }) {
            return Err(format!("twice: {line:?}"));
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Sends the bytes of a row on a stream, in pieces of `piece` bytes with a
/// tenth of a second between them, and says whether the reader has stopped
/// the stream once the time for its first bytes (10 seconds) has run out:
/// the code of the `STOP_SENDING`, or `None` for a stream that is left alone.
async fn stopped_after_the_first_bytes_timeout(
    send: &mut MemorySend,
    bytes: &[u8],
    piece: usize,
) -> Option<Option<u32>> {
    for piece in bytes.chunks(piece) {
        send.write_all(piece).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(11)).await;
    match timeout(SOON, send.stopped()).await {
        Ok(WriteError::Stopped(code)) => Some(code),
        Ok(other) => panic!("the connection is gone: {other:?}"),
        Err(_) => None,
    }
}

/// The contract `fixtures/first-bytes/` pins for both implementations: with
/// which bytes a stream of the peer has said what it is, so that the time
/// for that does not stop it.
#[tokio::test(start_paused = true)]
async fn a_stream_has_said_what_it_is_where_the_shared_table_says() {
    let path = format!(
        "{}/../../fixtures/first-bytes/said.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let table = std::fs::read_to_string(&path).expect("the fixture is there");
    let rows = first_bytes_rows(&table).unwrap_or_else(|error| panic!("{path}: {error}"));
    // Every row of the table is read here; a row that is added is run.
    assert_eq!(rows.len(), 18, "{path}");
    // All at once, and a byte at a time: what counts is what has come, not
    // in how many reads.
    for piece in [usize::MAX, 1] {
        for row in &rows {
            let stopped = if row.bidirectional {
                let mut requests = requests();
                let (mut send, _recv) = requests.peer.open_bi().await.unwrap();
                let stopped =
                    stopped_after_the_first_bytes_timeout(&mut send, &row.bytes, piece).await;
                // The session stays open whatever became of the stream.
                while let Ok(event) = requests.events.try_recv() {
                    assert!(
                        !matches!(event, PeerRequestStreamEvent::Failed(_)),
                        "{row:?}: the session failed"
                    );
                }
                // A request is given up with CANCELLED (0x1).
                stopped.map(|code| (code, Some(0x1)))
            } else {
                let mut reading = reading(StreamConfig::default(), 8);
                let mut send = reading.peer.open_uni().await.unwrap();
                let stopped =
                    stopped_after_the_first_bytes_timeout(&mut send, &row.bytes, piece).await;
                while let Ok(event) = reading.events.try_recv() {
                    assert!(!matches!(event, UniEvent::Failed(_)), "{row:?}: {event:?}");
                }
                // INTERNAL_ERROR (0x0).
                stopped.map(|code| (code, Some(0x0)))
            };
            assert_eq!(stopped.is_none(), row.said, "{row:?}, in pieces of {piece}");
            if let Some((code, expected)) = stopped {
                assert_eq!(code, expected, "{row:?}, in pieces of {piece}");
            }
        }
    }
}

/// A subgroup stream that is written whole and not delivered yet. WHY on a
/// connection of its own: the in-memory one has no send buffer for a stream
/// to wait in (`memory`), and over the loopback interface a stream is
/// delivered within a millisecond. This one hands the FIN to the peer when
/// the test says so, and resets where it is given up first, as a real
/// connection does (`contract::with_a_send_buffer`).
mod held_until_delivered {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::{Notify, mpsc};
    use tokio::time::timeout;

    use super::{SOON, header, object};
    use crate::memory::{MemoryConnection, MemorySend, pair};
    use crate::{
        Connection, ConnectionEnd, ConnectionLost, OutgoingEvent, OutgoingSubgroup, ReadError,
        RecvStream, SendBudget, SendLimits, SendStream, Trace, WriteError,
    };

    struct Held(MemoryConnection, Arc<Notify>);
    struct HeldSend(MemorySend, Arc<Notify>);

    impl SendStream for HeldSend {
        async fn write_all(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
            self.0.write_all(bytes).await
        }

        fn finish(self) {
            self.0.finish();
        }

        async fn finish_and_wait(self) -> Result<(), WriteError> {
            self.0.finish_and_wait().await
        }

        fn reset(self, code: u32) {
            self.0.reset(code);
        }

        async fn stopped(&mut self) -> WriteError {
            self.0.stopped().await
        }

        async fn finish_until<G>(self, give_up: G) -> bool
        where
            G: Future<Output = u32> + Send,
        {
            tokio::select! {
                () = self.1.notified() => {
                    self.0.finish();
                    true
                }
                code = give_up => {
                    self.0.reset(code);
                    false
                }
            }
        }
    }

    impl Connection for Held {
        type Send = HeldSend;
        type Recv = <MemoryConnection as Connection>::Recv;

        async fn open_uni(&self) -> Result<Self::Send, ConnectionLost> {
            Ok(HeldSend(self.0.open_uni().await?, Arc::clone(&self.1)))
        }

        async fn open_bi(&self) -> Result<(Self::Send, Self::Recv), ConnectionLost> {
            let (send, recv) = self.0.open_bi().await?;
            Ok((HeldSend(send, Arc::clone(&self.1)), recv))
        }

        async fn accept_uni(&self) -> Result<Self::Recv, ConnectionLost> {
            self.0.accept_uni().await
        }

        async fn accept_bi(&self) -> Result<(Self::Send, Self::Recv), ConnectionLost> {
            let (send, recv) = self.0.accept_bi().await?;
            Ok((HeldSend(send, Arc::clone(&self.1)), recv))
        }

        fn close(&self, code: u32, reason: &[u8]) {
            self.0.close(code, reason);
        }

        async fn closed(&self) -> ConnectionEnd {
            self.0.closed().await
        }
    }

    struct Scene {
        out: OutgoingSubgroup,
        budget: Arc<SendBudget>,
        reports: mpsc::UnboundedReceiver<OutgoingEvent>,
        recv: <MemoryConnection as Connection>::Recv,
        delivered: Arc<Notify>,
        /// Both ends: a connection that is let go of is closed.
        _ends: (Arc<Held>, MemoryConnection),
    }

    /// One subgroup stream with an Object of 3 bytes, finished by its
    /// writer and not delivered, towards a peer that has read the Object.
    /// `patience` is the sender's limit, and `own` the stream's own time.
    async fn written_and_not_delivered(patience: Option<Duration>, own: Option<Duration>) -> Scene {
        let (local, peer) = pair();
        let delivered = Arc::new(Notify::new());
        let budget = SendBudget::new(SendLimits {
            bytes: 1024,
            objects: 8,
            streams: 8,
            delivery_patience: patience,
        });
        let (report, mut reports) = mpsc::unbounded_channel();
        let local = Arc::new(Held(local, Arc::clone(&delivered)));
        let out = OutgoingSubgroup::spawn(
            Arc::clone(&local),
            header(),
            own,
            &budget,
            move |event| {
                let _ = report.send(event);
                std::future::ready(())
            },
            Trace::default(),
            "out".to_string(),
        )
        .expect("room for a stream");
        out.send(Arc::new(object(0, 3)))
            .expect("room for the Object");
        out.finish();
        assert_eq!(
            timeout(SOON, reports.recv()).await,
            Ok(Some(OutgoingEvent::Opened))
        );
        let mut recv = timeout(SOON, peer.accept_uni())
            .await
            .expect("in time")
            .expect("accept");
        let expected = super::subgroup_bytes(&[object(0, 3)]);
        assert_eq!(
            super::read_exactly(&mut recv, expected.len()).await,
            expected
        );
        // Written whole: its bytes and its Object are given back. Not
        // delivered: it still counts as a stream of the receiver.
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(budget.in_use(), (0, 0, 1));
        assert!(
            reports.try_recv().is_err(),
            "nothing is reported while it is held"
        );
        Scene {
            out,
            budget,
            reports,
            recv,
            delivered,
            _ends: (local, peer),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_reset_after_the_fin_takes_the_stream_back() {
        let mut scene = written_and_not_delivered(None, None).await;
        // CANCELLED (0x1).
        scene.out.reset(0x1);
        // No `TimedOut`: nobody's patience ran out.
        assert_eq!(
            timeout(SOON, scene.reports.recv()).await,
            Ok(Some(OutgoingEvent::Closed))
        );
        assert_eq!(scene.budget.in_use(), (0, 0, 0));
        assert_eq!(
            scene.recv.read(&mut [0; 8]).await,
            Err(ReadError::Reset(Some(0x1)))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_is_not_delivered_in_time_is_reset_and_said_to_have_timed_out() {
        let mut scene = written_and_not_delivered(Some(Duration::from_millis(500)), None).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(scene.budget.in_use(), (0, 0, 1));
        assert_eq!(
            timeout(SOON, scene.reports.recv()).await,
            Ok(Some(OutgoingEvent::TimedOut))
        );
        assert_eq!(
            timeout(SOON, scene.reports.recv()).await,
            Ok(Some(OutgoingEvent::Closed))
        );
        assert_eq!(scene.budget.in_use(), (0, 0, 0));
        // DELIVERY_TIMEOUT (0x2).
        assert_eq!(
            scene.recv.read(&mut [0; 8]).await,
            Err(ReadError::Reset(Some(0x2)))
        );
    }

    /// The time a stream is given up after is the smaller of the sender's
    /// limit and the stream's own, and the one that is there where the
    /// other is not (§8, for the two that speak for a subscription).
    #[tokio::test(start_paused = true)]
    async fn a_stream_is_given_up_after_the_smaller_of_the_limit_and_its_own_time() {
        // (the sender's limit, the stream's own time, when it is reset), in
        // milliseconds; never where the last is `None`.
        let cases: [(Option<u64>, Option<u64>, Option<u64>); 6] = [
            (None, None, None),
            (Some(500), None, Some(500)),
            (None, Some(300), Some(300)),
            (Some(500), Some(300), Some(300)),
            (Some(300), Some(500), Some(300)),
            (Some(300), Some(300), Some(300)),
        ];
        for (limit, own, reset_after) in cases {
            let case = format!("limit {limit:?}, own {own:?}");
            let mut scene = written_and_not_delivered(
                limit.map(Duration::from_millis),
                own.map(Duration::from_millis),
            )
            .await;
            let Some(reset_after) = reset_after else {
                tokio::time::sleep(Duration::from_secs(60)).await;
                assert_eq!(scene.budget.in_use(), (0, 0, 1), "{case}");
                assert!(scene.reports.try_recv().is_err(), "{case}");
                continue;
            };
            // Not yet 100 ms before its time, and gone 100 ms after it.
            tokio::time::sleep(Duration::from_millis(reset_after - 100)).await;
            assert_eq!(scene.budget.in_use(), (0, 0, 1), "{case}");
            assert!(scene.reports.try_recv().is_err(), "{case}");
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(
                scene.reports.try_recv(),
                Ok(OutgoingEvent::TimedOut),
                "{case}"
            );
            assert_eq!(
                scene.reports.try_recv(),
                Ok(OutgoingEvent::Closed),
                "{case}"
            );
            // DELIVERY_TIMEOUT (0x2).
            assert_eq!(
                scene.recv.read(&mut [0; 8]).await,
                Err(ReadError::Reset(Some(0x2))),
                "{case}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_is_closed_when_it_is_delivered_and_a_reset_after_that_does_nothing() {
        let mut scene = written_and_not_delivered(Some(Duration::from_millis(500)), None).await;
        scene.delivered.notify_one();
        assert_eq!(
            timeout(SOON, scene.reports.recv()).await,
            Ok(Some(OutgoingEvent::Closed))
        );
        assert_eq!(scene.budget.in_use(), (0, 0, 0));
        scene.out.reset(0x1);
        assert_eq!(scene.recv.read(&mut [0; 8]).await, Ok(None));
    }

    #[tokio::test(start_paused = true)]
    async fn streams_that_are_held_count_against_the_receivers_streams() {
        let (local, _peer) = pair();
        let local = Arc::new(Held(local, Arc::new(Notify::new())));
        let budget = SendBudget::new(SendLimits {
            bytes: 1024,
            objects: 8,
            streams: 2,
            delivery_patience: None,
        });
        let open = || {
            OutgoingSubgroup::spawn(
                Arc::clone(&local),
                header(),
                None,
                &budget,
                |_| std::future::ready(()),
                Trace::default(),
                "out".to_string(),
            )
        };
        let held: Vec<OutgoingSubgroup> = (0..2).map(|_| open().expect("room")).collect();
        for out in &held {
            out.finish();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(budget.in_use(), (0, 0, 2));
        // A third is one too many: the receiver is told it is too slow.
        assert!(open().is_err());
    }
}

/// The in-memory pair that delivers a finished stream when it is told to:
/// what it shares with a real connection of the contract of `finish_until`,
/// and a stream that is given up before it is written out.
mod delivers_when_told {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::{SOON, header, object};
    use crate::memory::{STREAM_WINDOW, pair, pair_that_delivers_when_told};
    use crate::{
        Connection, OutgoingEvent, OutgoingSubgroup, ReadError, RecvStream, SendBudget, SendLimits,
        SendStream, Trace,
    };

    /// A stream of the first end with three bytes written, and its reader.
    async fn held_stream() -> (
        <crate::memory::MemoryConnection as Connection>::Send,
        <crate::memory::MemoryConnection as Connection>::Recv,
        (
            crate::memory::MemoryConnection,
            crate::memory::MemoryConnection,
        ),
        Arc<tokio::sync::Notify>,
    ) {
        let (a, b, told) = pair_that_delivers_when_told();
        let mut send = a.open_uni().await.expect("open");
        send.write_all(b"abc").await.expect("write");
        let recv = timeout(SOON, b.accept_uni())
            .await
            .expect("in time")
            .expect("accept");
        (send, recv, (a, b), told)
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_stream_waits_until_it_is_told_and_is_then_delivered() {
        let (send, mut recv, _ends, told) = held_stream().await;
        let finishing = tokio::spawn(send.finish_until(std::future::pending()));
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(!finishing.is_finished());
        told.notify_waiters();
        let delivered = timeout(SOON, finishing)
            .await
            .expect("in time")
            .expect("the task ran");
        assert!(delivered);
        let mut buffer = [0; 8];
        assert_eq!(recv.read(&mut buffer).await, Ok(Some(3)));
        assert_eq!(recv.read(&mut buffer).await, Ok(None));
    }

    /// As `contract::finish_until_is_over_on_a_stopped_stream` has it of
    /// every connection.
    #[tokio::test(start_paused = true)]
    async fn a_stream_the_peer_stops_is_over_without_being_told() {
        let (send, recv, _ends, _told) = held_stream().await;
        let finishing = tokio::spawn(send.finish_until(std::future::pending()));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!finishing.is_finished());
        // CANCELLED (0x1).
        recv.stop(0x1);
        let delivered = timeout(SOON, finishing)
            .await
            .expect("in time")
            .expect("the task ran");
        assert!(delivered);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_of_a_connection_that_is_gone_is_over_without_being_told() {
        let (send, _recv, (a, _b), _told) = held_stream().await;
        let finishing = tokio::spawn(send.finish_until(std::future::pending()));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!finishing.is_finished());
        a.close(0, b"");
        let delivered = timeout(SOON, finishing)
            .await
            .expect("in time")
            .expect("the task ran");
        assert!(delivered);
    }

    /// §8: the timer runs from when the sender knows that the Subgroup is
    /// whole, and "If the timer expires before the underlying transport
    /// stream reaches "all data committed" state ..., the implementation
    /// MUST reset the stream": also where the receiver takes nothing and
    /// the Objects are not written out.
    #[tokio::test(start_paused = true)]
    async fn a_stream_that_cannot_be_written_out_is_given_up_when_its_time_after_finish_is_over() {
        // (the stream's own time in milliseconds, whether it is reset).
        for (own, reset) in [(Some(300), true), (None, false)] {
            let case = format!("own {own:?}");
            let (local, peer) = pair();
            // Kept to the end: a connection that is let go of is closed.
            let local = Arc::new(local);
            let budget = SendBudget::new(SendLimits::default());
            let (report, mut reports) = mpsc::unbounded_channel();
            let out = OutgoingSubgroup::spawn(
                Arc::clone(&local),
                header(),
                own.map(Duration::from_millis),
                &budget,
                move |event| {
                    let _ = report.send(event);
                    std::future::ready(())
                },
                Trace::default(),
                "out".to_string(),
            )
            .expect("room for a stream");
            // More than the stream's window: the write waits for a reader
            // that never comes.
            out.send(Arc::new(object(0, 4 * STREAM_WINDOW)))
                .expect("room for the Object");
            out.finish();
            assert_eq!(
                timeout(SOON, reports.recv()).await,
                Ok(Some(OutgoingEvent::Opened)),
                "{case}"
            );
            let mut recv = timeout(SOON, peer.accept_uni())
                .await
                .expect("in time")
                .expect("accept");
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(reports.try_recv().is_err(), "{case}");
            tokio::time::sleep(Duration::from_millis(200)).await;
            if !reset {
                tokio::time::sleep(Duration::from_secs(60)).await;
                assert!(reports.try_recv().is_err(), "{case}");
                assert_eq!(budget.in_use().2, 1, "{case}");
                continue;
            }
            assert_eq!(reports.try_recv(), Ok(OutgoingEvent::TimedOut), "{case}");
            assert_eq!(reports.try_recv(), Ok(OutgoingEvent::Closed), "{case}");
            assert_eq!(budget.in_use(), (0, 0, 0), "{case}");
            // DELIVERY_TIMEOUT (0x2), whatever of the Object was in the pipe.
            let mut buffer = vec![0; 8 * STREAM_WINDOW];
            let end = loop {
                match recv.read(&mut buffer).await {
                    Ok(Some(_)) => {}
                    other => break other,
                }
            };
            assert_eq!(end, Err(ReadError::Reset(Some(0x2))), "{case}");
        }
    }
}
