//! The relay end to end, on in-memory connections: the hub, the stream
//! handling and the decisions together. Time is paused.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::message::{ControlMessage, PublishDone, ReasonPhrase};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::{RequestEvent, RequestId, Role, Session, Transport};
use xmoq_moqt_io::memory::{MemoryConnection, MemoryRecv, MemorySend, STREAM_WINDOW, pair};
use xmoq_moqt_io::{
    Connection, IncomingStreams, RecvStream, RequestClose, RequestReader, RequestStreamEvent,
    SendLimits, SendStream, StreamConfig, StreamStops, Trace, UniEvent, WriteError,
    read_uni_streams, send_message,
};
use xmoq_relay_core::Counts;

use crate::hub::{self, Held, HubConfig, HubHandle};

const SOON: Duration = Duration::from_secs(1);
/// The alias the publisher in these tests chooses.
const PUBLISHER_ALIAS: u64 = 40;

fn track() -> FullTrackName {
    let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
    FullTrackName::new(namespace, b"a".to_vec()).unwrap()
}

fn object(object_id: u64, payload_len: usize) -> SubgroupObject {
    SubgroupObject {
        object_id,
        properties: vec![],
        status: ObjectStatus::Normal,
        payload: vec![0x61; payload_len],
    }
}

/// One client of the relay.
struct Peer {
    connection: Arc<MemoryConnection>,
    session: Session,
    _control: MemorySend,
    /// Set for peers that read the streams the relay opens.
    data: Option<(mpsc::Receiver<UniEvent>, IncomingStreams)>,
}

/// A request this peer opened: the handle, and the two halves of its stream.
struct OpenRequest {
    id: RequestId,
    send: MemorySend,
    reader: RequestReader<MemoryRecv>,
}

impl Peer {
    /// Connects to the relay and sends SETUP. A peer that `reads` takes what
    /// the relay sends on unidirectional streams; one that does not stands for
    /// a receiver that has stopped reading.
    async fn connect(hub: &HubHandle<MemoryConnection>, reads: bool) -> Self {
        let (local, remote) = pair();
        hub.add_session(remote).await;
        let connection = Arc::new(local);
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        let mut control = connection.open_uni().await.unwrap();
        let setup = session.setup(vec![]).unwrap();
        send_message(&mut control, &setup, &Trace::default(), "control")
            .await
            .unwrap();
        let data = reads.then(|| {
            let (queue, events) = mpsc::channel(64);
            let readers = read_uni_streams(
                Arc::clone(&connection),
                StreamConfig::default(),
                StreamStops::default(),
                queue,
                |event| event,
            );
            (events, readers)
        });
        let mut peer = Self {
            connection,
            session,
            _control: control,
            data,
        };
        if reads {
            let UniEvent::Control(setup) = peer.next_data().await else {
                panic!("the relay did not begin with SETUP");
            };
            peer.session.on_control_message(setup).unwrap();
        }
        peer
    }

    async fn next_data(&mut self) -> UniEvent {
        let (events, _) = self.data.as_mut().expect("this peer reads");
        timeout(SOON, events.recv())
            .await
            .expect("an event in time")
            .expect("the readers are running")
    }

    /// Sends a request and returns it with the event its response produced.
    async fn request(
        &mut self,
        id: RequestId,
        message: ControlMessage,
    ) -> (OpenRequest, RequestEvent) {
        let (mut send, recv) = self.connection.open_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .unwrap();
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        let response = match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(response))) => response,
            other => panic!("no response: {other:?}"),
        };
        let event = self.session.on_request_message(id, response).unwrap();
        (OpenRequest { id, send, reader }, event)
    }

    async fn publish(&mut self) -> OpenRequest {
        let (id, message) = self
            .session
            .publish(track(), PUBLISHER_ALIAS, vec![], vec![])
            .unwrap();
        let (request, event) = self.request(id, message).await;
        assert!(
            matches!(
                event,
                RequestEvent::PublishEstablished { forward: true, .. }
            ),
            "{event:?}"
        );
        request
    }

    /// Subscribes to the track and returns the request with the Track Alias
    /// the relay assigned.
    async fn subscribe(&mut self) -> (OpenRequest, u64) {
        let (id, message) = self.session.subscribe(track(), vec![]).unwrap();
        let (request, event) = self.request(id, message).await;
        let RequestEvent::SubscribeEstablished { track_alias, .. } = event else {
            panic!("not accepted: {event:?}");
        };
        (request, track_alias)
    }

    /// Opens a subgroup stream of Group 0 and writes these Objects.
    async fn send_objects(&self, objects: &[SubgroupObject]) -> MemorySend {
        self.send_group(0, objects).await
    }

    /// Opens a subgroup stream of a Group and writes these Objects.
    async fn send_group(&self, group_id: u64, objects: &[SubgroupObject]) -> MemorySend {
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias: PUBLISHER_ALIAS,
            group_id,
            subgroup_id: SubgroupId::Zero,
            publisher_priority: Some(0),
            properties: false,
            end_of_group: false,
            first_object: true,
        });
        for object in objects {
            encoder.object(object).unwrap();
        }
        let mut send = self.connection.open_uni().await.unwrap();
        send.write_all(&encoder.take_bytes()).await.unwrap();
        send
    }

    /// The Object IDs that arrive under `track_alias` until the stream ends.
    async fn receive_stream(&mut self, track_alias: u64) -> Vec<u64> {
        let mut received = Vec::new();
        loop {
            match self.next_data().await {
                UniEvent::SubgroupOpened { header, .. } => {
                    assert_eq!(header.track_alias, track_alias);
                }
                UniEvent::Object { object, .. } => received.push(object.object_id),
                UniEvent::SubgroupClosed { .. } => return received,
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

/// The `PUBLISH_DONE` a subscriber is sent, read from its request stream.
async fn publish_done(request: &mut OpenRequest) -> PublishDone {
    match timeout(SOON, request.reader.next_event()).await {
        Ok(Ok(RequestStreamEvent::Message(ControlMessage::PublishDone(done)))) => done,
        other => panic!("no PUBLISH_DONE: {other:?}"),
    }
}

fn done(status_code: u64, stream_count: u64) -> PublishDone {
    PublishDone {
        status_code,
        stream_count,
        error_reason: ReasonPhrase::default(),
    }
}

#[tokio::test(start_paused = true)]
async fn objects_travel_from_a_publisher_to_two_subscribers() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let mut publication = publisher.publish().await;
    let mut alice = Peer::connect(&hub, true).await;
    let (mut alice_request, alice_alias) = alice.subscribe().await;
    let mut bob = Peer::connect(&hub, true).await;
    let (mut bob_request, bob_alias) = bob.subscribe().await;

    let data = publisher.send_objects(&[object(0, 3), object(1, 3)]).await;
    data.finish();
    assert_eq!(alice.receive_stream(alice_alias).await, [0, 1]);
    assert_eq!(bob.receive_stream(bob_alias).await, [0, 1]);

    // The publisher ends the track: TRACK_ENDED (0x2), one stream.
    let ended = publisher
        .session
        .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
        .unwrap();
    send_message(&mut publication.send, &ended, &Trace::default(), "publish")
        .await
        .unwrap();
    publication.send.finish();

    // Each subscriber is told, with the count of the streams it was sent.
    assert_eq!(publish_done(&mut alice_request).await, done(0x2, 1));
    assert_eq!(publish_done(&mut bob_request).await, done(0x2, 1));
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_stops_reading_is_cut_off_alone() {
    // Room for four of the Objects below in what waits for one subscriber.
    let config = HubConfig {
        send: SendLimits {
            bytes: 4 * 4 * STREAM_WINDOW,
            objects: 4096,
            streams: 256,
        },
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut reader = Peer::connect(&hub, true).await;
    let (_reader_request, reader_alias) = reader.subscribe().await;
    // The stalled subscriber never takes a unidirectional stream.
    let mut stalled = Peer::connect(&hub, false).await;
    let (mut stalled_request, _) = stalled.subscribe().await;

    // Far more than one stream's window and its queue hold.
    let objects: Vec<_> = (0..12).map(|id| object(id, 4 * STREAM_WINDOW)).collect();
    let data = publisher.send_objects(&objects).await;
    data.finish();

    // The one who reads gets everything.
    assert_eq!(
        reader.receive_stream(reader_alias).await,
        (0..12).collect::<Vec<u64>>()
    );
    // The one who does not is told TOO_FAR_BEHIND (0x5), with the one stream
    // that was opened for it.
    assert_eq!(publish_done(&mut stalled_request).await, done(0x5, 1));
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_disconnects_ends_the_track_for_its_subscribers() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (mut request, track_alias) = subscriber.subscribe().await;
    let _data = publisher.send_objects(&[object(0, 3)]).await;
    match subscriber.next_data().await {
        UniEvent::SubgroupOpened { header, .. } => assert_eq!(header.track_alias, track_alias),
        other => panic!("unexpected {other:?}"),
    }

    publisher.connection.close(0, b"");

    // TRACK_ENDED (0x2), and the one stream the subscriber was sent.
    assert_eq!(publish_done(&mut request).await, done(0x2, 1));
    // The track is gone: a new subscription is refused with DOES_NOT_EXIST.
    let mut late = Peer::connect(&hub, true).await;
    let (id, message) = late.session.subscribe(track(), vec![]).unwrap();
    let (_, event) = late.request(id, message).await;
    assert!(
        matches!(&event, RequestEvent::Rejected(error) if error.error_code == 0x10),
        "{event:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_protocol_violation_closes_that_session_with_its_code() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let offender = Peer::connect(&hub, true).await;
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;

    // A request stream that begins with a response.
    let (mut send, _recv) = offender.connection.open_bi().await.unwrap();
    let response = ControlMessage::RequestOk(xmoq_moqt::message::RequestOk::default());
    send_message(&mut send, &response, &Trace::default(), "request")
        .await
        .unwrap();
    timeout(SOON, offender.connection.closed())
        .await
        .expect("the session is closed");
    // PROTOCOL_VIOLATION (0x3).
    assert_eq!(
        offender.connection.close_info().map(|close| close.code),
        Some(0x3)
    );

    // The other sessions go on: the track can still be subscribed to.
    let mut subscriber = Peer::connect(&hub, true).await;
    subscriber.subscribe().await;
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_reads_no_stream_of_many_short_groups_is_cut_off_alone() {
    // Each Group is one Object on a stream that the publisher finishes at
    // once, so no stream ever has more than one Object waiting. What piles up
    // for the stalled subscriber is finished streams that are not written
    // out: the Object is larger than the window of a stream.
    let config = HubConfig {
        send: SendLimits {
            streams: 8,
            ..SendLimits::default()
        },
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut reader = Peer::connect(&hub, true).await;
    let (_reader_request, reader_alias) = reader.subscribe().await;
    let mut stalled = Peer::connect(&hub, false).await;
    let (mut stalled_request, _) = stalled.subscribe().await;

    for group_id in 0..30 {
        timeout(
            SOON,
            publisher.send_group(group_id, &[object(0, 4 * STREAM_WINDOW)]),
        )
        .await
        .expect("the relay takes the publisher's stream")
        .finish();
        // The one who reads gets every Group.
        loop {
            match reader.next_data().await {
                UniEvent::SubgroupOpened { header, .. } => {
                    assert_eq!(
                        (header.track_alias, header.group_id),
                        (reader_alias, group_id)
                    );
                }
                UniEvent::Object { object, .. } => assert_eq!(object.object_id, 0),
                UniEvent::SubgroupClosed { .. } => break,
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    // The one who does not is told TOO_FAR_BEHIND (0x5). Some of its streams
    // were never opened, so the count is 2^62 - 1 (§10.11).
    assert_eq!(
        publish_done(&mut stalled_request).await,
        done(0x5, 4_611_686_018_427_387_903)
    );
    // The first stream the relay opened is its control stream.
    let _control = timeout(SOON, stalled.connection.accept_uni())
        .await
        .expect("the control stream")
        .unwrap();
    // The data streams that did open are reset with TOO_FAR_BEHIND (0x5),
    // although the relay had already been asked to finish them.
    for _ in 0..3 {
        let mut recv = timeout(SOON, stalled.connection.accept_uni())
            .await
            .expect("a stream that was opened")
            .unwrap();
        let mut end = timeout(SOON, recv.read(&mut [0; 64]))
            .await
            .expect("an end");
        while let Ok(Some(_)) = end {
            end = timeout(SOON, recv.read(&mut [0; 64]))
                .await
                .expect("an end");
        }
        assert_eq!(end, Err(xmoq_moqt_io::ReadError::Reset(Some(0x5))));
    }
}

#[tokio::test(start_paused = true)]
async fn stop_sending_on_a_subscribe_stream_cancels_the_subscription() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (request, _) = subscriber.subscribe().await;
    let OpenRequest {
        mut send, reader, ..
    } = request;

    // §3.3.2: the subscriber stops the relay's half and says nothing else.
    reader.stop(0x1);

    // The relay lets go of the request: it stops the subscriber's half in
    // turn, with CANCELLED (0x1).
    assert_eq!(
        timeout(SOON, send.stopped()).await,
        Ok(WriteError::Stopped(Some(0x1)))
    );
    // And the subscription: what the publisher sends now opens no stream.
    publisher.send_objects(&[object(0, 3)]).await.finish();
    let (events, _) = subscriber.data.as_mut().expect("this peer reads");
    assert!(timeout(SOON, events.recv()).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn stopping_the_relays_control_stream_closes_the_session() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let peer = Peer::connect(&hub, false).await;
    let control = timeout(SOON, peer.connection.accept_uni())
        .await
        .expect("the relay opens its control stream")
        .unwrap();
    control.stop(0x1);

    timeout(SOON, peer.connection.closed())
        .await
        .expect("the session is closed");
    // PROTOCOL_VIOLATION (0x3).
    assert_eq!(
        peer.connection.close_info().map(|close| close.code),
        Some(0x3)
    );
}

#[tokio::test(start_paused = true)]
async fn a_session_over_the_limit_is_closed_at_once() {
    let config = HubConfig {
        max_sessions: 1,
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut first = Peer::connect(&hub, true).await;
    let second = Peer::connect(&hub, false).await;
    timeout(SOON, second.connection.closed())
        .await
        .expect("the session is closed");
    // INTERNAL_ERROR (0x1).
    assert_eq!(
        second.connection.close_info().map(|close| close.code),
        Some(0x1)
    );
    // The first one is served as before.
    first.publish().await;
}

/// A hub that holds nothing.
const NOTHING: Held = Held {
    sessions: 0,
    request_writers: 0,
    out_streams: 0,
    timers: 0,
    relay: Counts {
        sessions: 0,
        tracks: 0,
        subscriptions: 0,
        requests: 0,
        in_streams: 0,
        out_streams: 0,
        early_streams: 0,
    },
};

fn alive_tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

#[tokio::test(start_paused = true)]
async fn nothing_is_left_of_sessions_that_disconnected() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    tokio::time::sleep(SOON).await;
    // The hub's own task.
    let tasks_before = alive_tasks();

    let mut publisher = Peer::connect(&hub, true).await;
    let publication = publisher.publish().await;
    let mut reader = Peer::connect(&hub, true).await;
    let (reader_request, reader_alias) = reader.subscribe().await;
    let mut stalled = Peer::connect(&hub, false).await;
    let stalled_request = stalled.subscribe().await;
    // One stream that is written out, and one that is cut short and stuck in
    // the stalled subscriber's window.
    publisher.send_group(0, &[object(0, 3)]).await.finish();
    assert_eq!(reader.receive_stream(reader_alias).await, [0]);
    let open = publisher
        .send_group(1, &[object(0, 4 * STREAM_WINDOW)])
        .await;
    tokio::time::sleep(SOON).await;
    let held = hub.held().await;
    assert_eq!((held.sessions, held.relay.tracks), (3, 1));
    assert!(held.out_streams > 0, "{held:?}");

    // Everyone goes, without a word.
    drop((open, publication, reader_request, stalled_request));
    drop((publisher, reader, stalled));
    // Longer than every timer the hub may have started.
    tokio::time::sleep(Duration::from_secs(30)).await;

    assert_eq!(hub.held().await, NOTHING);
    assert_eq!(alive_tasks(), tasks_before);
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_disconnects_leaves_the_others_as_they_are() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut stayer = Peer::connect(&hub, true).await;
    let (mut stayer_request, stayer_alias) = stayer.subscribe().await;
    let mut leaver = Peer::connect(&hub, true).await;
    let (_leaver_request, leaver_alias) = leaver.subscribe().await;

    publisher.send_group(0, &[object(0, 3)]).await.finish();
    assert_eq!(stayer.receive_stream(stayer_alias).await, [0]);
    assert_eq!(leaver.receive_stream(leaver_alias).await, [0]);

    leaver.connection.close(0, b"");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let held = hub.held().await;
    assert_eq!((held.sessions, held.relay.subscriptions), (2, 1));

    // The one who stayed gets the next Group, and nothing ended for it.
    publisher.send_group(1, &[object(0, 3)]).await.finish();
    assert_eq!(stayer.receive_stream(stayer_alias).await, [0]);
    assert!(
        timeout(SOON, stayer_request.reader.next_event())
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn publish_done_is_passed_on_when_the_wait_for_late_streams_is_over() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let mut publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (mut request, _) = subscriber.subscribe().await;

    // PUBLISH_DONE counts one stream that never comes: TRACK_ENDED (0x2).
    let ended = publisher
        .session
        .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
        .unwrap();
    send_message(&mut publication.send, &ended, &Trace::default(), "publish")
        .await
        .unwrap();
    publication.send.finish();

    // Nothing reaches the subscriber before the two seconds of the wait.
    assert!(
        timeout(Duration::from_millis(1900), request.reader.next_event())
            .await
            .is_err()
    );
    assert_eq!(publish_done(&mut request).await, done(0x2, 0));
}

#[tokio::test(start_paused = true)]
async fn a_stream_without_a_publish_is_stopped_when_its_time_is_up() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let peer = Peer::connect(&hub, true).await;
    let mut early = peer.send_group(0, &[object(0, 3)]).await;

    assert!(
        timeout(Duration::from_millis(1900), early.stopped())
            .await
            .is_err()
    );
    // CANCELLED (0x1).
    assert_eq!(
        timeout(SOON, early.stopped()).await,
        Ok(WriteError::Stopped(Some(0x1)))
    );
    assert_eq!(hub.held().await.relay.early_streams, 0);
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_arrives_before_its_publish_belongs_to_the_track() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    // Group 4 goes out before the PUBLISH that explains its alias.
    publisher.send_group(4, &[object(0, 3)]).await.finish();
    tokio::time::sleep(SOON).await;
    assert_eq!(hub.held().await.relay.early_streams, 1);
    let _publication = publisher.publish().await;

    // Nobody was subscribed, so the Object went nowhere; it is the track's
    // largest all the same (§10.2.11).
    let mut subscriber = Peer::connect(&hub, true).await;
    let (id, message) = subscriber.session.subscribe(track(), vec![]).unwrap();
    let (_request, event) = subscriber.request(id, message).await;
    let RequestEvent::SubscribeEstablished { parameters, .. } = event else {
        panic!("not accepted: {event:?}");
    };
    assert_eq!(
        parameters,
        [Parameter::LargestObject(Location {
            group: 4,
            object: 0
        })]
    );
    assert_eq!(hub.held().await.relay.early_streams, 0);
}

#[tokio::test(start_paused = true)]
async fn a_session_whose_messages_pile_up_is_closed() {
    // Room for one message on a request stream. A refusal is two: the
    // REQUEST_ERROR and the order to finish the stream.
    let config = HubConfig {
        session_queue_len: 1,
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut peer = Peer::connect(&hub, true).await;
    let (_, message) = peer.session.subscribe(track(), vec![]).unwrap();
    let (mut send, _recv) = peer.connection.open_bi().await.unwrap();
    send_message(&mut send, &message, &Trace::default(), "request")
        .await
        .unwrap();

    timeout(SOON, peer.connection.closed())
        .await
        .expect("the session is closed");
    // INTERNAL_ERROR (0x1).
    assert_eq!(
        peer.connection.close_info().map(|close| close.code),
        Some(0x1)
    );
}

/// What a publisher does at its end: the last Object on a new stream, then
/// `PUBLISH_DONE` that counts the stream, then a FIN.
async fn send_last_object_and_publish_done(publisher: &mut Peer, publication: &mut OpenRequest) {
    let data = publisher.send_objects(&[object(0, 3)]).await;
    data.finish_and_wait().await.unwrap();
    // TRACK_ENDED (0x2), one stream.
    let ended = publisher
        .session
        .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
        .unwrap();
    send_message(&mut publication.send, &ended, &Trace::default(), "publish")
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_waits_for_the_relays_fin_before_closing_loses_nothing() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let mut publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (mut request, track_alias) = subscriber.subscribe().await;

    send_last_object_and_publish_done(&mut publisher, &mut publication).await;
    let OpenRequest {
        send, mut reader, ..
    } = publication;
    send.finish();
    // The relay finishes its direction once the one stream that PUBLISH_DONE
    // counted has ended there. Only then does the publisher close.
    assert_eq!(
        timeout(SOON, reader.next_event()).await,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Fin)))
    );
    publisher.connection.close(0, b"");

    assert_eq!(subscriber.receive_stream(track_alias).await, [0]);
    assert_eq!(publish_done(&mut request).await, done(0x2, 1));
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_closes_without_waiting_can_lose_its_last_object() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let mut publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (mut request, _) = subscriber.subscribe().await;

    // Everything reached the relay's transport (`finish_and_wait`), and the
    // publisher closes at once. The relay had not accepted the data stream,
    // and on this connection such a stream is always lost with the close.
    send_last_object_and_publish_done(&mut publisher, &mut publication).await;
    publication.send.finish_and_wait().await.unwrap();
    publisher.connection.close(0, b"");

    // The subscriber is told the track ended, and was sent no stream.
    assert_eq!(publish_done(&mut request).await, done(0x2, 0));
    let (events, _) = subscriber.data.as_mut().expect("this peer reads");
    assert!(timeout(SOON, events.recv()).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn the_relay_resets_the_publish_stream_when_the_counted_streams_do_not_come() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let mut publication = publisher.publish().await;

    // PUBLISH_DONE counts one stream, and none is ever opened.
    let ended = publisher
        .session
        .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
        .unwrap();
    send_message(&mut publication.send, &ended, &Trace::default(), "publish")
        .await
        .unwrap();
    publication.send.finish();

    // Nothing for the two seconds of the wait, then a reset with CANCELLED
    // (0x1) where a FIN would have said that everything arrived.
    assert!(
        timeout(Duration::from_millis(1900), publication.reader.next_event())
            .await
            .is_err()
    );
    assert_eq!(
        timeout(SOON, publication.reader.next_event()).await,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Reset(Some(
            0x1
        )))))
    );
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_keeps_its_stream_open_after_publish_done_is_stopped() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (mut request, _) = subscriber.subscribe().await;
    publisher.connection.close(0, b"");
    assert_eq!(publish_done(&mut request).await, done(0x2, 0));
    assert_eq!(hub.held().await.relay.requests, 1);

    // The subscriber does not close its direction. After two seconds the
    // relay stops it with CANCELLED (0x1) and forgets the request.
    assert!(
        timeout(Duration::from_millis(1900), request.send.stopped())
            .await
            .is_err()
    );
    assert_eq!(
        timeout(SOON, request.send.stopped()).await,
        Ok(WriteError::Stopped(Some(0x1)))
    );
    assert_eq!(hub.held().await.relay.requests, 0);
}
