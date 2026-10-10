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
use xmoq_moqt::message::{ControlMessage, RequestOk, Setup};

use crate::memory::{MemoryConnection, MemorySend, STREAM_WINDOW, pair};
use crate::{
    Connection, IncomingStreams, OutgoingEvent, OutgoingSubgroup, PeerRequestStreamEvent,
    QueueFull, ReadError, RecvStream, RequestReader, RequestStreamEvent, SendBudget, SendLimits,
    SendStream, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent, WriteError,
    read_request_streams, read_uni_streams,
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
    Closed(u64, bool),
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
            PeerRequestStreamEvent::Closed { stream, fin } => Seen::Closed(stream, fin),
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
    // (how the peer ends the stream: with this reset code, or with a FIN)
    let endings: [(&str, Option<u32>); 2] = [("FIN", None), ("reset", Some(0x1))];
    for (name, reset) in endings {
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
            Seen::Closed(stream, reset.is_none()),
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
    assert_eq!(requests.next().await, Seen::Closed(stream, false));
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
