//! The relay end to end, on in-memory connections: the hub, the stream
//! handling and the decisions together. Time is paused.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::fetch::{FetchEvent, FetchHeader, FetchObject, FetchStreamDecoder};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, FetchRange, PublishDone, ReasonPhrase, RequestError, RequestUpdate, Subscribe,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{
    PeerRequest, PeerRequestId, PeerRequestOpened, RequestEvent, RequestId, Role, Session,
    Transport,
};
use xmoq_moqt_io::memory::{
    MemoryConnection, MemoryRecv, MemorySend, STREAM_WINDOW, pair, pair_that_delivers_when_told,
    pair_with_stream_limit,
};
use xmoq_moqt_io::{
    Connection, FetchBody, FetchChunk, FetchStreamWriter, IdleSubgroupStreamTimeout,
    IncomingStreams, ReadError, RecvStream, RequestClose, RequestReader, RequestStreamEvent,
    SendLimits, SendStream, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent, WriteError,
    open_fetch_stream, read_uni_streams, send_message,
};
use xmoq_relay_core::{Counts, Event, FetchPipeEnd, SessionKey, StreamKey};

use crate::hub::{self, Arrival, Held, HubConfig, HubHandle};

const SOON: Duration = Duration::from_secs(1);

/// Lets the time pass that the relay gives a peer to end its half of a
/// request stream the relay finished its own half of: 2 seconds. The peers
/// of these tests keep theirs open.
async fn request_close_time() {
    assert_eq!(
        HubConfig::default().request_close_timeout,
        Duration::from_secs(2)
    );
    tokio::time::sleep(Duration::from_millis(2_100)).await;
}
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
    /// Where such a peer takes what a fetch stream carries.
    stops: StreamStops,
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
        Self::arrive(hub, reads, Arrival::First).await
    }

    /// [`Self::connect`], for a session that came as `arrival` says.
    async fn arrive(hub: &HubHandle<MemoryConnection>, reads: bool, arrival: Arrival) -> Self {
        let (local, remote) = pair();
        Self::connect_over(hub, local, remote, reads, arrival).await
    }

    /// A peer that reads, towards which the relay's finished subgroup
    /// streams are not delivered: they stay with their writers, as on a
    /// real connection whose receiver has not acknowledged them, until the
    /// test notifies what is returned with the peer.
    async fn connect_without_delivery(
        hub: &HubHandle<MemoryConnection>,
    ) -> (Self, Arc<tokio::sync::Notify>) {
        let (remote, local, told) = pair_that_delivers_when_told();
        (
            Self::connect_over(hub, local, remote, true, Arrival::First).await,
            told,
        )
    }

    async fn connect_over(
        hub: &HubHandle<MemoryConnection>,
        local: MemoryConnection,
        remote: MemoryConnection,
        reads: bool,
        arrival: Arrival,
    ) -> Self {
        hub.add_session(remote, arrival).await;
        let connection = Arc::new(local);
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        let mut control = connection.open_uni().await.unwrap();
        let setup = session.setup(vec![]).unwrap();
        send_message(&mut control, &setup, &Trace::default(), "control")
            .await
            .unwrap();
        let stops = StreamStops::default();
        let data = reads.then(|| {
            let (queue, events) = mpsc::channel(64);
            // WHY the longest idle time: these peers stand for viewers of
            // the relay, and some tests hold a stream of the relay
            // undelivered for longer than the default 30 seconds; what a
            // viewer does with a silent stream is tested in xmoq-moqt-io.
            let config = StreamConfig {
                idle_subgroup_stream_timeout: IdleSubgroupStreamTimeout::from_millis(
                    IdleSubgroupStreamTimeout::MAX_MS,
                )
                .unwrap(),
                ..StreamConfig::default()
            };
            let readers = read_uni_streams(
                Arc::clone(&connection),
                config,
                stops.clone(),
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
            stops,
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
        self.subscribe_with(vec![]).await
    }

    /// The same with these parameters on the SUBSCRIBE.
    async fn subscribe_with(&mut self, parameters: Vec<Parameter>) -> (OpenRequest, u64) {
        let (id, message) = self.session.subscribe(track(), parameters).unwrap();
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
        self.send_group_as(PUBLISHER_ALIAS, group_id, objects).await
    }

    /// The same under a Track Alias of the caller's choice.
    async fn send_group_as(
        &self,
        track_alias: u64,
        group_id: u64,
        objects: &[SubgroupObject],
    ) -> MemorySend {
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias,
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

/// Longer than the second after which a session has back the announcement
/// it used, and the timer for that is gone.
const ANNOUNCEMENT_IS_BACK: Duration = Duration::from_millis(1100);

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

/// §10.2.13, over a connection: the relay's `REQUEST_UPDATE` reaches the
/// publisher on the stream of its PUBLISH, and its answer is one the relay
/// takes.
#[tokio::test(start_paused = true)]
async fn a_new_group_request_of_a_subscriber_reaches_the_publisher_of_a_track_with_dynamic_groups()
{
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    // DYNAMIC_GROUPS (0x30) = 1.
    let dynamic_groups = vec![KeyValuePair::varint(0x30, 1).unwrap()];
    let (id, message) = publisher
        .session
        .publish(track(), PUBLISHER_ALIAS, vec![], dynamic_groups)
        .unwrap();
    let (mut publication, _) = publisher.request(id, message).await;
    let mut alice = Peer::connect(&hub, true).await;
    let (_request, alice_alias) = alice
        .subscribe_with(vec![Parameter::NewGroupRequest(0)])
        .await;

    // The relay is the server: its first Request ID is 1.
    let update = match timeout(SOON, publication.reader.next_event()).await {
        Ok(Ok(RequestStreamEvent::Message(update))) => update,
        other => panic!("no REQUEST_UPDATE: {other:?}"),
    };
    assert_eq!(
        update,
        ControlMessage::RequestUpdate(RequestUpdate {
            request_id: 1,
            parameters: vec![Parameter::NewGroupRequest(0)],
        })
    );
    let RequestEvent::SubscriptionUpdated {
        response,
        new_group_request,
        ..
    } = publisher
        .session
        .on_request_message(publication.id, update)
        .unwrap()
    else {
        panic!("the publisher's session did not take the update");
    };
    assert_eq!(new_group_request, Some(0));
    send_message(
        &mut publication.send,
        &response,
        &Trace::default(),
        "publish",
    )
    .await
    .unwrap();

    // The session of the publisher goes on: its next Group reaches Alice.
    let data = publisher.send_group(1, &[object(0, 3)]).await;
    data.finish();
    assert_eq!(alice.receive_stream(alice_alias).await, [0]);
    assert_eq!(hub.held().await.sessions, 2);
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_stops_reading_is_cut_off_alone_by_each_of_the_limits() {
    // Room for four of the Objects below in what waits for one subscription,
    // one subscriber's session, or the relay; the others far larger.
    let room = 4 * 4 * STREAM_WINDOW;
    let limits = |bytes| SendLimits {
        bytes,
        objects: 4096,
        streams: 256,
        delivery_patience: None,
    };
    let large = 1 << 30;
    // (the limit that is reached, the config)
    let cases = [
        (
            "subscription",
            HubConfig {
                send_for_one_subscription: limits(room),
                send: limits(large),
                ..HubConfig::default()
            },
        ),
        (
            "session",
            HubConfig {
                send_for_one_subscription: limits(large),
                send: limits(room),
                ..HubConfig::default()
            },
        ),
        (
            "relay",
            HubConfig {
                send_for_one_subscription: limits(large),
                send: limits(large),
                send_for_the_relay_bytes: room,
                ..HubConfig::default()
            },
        ),
    ];
    for (limit, config) in cases {
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
            (0..12).collect::<Vec<u64>>(),
            "{limit}"
        );
        // The one who does not is told TOO_FAR_BEHIND (0x5), with the one
        // stream that was opened for it.
        assert_eq!(
            publish_done(&mut stalled_request).await,
            done(0x5, 1),
            "{limit}"
        );
    }
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

#[tokio::test(start_paused = true)]
async fn a_session_over_the_limit_is_sent_to_when_full_once_and_closed_otherwise() {
    // (where the relay sends it; how it came; the New Session URI of its
    //  GOAWAY, or None for a session closed at once with INTERNAL_ERROR)
    let cases: [(Option<&str>, Arrival, Option<&str>); 4] = [
        (
            Some("https://b.example/moq"),
            Arrival::First,
            Some("https://b.example/moq?sent-away=1"),
        ),
        (
            Some("https://b.example/moq?room=r#x"),
            Arrival::First,
            Some("https://b.example/moq?room=r&sent-away=1#x"),
        ),
        (Some("https://b.example/moq"), Arrival::SentAway, None),
        (None, Arrival::First, None),
    ];
    for (when_full, arrival, uri) in cases {
        let config = HubConfig {
            max_sessions: 1,
            when_full: when_full.map(str::to_string),
            ..HubConfig::default()
        };
        let (hub, _task) = hub::spawn(config);
        let mut first = Peer::connect(&hub, true).await;
        let mut second = Peer::arrive(&hub, uri.is_some(), arrival).await;
        let Some(uri) = uri else {
            timeout(SOON, second.connection.closed())
                .await
                .expect("the session is closed");
            assert_eq!(
                second.connection.close_info().map(|close| close.code),
                Some(0x1),
                "{when_full:?} {arrival:?}"
            );
            continue;
        };
        let UniEvent::Control(ControlMessage::GoAway(go_away)) = second.next_data().await else {
            panic!("no GOAWAY: {when_full:?} {arrival:?}");
        };
        assert_eq!(
            (
                go_away.new_session_uri.as_slice(),
                go_away.timeout_ms,
                go_away.request_id
            ),
            (uri.as_bytes(), hub::SEND_AWAY_TIMEOUT_MS, Some(0)),
        );
        // It was not served: what it asks is refused with GOING_AWAY (0x6).
        let (id, message) = second
            .session
            .publish(track(), PUBLISHER_ALIAS, vec![], vec![])
            .unwrap();
        let (_request, event) = second.request(id, message).await;
        assert!(
            matches!(&event, RequestEvent::Rejected(error) if error.error_code == 0x6),
            "{event:?}"
        );
        // The first is served as before, and was told nothing.
        first.publish().await;
        // GOAWAY_TIMEOUT (0x10) once its Timeout is over.
        tokio::time::sleep(Duration::from_millis(hub::SEND_AWAY_TIMEOUT_MS - 10)).await;
        assert!(second.connection.close_info().is_none());
        timeout(SOON, second.connection.closed())
            .await
            .expect("the session is closed");
        assert_eq!(
            second.connection.close_info().map(|close| close.code),
            Some(0x10)
        );
    }
}

#[test]
fn a_session_came_sent_away_where_its_query_has_the_pair() {
    // (the query of the URL; how the session came)
    let cases = [
        (None, Arrival::First),
        (Some(""), Arrival::First),
        (Some("sent-away=1"), Arrival::SentAway),
        (Some("room=r&sent-away=1"), Arrival::SentAway),
        (Some("sent-away=10"), Arrival::First),
        (Some("not-sent-away=1"), Arrival::First),
    ];
    for (query, arrival) in cases {
        assert_eq!(Arrival::of_query(query), arrival, "{query:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn sessions_being_sent_away_have_a_limit_of_their_own() {
    let config = HubConfig {
        max_sessions: 1,
        when_full: Some("https://b.example/".to_string()),
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let _first = Peer::connect(&hub, true).await;
    let mut away = Vec::new();
    for _ in 0..hub::MAX_SENDING_AWAY {
        let mut peer = Peer::connect(&hub, true).await;
        assert!(
            matches!(
                peer.next_data().await,
                UniEvent::Control(ControlMessage::GoAway(_))
            ),
            "sent away"
        );
        away.push(peer);
    }
    let one_more = Peer::connect(&hub, false).await;
    timeout(SOON, one_more.connection.closed())
        .await
        .expect("the session is closed");
    assert_eq!(
        one_more.connection.close_info().map(|close| close.code),
        Some(0x1)
    );
    // One that goes makes room for the next to be sent away.
    away.pop().expect("one").connection.close(0, b"");
    tokio::time::sleep(Duration::from_millis(10)).await;
    let mut next = Peer::connect(&hub, true).await;
    assert!(
        matches!(
            next.next_data().await,
            UniEvent::Control(ControlMessage::GoAway(_))
        ),
        "sent away"
    );
}

#[tokio::test(start_paused = true)]
async fn a_client_that_sends_no_setup_is_closed_when_its_time_is_over() {
    // (the time a client has; how long the test waits; whether the session
    //  is closed by then)
    let cases = [
        (10_000, 9_999, false),
        (10_000, 10_001, true),
        (1_500, 1_499, false),
        (1_500, 1_501, true),
    ];
    for (has_ms, waits_ms, closed) in cases {
        let config = HubConfig {
            setup_timeout: Duration::from_millis(has_ms),
            ..HubConfig::default()
        };
        let (hub, _task) = hub::spawn(config);
        // A session and nothing on it: no control stream, no SETUP.
        let (local, remote) = pair();
        hub.add_session(remote, Arrival::First).await;
        let ended = timeout(Duration::from_millis(waits_ms), local.closed()).await;
        assert_eq!(ended.is_ok(), closed, "{has_ms} {waits_ms}");
        // CONTROL_MESSAGE_TIMEOUT (0x11).
        assert_eq!(
            local.close_info().map(|close| close.code),
            closed.then_some(0x11),
            "{has_ms} {waits_ms}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_client_that_sent_its_setup_stays_when_that_time_is_over() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut peer = Peer::connect(&hub, true).await;
    // Twice the time a SETUP has, and less than a session may hold nothing.
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(peer.connection.close_info().map(|close| close.code), None);
    peer.publish().await;
}

/// What a session does after its SETUP, for the test below.
#[derive(Debug, Clone, Copy)]
enum Holds {
    Nothing,
    /// A namespace it announced, with no subscriber.
    Namespace,
    /// A PUBLISH with no subscriber.
    Publish,
}

#[tokio::test(start_paused = true)]
async fn a_session_that_holds_nothing_is_closed_after_thirty_seconds_and_one_that_holds_something_stays()
 {
    // (what the session holds; whether it is closed, with NO_ERROR, after
    //  30 seconds)
    let cases = [
        (Holds::Nothing, true),
        (Holds::Namespace, false),
        (Holds::Publish, false),
    ];
    for (holds, closed) in cases {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut peer = Peer::connect(&hub, true).await;
        let _kept = match holds {
            Holds::Nothing => None,
            Holds::Namespace => Some(peer.publish_namespace().await),
            Holds::Publish => Some(peer.publish().await),
        };
        tokio::time::sleep(Duration::from_millis(29_900)).await;
        assert_eq!(
            peer.connection.close_info().map(|close| close.code),
            None,
            "{holds:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            peer.connection.close_info().map(|close| close.code),
            closed.then_some(0x0),
            "{holds:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_time_a_session_may_hold_nothing_begins_when_its_last_request_has_gone() {
    let config = HubConfig {
        idle_session_timeout: Duration::from_secs(3),
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut peer = Peer::connect(&hub, true).await;
    let announced = peer.publish_namespace().await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(peer.connection.close_info().map(|close| close.code), None);
    // The namespace is withdrawn: the request is cancelled (§6.2).
    // CANCELLED (0x1).
    announced.send.reset(0x1);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(peer.connection.close_info().map(|close| close.code), None);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        peer.connection.close_info().map(|close| close.code),
        Some(0x0)
    );
}

/// A session that comes to hold nothing again and again has the hub start
/// no task for it: a peer decides how often that is.
#[tokio::test(start_paused = true)]
async fn a_session_that_comes_to_hold_nothing_again_and_again_adds_no_task() {
    // 30 s to hold nothing, longer than the turns below.
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut peer = Peer::connect(&hub, true).await;
    let mut announce_and_withdraw = async || {
        let announced = peer.publish_namespace().await;
        // CANCELLED (0x1) withdraws the namespace (§6.2).
        announced.send.reset(0x1);
        tokio::time::sleep(SOON).await;
    };
    announce_and_withdraw().await;
    let tasks = alive_tasks();
    for _ in 0..8 {
        announce_and_withdraw().await;
    }
    assert_eq!(alive_tasks(), tasks);
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(
        peer.connection.close_info().map(|close| close.code),
        Some(0x0)
    );
}

/// A hub that holds nothing.
const NOTHING: Held = Held {
    sessions: 0,
    request_writers: 0,
    out_streams: 0,
    own_requests: 0,
    fetch_pipes: 0,
    timers: 0,
    relay: Counts {
        sessions: 0,
        tracks: 0,
        subscriptions: 0,
        requests: 0,
        request_streams: 0,
        half_closed_streams: 0,
        in_streams: 0,
        out_streams: 0,
        sources: 0,
        asks: 0,
        declined: 0,
        early_streams: 0,
        cached_bytes: 0,
        held_beyond_cache_bytes: 0,
        remembered_subgroups: 0,
        namespaces: 0,
        pending_tracks: 0,
        own_requests: 0,
        fetches: 0,
        track_statuses: 0,
        fetch_ends: 0,
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

/// One round of a publisher that publishes the track, says `PUBLISH_DONE`
/// that counts one stream, and sends that stream `late` afterwards. Returns
/// how the relay ended its direction of the PUBLISH stream: a FIN where the
/// stream was waited for.
async fn publishes_and_ends(publisher: &mut Peer, group: u64, late: Duration) -> RequestClose {
    let mut publication = publisher.publish().await;
    let ended = publisher
        .session
        .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
        .unwrap();
    send_message(&mut publication.send, &ended, &Trace::default(), "publish")
        .await
        .unwrap();
    publication.send.finish();
    tokio::time::sleep(late).await;
    let data = publisher.send_group(group, &[object(0, 3)]).await;
    data.finish_and_wait().await.unwrap();
    match timeout(SOON, publication.reader.next_event()).await {
        Ok(Ok(RequestStreamEvent::Closed(close))) => close,
        other => panic!("the relay did not end the stream: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_ended_its_track_a_thousand_times_is_still_waited_for() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    // 1,024 rounds whose counted stream comes at once: as many waits as a
    // session may have timers, each over as soon as it began.
    for group in 0..1024 {
        let close = publishes_and_ends(&mut publisher, group, Duration::ZERO).await;
        assert_eq!(close, RequestClose::Fin, "round {group}");
    }
    // The next round's stream comes 100 ms after PUBLISH_DONE, within the
    // two seconds of the wait: a FIN, as in the first round.
    let close = publishes_and_ends(&mut publisher, 1024, Duration::from_millis(100)).await;
    assert_eq!(close, RequestClose::Fin);
    // No timer of a wait that is over is left.
    assert_eq!(hub.held().await.timers, 0);
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

#[tokio::test(start_paused = true)]
async fn a_subscribe_namespace_whose_messages_pile_up_is_reset_alone() {
    // Room for one message on a request stream. The SUBSCRIBE_NAMESPACE is
    // answered with REQUEST_OK and a NAMESPACE at once: the second does not fit.
    let config = HubConfig {
        session_queue_len: 1,
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut watcher = Peer::connect(&hub, true).await;
    let prefix = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
    let (_, message) = watcher.session.subscribe_namespace(prefix, vec![]).unwrap();
    let (mut send, recv) = watcher.connection.open_bi().await.unwrap();
    send_message(&mut send, &message, &Trace::default(), "request")
        .await
        .unwrap();
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    // The REQUEST_OK may or may not be written before the reset.
    let end = loop {
        match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(ControlMessage::RequestOk(_)))) => {}
            other => break other,
        }
    };
    // EXCESSIVE_LOAD (0x9), and the session stays.
    assert_eq!(
        end,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Reset(Some(
            0x9
        )))))
    );
    assert_eq!(
        timeout(SOON, send.stopped()).await,
        Ok(WriteError::Stopped(Some(0x9)))
    );
    tokio::time::sleep(SOON).await;
    assert_eq!(watcher.connection.close_info(), None);
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

#[tokio::test(start_paused = true)]
async fn a_peer_that_keeps_the_stream_of_a_refused_request_open_is_stopped() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut subscriber = Peer::connect(&hub, true).await;
    // Nobody has the track: DOES_NOT_EXIST (0x10), and a FIN.
    let mut kept_open = subscriber.start_subscribing().await;
    let answer = subscriber.answer(&mut kept_open).await;
    assert!(
        matches!(&answer, RequestEvent::Rejected(error) if error.error_code == 0x10),
        "{answer:?}"
    );
    let held = hub.held().await;
    assert_eq!((held.relay.requests, held.timers), (1, 1));

    // The subscriber does not end its direction. After two seconds the
    // relay stops it with CANCELLED (0x1) and forgets the request.
    assert!(
        timeout(Duration::from_millis(1900), kept_open.send.stopped())
            .await
            .is_err()
    );
    assert_eq!(
        timeout(SOON, kept_open.send.stopped()).await,
        Ok(WriteError::Stopped(Some(0x1)))
    );
    let held = hub.held().await;
    assert_eq!((held.relay.requests, held.timers), (0, 0));

    // One that ends its direction in time is not waited for any longer.
    let mut ended = subscriber.start_subscribing().await;
    let answer = subscriber.answer(&mut ended).await;
    assert!(matches!(&answer, RequestEvent::Rejected(_)), "{answer:?}");
    ended.send.finish();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let held = hub.held().await;
    assert_eq!(
        (held.relay.requests, held.relay.request_streams, held.timers),
        (0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_ended_its_direction_before_the_relay_did_is_not_timed() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (request, _) = subscriber.subscribe().await;
    let OpenRequest {
        send, mut reader, ..
    } = request;
    // The subscriber finishes its direction, which ends nothing (§5.1).
    send.finish();
    tokio::time::sleep(SOON).await;
    // The publisher's PUBLISH, and the SUBSCRIBE.
    assert_eq!(hub.held().await.relay.requests, 2);

    // The track ends: PUBLISH_DONE and the relay's FIN. Nothing is left to
    // wait for on that stream, and no timer runs for it.
    publisher.connection.close(0, b"");
    let done = timeout(SOON, reader.next_event()).await;
    assert!(
        matches!(
            done,
            Ok(Ok(RequestStreamEvent::Message(
                ControlMessage::PublishDone(_)
            )))
        ),
        "{done:?}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let held = hub.held().await;
    assert_eq!(
        (held.relay.requests, held.relay.request_streams, held.timers),
        (0, 0, 0)
    );
}

/// The lines the relay has logged so far.
fn logged(lines: &mut mpsc::Receiver<String>) -> Vec<String> {
    let mut logged = Vec::new();
    while let Ok(line) = lines.try_recv() {
        logged.push(line);
    }
    logged
}

#[tokio::test(start_paused = true)]
async fn a_stop_of_the_control_stream_is_a_peer_leaving_when_its_connection_ends_right_after() {
    // (how long after the stop the peer's connection ends, whether the relay
    //  closes the session with PROTOCOL_VIOLATION first)
    let cases = [
        (Duration::from_millis(150), false),
        (Duration::from_millis(250), true),
    ];
    for (leaves_after, violation) in cases {
        let (log, mut lines) = hub::Log::collect();
        let (hub, _task) = hub::spawn(HubConfig {
            log: Some(log),
            ..HubConfig::default()
        });
        let peer = Peer::connect(&hub, false).await;
        let control = timeout(SOON, peer.connection.accept_uni())
            .await
            .expect("the relay opens its control stream")
            .unwrap();
        control.stop(0x0);
        tokio::time::sleep(leaves_after).await;
        peer.connection.close(0, b"");
        tokio::time::sleep(Duration::from_secs(5)).await;

        let closed_by_relay = logged(&mut lines)
            .iter()
            .any(|line| line.starts_with("CLOSE_SESSION") && line.contains("code=0x3"));
        assert_eq!(closed_by_relay, violation, "{leaves_after:?}");
        assert_eq!(hub.held().await, NOTHING, "{leaves_after:?}");
    }
}

/// A SUBSCRIBE of the relay that a publisher accepted on its side.
struct RelaySubscription {
    track_alias: u64,
    send: MemorySend,
    reader: RequestReader<MemoryRecv>,
}

impl Peer {
    /// Announces the namespace `(xmoq)`, which the track of these tests is in.
    async fn publish_namespace(&mut self) -> OpenRequest {
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        let (id, message) = self.session.publish_namespace(namespace, vec![]).unwrap();
        let (request, event) = self.request(id, message).await;
        assert_eq!(event, RequestEvent::PublishNamespaceEstablished);
        request
    }

    /// Sends SUBSCRIBE for the track and returns without waiting for the
    /// answer.
    async fn start_subscribing(&mut self) -> OpenRequest {
        let (id, message) = self.session.subscribe(track(), vec![]).unwrap();
        let (mut send, recv) = self.connection.open_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .unwrap();
        let reader = RequestReader::new(recv, &Trace::default(), "request");
        OpenRequest { id, send, reader }
    }

    /// The answer to a request started with [`Self::start_subscribing`].
    async fn answer(&mut self, request: &mut OpenRequest) -> RequestEvent {
        let response = match timeout(SOON, request.reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(response))) => response,
            other => panic!("no response: {other:?}"),
        };
        self.session
            .on_request_message(request.id, response)
            .unwrap()
    }

    /// Takes the SUBSCRIBE the relay sends this peer, without answering it.
    async fn relay_subscribe(&mut self) -> (PeerRequestId, MemorySend, RequestReader<MemoryRecv>) {
        let (send, recv) = timeout(SOON, self.connection.accept_bi())
            .await
            .expect("the relay opens a request stream")
            .unwrap();
        let mut reader = RequestReader::new(recv, &Trace::default(), "relay");
        let first = match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(first))) => first,
            other => panic!("no request: {other:?}"),
        };
        match self.session.on_peer_request_opened(first).unwrap() {
            PeerRequestOpened::Request(id, PeerRequest::Subscribe(subscribe)) => {
                assert_eq!(subscribe.full_track_name, track());
                // §9.2: the relay subscribes upstream with Forward State 1,
                // the value of a SUBSCRIBE without FORWARD (§10.2.12).
                assert_eq!(subscribe.parameters, []);
                (id, send, reader)
            }
            other => panic!("not a SUBSCRIBE: {other:?}"),
        }
    }

    /// Takes the relay's SUBSCRIBE and accepts it.
    async fn accept_relay_subscribe(&mut self) -> RelaySubscription {
        let (id, mut send, reader) = self.relay_subscribe().await;
        let (track_alias, ok) = self.session.accept_subscribe(id, vec![], vec![]).unwrap();
        send_message(&mut send, &ok, &Trace::default(), "relay")
            .await
            .unwrap();
        RelaySubscription {
            track_alias,
            send,
            reader,
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_subscribe_is_routed_to_the_publisher_of_its_namespace() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut alice = Peer::connect(&hub, true).await;
    let mut alice_request = alice.start_subscribing().await;
    let mut bob = Peer::connect(&hub, true).await;
    let mut bob_request = bob.start_subscribing().await;

    // One SUBSCRIBE reaches the publisher for the two of them (§9.4), and
    // nobody is answered before the publisher is (§9.4).
    assert!(
        timeout(SOON, alice_request.reader.next_event())
            .await
            .is_err()
    );
    let upstream = publisher.accept_relay_subscribe().await;
    assert!(
        timeout(SOON, publisher.connection.accept_bi())
            .await
            .is_err()
    );
    let RequestEvent::SubscribeEstablished {
        track_alias: alice_alias,
        ..
    } = alice.answer(&mut alice_request).await
    else {
        panic!("Alice was not accepted");
    };
    let RequestEvent::SubscribeEstablished {
        track_alias: bob_alias,
        ..
    } = bob.answer(&mut bob_request).await
    else {
        panic!("Bob was not accepted");
    };

    publisher
        .send_group_as(upstream.track_alias, 0, &[object(0, 3), object(1, 3)])
        .await
        .finish();
    assert_eq!(alice.receive_stream(alice_alias).await, [0, 1]);
    assert_eq!(bob.receive_stream(bob_alias).await, [0, 1]);
    let held = hub.held().await;
    assert_eq!((held.own_requests, held.relay.own_requests), (1, 1));
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_does_not_answer_the_relay_costs_the_subscriber_a_timeout() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    let (_id, _send, mut reader) = publisher.relay_subscribe().await;

    // Nothing for the five seconds the publisher gets.
    assert!(
        timeout(Duration::from_millis(4900), request.reader.next_event())
            .await
            .is_err()
    );
    // TIMEOUT (0x2) for the subscriber.
    let event = subscriber.answer(&mut request).await;
    assert!(
        matches!(&event, RequestEvent::Rejected(error) if error.error_code == 0x2),
        "{event:?}"
    );
    // The relay cancels its own request: a reset with CANCELLED (0x1).
    assert_eq!(
        timeout(SOON, reader.next_event()).await,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Reset(Some(
            0x1
        )))))
    );
    let held = hub.held().await;
    assert_eq!(
        (
            held.own_requests,
            held.relay.own_requests,
            held.relay.pending_tracks
        ),
        (0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn the_relay_cancels_its_subscribe_when_the_last_subscriber_is_gone() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    let mut upstream = publisher.accept_relay_subscribe().await;
    subscriber.answer(&mut request).await;

    subscriber.connection.close(0, b"");

    // Both directions of the relay's request: a reset and a STOP_SENDING,
    // with CANCELLED (0x1) (§3.3.2).
    assert_eq!(
        timeout(Duration::from_secs(5), upstream.reader.next_event()).await,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Reset(Some(
            0x1
        )))))
    );
    assert_eq!(
        timeout(SOON, upstream.send.stopped()).await,
        Ok(WriteError::Stopped(Some(0x1)))
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    let held = hub.held().await;
    assert_eq!(
        (
            held.sessions,
            held.own_requests,
            held.relay.tracks,
            held.relay.namespaces
        ),
        (1, 0, 0, 1)
    );
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_finishes_its_direction_keeps_getting_objects() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let (request, track_alias) = subscriber.subscribe().await;
    let OpenRequest {
        send, mut reader, ..
    } = request;

    // §5.1: a subscriber ends a subscription with STOP_SENDING, not by
    // finishing its own direction.
    send.finish();
    tokio::time::sleep(SOON).await;
    publisher.send_group(0, &[object(0, 3)]).await.finish();
    assert_eq!(subscriber.receive_stream(track_alias).await, [0]);
    // The relay's direction is still open.
    assert!(timeout(SOON, reader.next_event()).await.is_err());

    // The subscription goes with the session.
    subscriber.connection.close(0, b"");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let held = hub.held().await;
    assert_eq!((held.sessions, held.relay.subscriptions), (1, 0));
}

#[tokio::test(start_paused = true)]
async fn the_relay_finishes_its_subscribe_when_the_publishers_track_is_done() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    let mut upstream = publisher.accept_relay_subscribe().await;
    subscriber.answer(&mut request).await;

    // PUBLISH_DONE that counts one stream, and the publisher's FIN. The
    // relay reads both before the counted stream comes.
    let ended = ControlMessage::PublishDone(done(0x2, 1));
    send_message(&mut upstream.send, &ended, &Trace::default(), "relay")
        .await
        .unwrap();
    upstream.send.finish();
    assert!(timeout(SOON, upstream.reader.next_event()).await.is_err());
    publisher
        .send_group_as(upstream.track_alias, 0, &[object(0, 3)])
        .await
        .finish();

    // Every counted stream has ended now, so the relay answers with a FIN,
    // not with a reset.
    assert_eq!(
        timeout(SOON, upstream.reader.next_event()).await,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Fin)))
    );
    assert_eq!(publish_done(&mut request).await, done(0x2, 1));
    tokio::time::sleep(SOON).await;
    let held = hub.held().await;
    assert_eq!((held.own_requests, held.relay.own_requests), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn a_timer_that_cannot_be_started_does_not_leave_a_subscriber_waiting() {
    // No room for any timer: the wait for the publisher's answer cannot be
    // timed, so it is over at once.
    let config = HubConfig {
        max_timers_per_session: 0,
        ..HubConfig::default()
    };
    let (hub, _task) = hub::spawn(config);
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;

    // TIMEOUT (0x2), without the five seconds.
    let event = subscriber.answer(&mut request).await;
    assert!(
        matches!(&event, RequestEvent::Rejected(error) if error.error_code == 0x2),
        "{event:?}"
    );
    let held = hub.held().await;
    assert_eq!(
        (
            held.own_requests,
            held.relay.own_requests,
            held.relay.pending_tracks
        ),
        (0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn the_timer_for_the_publishers_answer_goes_when_the_answer_comes() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    // The timer that gives the announcement back is gone before the timers
    // are counted.
    tokio::time::sleep(ANNOUNCEMENT_IS_BACK).await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    tokio::time::sleep(SOON).await;
    assert_eq!(hub.held().await.timers, 1);

    let _upstream = publisher.accept_relay_subscribe().await;
    subscriber.answer(&mut request).await;
    assert_eq!(hub.held().await.timers, 0);
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_answers_the_relay_wrongly_or_not_at_all() {
    // (what the publisher puts on the relay's SUBSCRIBE stream, whether its
    //  session is closed with PROTOCOL_VIOLATION (0x3) for it)
    let cases: [(&str, &[u8], bool); 2] = [
        // A FIN and no answer.
        ("a FIN without an answer", &[], false),
        // Message type 0x3f, which no message has.
        ("bytes that are no message", &[0x3f, 0x00, 0x00], true),
    ];
    for (name, bytes, violation) in cases {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut publisher = Peer::connect(&hub, true).await;
        let _namespace = publisher.publish_namespace().await;
        let mut subscriber = Peer::connect(&hub, true).await;
        let mut request = subscriber.start_subscribing().await;
        let (_id, mut send, _reader) = publisher.relay_subscribe().await;

        send.write_all(bytes).await.unwrap();
        send.finish();

        // DOES_NOT_EXIST (0x10) for the subscriber, either way.
        let event = subscriber.answer(&mut request).await;
        assert!(
            matches!(&event, RequestEvent::Rejected(error) if error.error_code == 0x10),
            "{name}: {event:?}"
        );
        tokio::time::sleep(SOON).await;
        assert_eq!(
            publisher.connection.close_info().map(|close| close.code),
            violation.then_some(0x3),
            "{name}"
        );
        request_close_time().await;
        let held = hub.held().await;
        assert_eq!(
            (held.own_requests, held.relay.own_requests, held.timers),
            (0, 0, 0),
            "{name}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_publish_that_crosses_the_relays_subscribe_becomes_the_source_of_the_track() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    tokio::time::sleep(SOON).await;
    assert_eq!(hub.held().await.own_requests, 1);

    // The publisher sends its PUBLISH before it has read the relay's
    // SUBSCRIBE, and the relay accepts it (§5.1).
    let _publication = publisher.publish().await;

    // The relay's SUBSCRIBE gave way: its stream is reset with CANCELLED
    // (0x1), whether or not the SUBSCRIBE on it is still read first.
    let (_send, recv) = timeout(SOON, publisher.connection.accept_bi())
        .await
        .expect("the relay opened a request stream")
        .unwrap();
    let mut reader = RequestReader::new(recv, &Trace::default(), "relay");
    let closed = loop {
        match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(_))) => {}
            other => break other,
        }
    };
    assert_eq!(
        closed,
        Ok(Ok(RequestStreamEvent::Closed(RequestClose::Reset(Some(
            0x1
        )))))
    );

    // The subscriber that waited is served from the PUBLISH.
    let RequestEvent::SubscribeEstablished { track_alias, .. } =
        subscriber.answer(&mut request).await
    else {
        panic!("the subscriber was not accepted");
    };
    publisher
        .send_objects(&[object(0, 3), object(1, 3)])
        .await
        .finish();
    assert_eq!(subscriber.receive_stream(track_alias).await, [0, 1]);
    let held = hub.held().await;
    assert_eq!(
        (held.own_requests, held.relay.own_requests, held.timers),
        (0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_second_message_after_duplicate_subscription_does_not_end_the_wait_for_the_publish() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    // The timer that gives the announcement back is gone before the timers
    // are counted.
    tokio::time::sleep(ANNOUNCEMENT_IS_BACK).await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    let (_id, mut send, _reader) = publisher.relay_subscribe().await;

    // DUPLICATE_SUBSCRIPTION (0x19), and a second message right behind it in
    // one write, so that the relay reads both before it has dealt with the
    // first. DOES_NOT_EXIST (0x10) would refuse the subscriber if it counted.
    let refusal = |error_code| {
        ControlMessage::RequestError(RequestError {
            error_code,
            retry_interval: 0,
            error_reason: ReasonPhrase::default(),
            redirect: None,
        })
    };
    let mut bytes = Vec::new();
    refusal(0x19).encode(&mut bytes).unwrap();
    refusal(0x10).encode(&mut bytes).unwrap();
    send.write_all(&bytes).await.unwrap();
    send.finish();

    // The wait for the PUBLISH is timed, by one timer, and the subscriber is
    // not answered while it runs.
    tokio::time::sleep(SOON).await;
    let held = hub.held().await;
    assert_eq!(
        (
            held.timers,
            held.relay.pending_tracks,
            held.relay.own_requests
        ),
        (1, 1, 1)
    );
    assert!(
        timeout(Duration::from_secs(3), request.reader.next_event())
            .await
            .is_err()
    );
    // No PUBLISH came in the five seconds: TIMEOUT (0x2), and nothing is
    // left.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let event = subscriber.answer(&mut request).await;
    assert!(
        matches!(&event, RequestEvent::Rejected(error) if error.error_code == 0x2),
        "{event:?}"
    );
    request_close_time().await;
    let held = hub.held().await;
    assert_eq!(
        (
            held.timers,
            held.own_requests,
            held.relay.pending_tracks,
            held.relay.own_requests
        ),
        (0, 0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn the_wait_for_an_announced_publish_ends_when_the_publish_comes() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    // The timer that gives the announcement back is gone before the timers
    // are counted.
    tokio::time::sleep(ANNOUNCEMENT_IS_BACK).await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let mut request = subscriber.start_subscribing().await;
    // The publisher sends its PUBLISH without having read the relay's
    // SUBSCRIBE, and answers that one with DUPLICATE_SUBSCRIPTION (0x19) by
    // hand, as §5.1 has it: here the answer is read first.
    let (mut send, _recv) = timeout(SOON, publisher.connection.accept_bi())
        .await
        .expect("the relay opened a request stream")
        .unwrap();
    let duplicate = ControlMessage::RequestError(RequestError {
        error_code: 0x19,
        retry_interval: 0,
        error_reason: ReasonPhrase::default(),
        redirect: None,
    });
    send_message(&mut send, &duplicate, &Trace::default(), "relay")
        .await
        .unwrap();
    send.finish();
    tokio::time::sleep(SOON).await;
    assert_eq!(hub.held().await.timers, 1);

    let _publication = publisher.publish().await;
    let RequestEvent::SubscribeEstablished { .. } = subscriber.answer(&mut request).await else {
        panic!("the subscriber was not accepted");
    };
    // The timer went with the wait.
    let held = hub.held().await;
    assert_eq!((held.timers, held.relay.own_requests), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn the_code_of_a_reset_and_of_a_stop_of_a_request_stream_is_logged() {
    // (how the subscriber cancels its SUBSCRIBE, the line the relay logs for
    //  it). A peer that cancels sends both (§3.3.2), and the relay acts on
    //  the one it sees first; each has its line, with the peer's code.
    //  TOO_FAR_BEHIND (0x5) and MALFORMED_TRACK (0x12) are codes the relay
    //  does not send by itself here. The subscriber is the relay's second
    //  session, and its SUBSCRIBE its second stream: session 1, stream 1.
    let cases = [
        (true, "RESET_RECEIVED session=1 stream=1 code=0x5"),
        (false, "STOP_SENDING_RECEIVED session=1 stream=1 code=0x12"),
    ];
    for (resets, line) in cases {
        let (log, mut lines) = hub::Log::collect();
        let (hub, _task) = hub::spawn(HubConfig {
            log: Some(log),
            ..HubConfig::default()
        });
        let mut publisher = Peer::connect(&hub, true).await;
        let _publication = publisher.publish().await;
        let mut subscriber = Peer::connect(&hub, true).await;
        let (request, _alias) = subscriber.subscribe().await;
        let OpenRequest { send, reader, .. } = request;
        // The half that is not used stays open: letting go of it would say
        // something to the relay as well.
        let (mut send_kept, mut reader_kept) = (None, None);
        if resets {
            send.reset(0x5);
            reader_kept = Some(reader);
        } else {
            reader.stop(0x12);
            send_kept = Some(send);
        }
        tokio::time::sleep(SOON).await;
        let received: Vec<String> = logged(&mut lines)
            .into_iter()
            .filter(|logged| logged.contains("_RECEIVED "))
            .collect();
        assert_eq!(received, [line]);
        assert_eq!(hub.held().await.relay.subscriptions, 0, "{line}");
        drop((send_kept, reader_kept));
    }
}

/// The request stream of a FETCH the relay sent this peer.
struct RelayFetch {
    id: PeerRequestId,
    send: MemorySend,
    reader: RequestReader<MemoryRecv>,
}

fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

impl Peer {
    /// Sends a Standalone FETCH for {0, 0} to {2, 0} of the track and returns
    /// without waiting for the answer.
    async fn start_fetching(&mut self, parameters: Vec<Parameter>) -> OpenRequest {
        let range = FetchRange::Standalone {
            full_track_name: track(),
            start: at(0, 0),
            end: at(2, 0),
        };
        let (id, message) = self.session.fetch(range, parameters).unwrap();
        let (mut send, recv) = self.connection.open_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .unwrap();
        let reader = RequestReader::new(recv, &Trace::default(), "request");
        OpenRequest { id, send, reader }
    }

    /// Takes the FETCH the relay sends this peer, without answering it.
    async fn relay_fetch(&mut self) -> RelayFetch {
        let (send, recv) = timeout(SOON, self.connection.accept_bi())
            .await
            .expect("the relay opens a request stream")
            .unwrap();
        let mut reader = RequestReader::new(recv, &Trace::default(), "relay");
        let first = match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(first))) => first,
            other => panic!("no request: {other:?}"),
        };
        match self.session.on_peer_request_opened(first).unwrap() {
            PeerRequestOpened::Request(id, PeerRequest::Fetch(fetch)) => {
                let FetchRange::Standalone {
                    full_track_name,
                    start,
                    end,
                } = fetch.range
                else {
                    panic!("not a Standalone FETCH");
                };
                assert_eq!((full_track_name, start, end), (track(), at(0, 0), at(2, 0)));
                RelayFetch { id, send, reader }
            }
            other => panic!("not a FETCH: {other:?}"),
        }
    }

    /// Answers the relay's FETCH with `FETCH_OK`, End Location {2, 0}.
    async fn accept_relay_fetch(&mut self, fetch: &mut RelayFetch) {
        let (ok, _) = self
            .session
            .accept_fetch(fetch.id, false, at(2, 0), vec![])
            .unwrap();
        send_message(&mut fetch.send, &ok, &Trace::default(), "relay")
            .await
            .unwrap();
    }

    /// Opens the fetch stream that answers the relay's FETCH.
    async fn open_fetch_stream(&self, fetch: &RelayFetch) -> FetchStreamWriter<MemorySend> {
        let header = FetchHeader {
            request_id: fetch.id.value(),
        };
        open_fetch_stream(&*self.connection, header, &Trace::default(), "fetch", 0x1)
            .await
            .unwrap()
    }

    /// The fetch stream the relay opens for `request`: what it carries.
    async fn fetch_body(&mut self, request: &OpenRequest) -> FetchBody {
        match self.next_data().await {
            UniEvent::FetchOpened { stream, header } => {
                assert_eq!(header.request_id, request.id.value());
                self.session.on_fetch_stream(header.request_id).unwrap();
                self.stops.take_fetch_body(stream).expect("the body")
            }
            other => panic!("not a fetch stream: {other:?}"),
        }
    }
}

/// Reads a fetch stream to its end: the bytes, and the end.
async fn read_fetch(body: &mut FetchBody) -> (Vec<u8>, FetchChunk) {
    let mut bytes = Vec::new();
    loop {
        match timeout(Duration::from_secs(60), body.next())
            .await
            .expect("a chunk in time")
        {
            FetchChunk::Bytes(chunk) => bytes.extend_from_slice(&chunk),
            end => return (bytes, end),
        }
    }
}

async fn request_end(reader: &mut RequestReader<MemoryRecv>) -> RequestStreamEvent {
    timeout(Duration::from_secs(60), reader.next_event())
        .await
        .expect("the stream ends in time")
        .expect("the stream is read")
}

/// A relay with a publisher of the track and a subscriber that has sent a
/// FETCH, which the relay has passed on to the publisher.
async fn fetching(
    log: Option<hub::Log>,
    reads: bool,
) -> (
    HubHandle<MemoryConnection>,
    Peer,
    Peer,
    OpenRequest,
    RelayFetch,
) {
    let (hub, _task) = hub::spawn(HubConfig {
        log,
        ..HubConfig::default()
    });
    let mut publisher = Peer::connect(&hub, true).await;
    let publication = publisher.publish().await;
    // Kept open by the peer's connection; the track goes on without the
    // handle.
    std::mem::forget(publication);
    let mut alice = Peer::connect(&hub, reads).await;
    let request = alice.start_fetching(vec![]).await;
    let upstream = publisher.relay_fetch().await;
    (hub, publisher, alice, request, upstream)
}

#[tokio::test(start_paused = true)]
async fn a_fetch_is_passed_on_to_the_publisher_and_its_stream_back() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, mut publisher, mut alice, mut request, mut upstream) =
        fetching(Some(log), true).await;
    // Nobody is answered before the publisher is (§9.4).
    assert!(timeout(SOON, request.reader.next_event()).await.is_err());

    publisher.accept_relay_fetch(&mut upstream).await;
    assert_eq!(
        alice.answer(&mut request).await,
        RequestEvent::FetchEstablished {
            end_of_track: false,
            end_location: at(2, 0),
            track_properties: vec![],
        }
    );
    // More than the relay or a stream window holds at once, and no Objects:
    // the relay does not look at what it passes on.
    let sent: Vec<u8> = (0..300_000u32)
        .map(|index| index.to_le_bytes()[0])
        .collect();
    let mut stream = publisher.open_fetch_stream(&upstream).await;
    let expected = sent.clone();
    let writing = tokio::spawn(async move {
        stream.write(&sent).await.unwrap();
        stream.finish();
    });
    let mut body = alice.fetch_body(&request).await;
    let (received, end) = read_fetch(&mut body).await;
    assert_eq!(end, FetchChunk::End(StreamEnd::Fin));
    assert!(received == expected, "{} bytes", received.len());
    writing.await.unwrap();

    // The relay ends both request streams with a FIN, and holds nothing.
    assert_eq!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Closed(RequestClose::Fin)
    );
    assert_eq!(
        request_end(&mut upstream.reader).await,
        RequestStreamEvent::Closed(RequestClose::Fin)
    );
    let held = hub.held().await;
    assert_eq!(
        (
            held.fetch_pipes,
            held.relay.fetches,
            held.own_requests,
            held.relay.own_requests
        ),
        (0, 0, 0, 0)
    );
    // Alice is the relay's second session, 1. The streams of a session are
    // numbered as the relay comes to read them: her FETCH is her second,
    // after her control stream, and the publisher's fetch stream its third,
    // after its control stream and its PUBLISH.
    let lines = logged(&mut lines);
    for line in [
        "OPEN_REQUEST session=0 own=0 FETCH",
        "RECV session=0 own=0 FETCH_OK",
        "PIPE_FETCH session=1 stream=1 request_id=0 from_session=0 from_stream=2",
        "FETCH_PIPE_ENDED session=1 stream=1 finished",
    ] {
        assert!(
            lines.iter().any(|logged| logged == line),
            "{line}: {lines:#?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_priority_in_an_update_of_a_fetch_moves_the_stream_that_is_passed_on() {
    let (log, mut lines) = hub::Log::collect();
    let (_hub, mut publisher, mut alice, mut request, mut upstream) =
        fetching(Some(log), true).await;
    publisher.accept_relay_fetch(&mut upstream).await;
    alice.answer(&mut request).await;
    let mut stream = publisher.open_fetch_stream(&upstream).await;
    stream.write(&[1; 100]).await.unwrap();
    let mut body = alice.fetch_body(&request).await;
    // SUBSCRIBER_PRIORITY 3 while the stream is passed on: answered by the
    // relay, and nothing reaches the publisher (§10.2.1).
    let update = ControlMessage::RequestUpdate(RequestUpdate {
        request_id: request.id.value() + 2,
        parameters: vec![Parameter::SubscriberPriority(3)],
    });
    send_message(&mut request.send, &update, &Trace::default(), "request")
        .await
        .unwrap();
    assert_eq!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Message(ControlMessage::RequestOk(
            xmoq_moqt::message::RequestOk::default()
        ))
    );
    assert!(
        timeout(SOON, upstream.reader.next_event()).await.is_err(),
        "the publisher was told"
    );
    stream.write(&[2; 100]).await.unwrap();
    stream.finish();
    let (received, end) = read_fetch(&mut body).await;
    assert_eq!(
        (received.len(), end),
        (200, FetchChunk::End(StreamEnd::Fin))
    );
    // -1 - (3 << 8 | 128).
    let line = "FETCH_SEND_ORDER session=1 stream=1 send_order=-897";
    let lines = logged(&mut lines);
    assert!(lines.iter().any(|logged| logged == line), "{lines:#?}");
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_does_not_read_its_fetch_holds_the_publisher_back_and_is_cut_off() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, mut publisher, alice, mut request, mut upstream) = fetching(Some(log), false).await;
    publisher.accept_relay_fetch(&mut upstream).await;
    // Alice reads her request stream, and no data stream.
    assert!(matches!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Message(ControlMessage::FetchOk(_))
    ));
    let mut stream = publisher.open_fetch_stream(&upstream).await;
    let writing = tokio::spawn(async move { stream.write(&vec![0x5a; 1024 * 1024]).await });

    // Alice takes nothing. The publisher is held back, not read into memory.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!writing.is_finished(), "the whole stream was taken");
    assert_eq!(hub.held().await.fetch_pipes, 1);

    // After 10 seconds without progress the relay gives the FETCH up: the
    // publisher's stream is stopped and its FETCH cancelled, both with
    // CANCELLED (0x1), and Alice's request stream is reset with
    // TOO_FAR_BEHIND (0x5).
    assert_eq!(
        timeout(Duration::from_secs(60), writing)
            .await
            .unwrap()
            .unwrap(),
        Err(WriteError::Stopped(Some(0x1)))
    );
    assert_eq!(
        request_end(&mut upstream.reader).await,
        RequestStreamEvent::Closed(RequestClose::Reset(Some(0x1)))
    );
    assert_eq!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Closed(RequestClose::Reset(Some(0x5)))
    );
    // The fetch stream she never read was reset with the same code, and not
    // left to end as if it had come whole.
    // The first stream the relay opened to her is its control stream.
    let _control = timeout(SOON, alice.connection.accept_uni())
        .await
        .expect("the relay's control stream")
        .unwrap();
    let mut unread = timeout(SOON, alice.connection.accept_uni())
        .await
        .expect("the relay opened it")
        .unwrap();
    loop {
        match unread.read(&mut [0; 64]).await {
            Ok(Some(_)) => {}
            end => {
                assert_eq!(end, Err(ReadError::Reset(Some(0x5))));
                break;
            }
        }
    }
    let held = hub.held().await;
    assert_eq!(
        (held.fetch_pipes, held.relay.fetches, held.own_requests),
        (0, 0, 0)
    );
    let lines = logged(&mut lines);
    let line = "FETCH_PIPE_ENDED session=1 stream=1 downstream_stalled";
    assert!(lines.iter().any(|logged| logged == line), "{lines:#?}");
}

/// What happens to a FETCH after the publisher answered, opened its stream
/// and sent two bytes.
#[derive(Debug, Clone, Copy)]
enum Then {
    /// It sends nothing more.
    PublisherStalls,
    /// It resets its stream with `UNKNOWN_OBJECT_STATUS` (0x6).
    PublisherResets,
    /// It disconnects.
    PublisherLeaves,
    /// Alice stops the fetch stream with CANCELLED (0x1).
    SubscriberStops,
    /// Alice resets the request stream of her FETCH.
    SubscriberCancels,
}

#[tokio::test(start_paused = true)]
async fn the_ways_a_passed_on_fetch_is_cut_short() {
    // (what happens; how Alice's fetch stream ends, where she still reads it;
    //  how her request stream ends; the line of the log)
    let reset = |code| FetchChunk::End(StreamEnd::Aborted(Some(code)));
    let cases = [
        (
            Then::PublisherStalls,
            Some(reset(0x6)),
            Some(RequestClose::Fin),
            "FETCH_PIPE_ENDED session=1 stream=1 upstream_stalled",
        ),
        (
            Then::PublisherResets,
            Some(reset(0x6)),
            Some(RequestClose::Fin),
            "FETCH_PIPE_ENDED session=1 stream=1 upstream_reset code=0x6",
        ),
        (
            Then::PublisherLeaves,
            Some(reset(0x6)),
            Some(RequestClose::Fin),
            "ABORT_FETCH_PIPE session=1 stream=1 code=0x6",
        ),
        (
            Then::SubscriberStops,
            None,
            Some(RequestClose::Reset(Some(0x1))),
            "FETCH_PIPE_ENDED session=1 stream=1 downstream_stopped code=0x1",
        ),
        (
            Then::SubscriberCancels,
            Some(reset(0x1)),
            None,
            "ABORT_FETCH_PIPE session=1 stream=1 code=0x1",
        ),
    ];
    for (then, stream_end, request_close, line) in cases {
        let (log, mut lines) = hub::Log::collect();
        let (hub, mut publisher, mut alice, mut request, mut upstream) =
            fetching(Some(log), true).await;
        publisher.accept_relay_fetch(&mut upstream).await;
        alice.answer(&mut request).await;
        let mut stream = publisher.open_fetch_stream(&upstream).await;
        stream.write(&[0xaa, 0xbb]).await.unwrap();
        let mut stream = Some(stream);
        let mut body = Some(alice.fetch_body(&request).await);
        if let Some(body) = &mut body {
            assert_eq!(
                body.next().await,
                FetchChunk::Bytes(vec![0xaa, 0xbb]),
                "{then:?}"
            );
        }

        let mut publisher = Some(publisher);
        match then {
            Then::PublisherStalls => {}
            Then::PublisherResets => {
                if let Some(stream) = stream.take() {
                    stream.reset(0x6);
                }
            }
            Then::PublisherLeaves => drop(publisher.take()),
            // Dropping what a fetch stream carries stops the stream.
            Then::SubscriberStops => body = None,
            Then::SubscriberCancels => request.send.reset(0x1),
        }
        if let (Some(expected), Some(body)) = (stream_end, &mut body) {
            assert_eq!(read_fetch(body).await, (vec![], expected), "{then:?}");
        }
        if let Some(expected) = request_close {
            assert_eq!(
                request_end(&mut request.reader).await,
                RequestStreamEvent::Closed(expected),
                "{then:?}"
            );
        }
        // The publisher is not left sending into nothing: its stream is
        // stopped, and the relay's FETCH is over.
        if let (Some(stream), true) = (&mut stream, publisher.is_some()) {
            assert_eq!(
                timeout(Duration::from_secs(60), stream.stopped()).await,
                Ok(WriteError::Stopped(Some(0x1))),
                "{then:?}"
            );
        }
        tokio::time::sleep(SOON).await;
        let held = hub.held().await;
        assert_eq!(
            (
                held.fetch_pipes,
                held.relay.fetches,
                held.own_requests,
                held.relay.own_requests
            ),
            (0, 0, 0, 0),
            "{then:?}"
        );
        let lines = logged(&mut lines);
        assert!(
            lines.iter().any(|logged| logged == line),
            "{then:?}: {lines:#?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_does_not_answer_a_fetch_costs_the_subscriber_a_timeout() {
    let (hub, _publisher, mut alice, mut request, upstream) = fetching(None, true).await;
    // Nothing within the 5 seconds a publisher has; then TIMEOUT (0x2).
    assert!(
        timeout(Duration::from_secs(4), request.reader.next_event())
            .await
            .is_err()
    );
    let RequestStreamEvent::Message(response) = request_end(&mut request.reader).await else {
        panic!("no answer");
    };
    let answer = alice.session.on_request_message(request.id, response);
    assert!(
        matches!(&answer, Ok(RequestEvent::Rejected(error)) if error.error_code == 0x2),
        "{answer:?}"
    );
    drop(upstream);
    tokio::time::sleep(SOON).await;
    request_close_time().await;
    let held = hub.held().await;
    assert_eq!(
        (held.relay.fetches, held.own_requests, held.timers),
        (0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_fetch_that_waits_for_nothing_gets_a_stream_that_says_unknown() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let (id, message) = publisher
        .session
        .publish(
            track(),
            PUBLISHER_ALIAS,
            vec![Parameter::LargestObject(at(1, 4))],
            vec![],
        )
        .unwrap();
    let _publication = publisher.request(id, message).await;
    let mut alice = Peer::connect(&hub, true).await;
    let mut request = alice.start_fetching(vec![Parameter::FillTimeout(0)]).await;

    // The range {0, 0} to {2, 0} is cut at the largest Location, {1, 4}.
    assert_eq!(
        alice.answer(&mut request).await,
        RequestEvent::FetchEstablished {
            end_of_track: false,
            end_location: at(1, 5),
            track_properties: vec![],
        }
    );
    // One End of Unknown Range indicator (0x10C) at {1, 4}, and a FIN.
    let mut body = alice.fetch_body(&request).await;
    assert_eq!(
        read_fetch(&mut body).await,
        (
            vec![0x81, 0x0c, 0x01, 0x04],
            FetchChunk::End(StreamEnd::Fin)
        )
    );
    assert_eq!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Closed(RequestClose::Fin)
    );
    // The publisher was not asked, and nothing is left of the FETCH.
    assert!(
        timeout(SOON, publisher.connection.accept_bi())
            .await
            .is_err()
    );
    assert_eq!(hub.held().await.relay.fetches, 0);
}

#[tokio::test(start_paused = true)]
async fn nothing_is_left_of_a_fetch_whose_sessions_disconnected() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    tokio::time::sleep(SOON).await;
    let tasks_before = alive_tasks();
    {
        let mut publisher = Peer::connect(&hub, true).await;
        let publication = publisher.publish().await;
        let mut alice = Peer::connect(&hub, false).await;
        let request = alice.start_fetching(vec![]).await;
        let mut upstream = publisher.relay_fetch().await;
        publisher.accept_relay_fetch(&mut upstream).await;
        let mut stream = publisher.open_fetch_stream(&upstream).await;
        // Stuck in the window of a subscriber that does not read.
        let writing = tokio::spawn(async move { stream.write(&vec![0; 1024 * 1024]).await });
        tokio::time::sleep(SOON).await;
        assert_eq!(hub.held().await.fetch_pipes, 1);
        writing.abort();
        drop((publication, request, upstream, publisher, alice));
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(hub.held().await, NOTHING);
    assert_eq!(alive_tasks(), tasks_before);
}

#[tokio::test(start_paused = true)]
async fn a_fetch_that_goes_on_slowly_is_not_cut() {
    // A publisher that sends a byte every 4 seconds, five times: 20 seconds
    // in all, and never 5 seconds without a byte.
    let (_hub, mut publisher, mut alice, mut request, mut upstream) = fetching(None, true).await;
    publisher.accept_relay_fetch(&mut upstream).await;
    alice.answer(&mut request).await;
    let mut stream = publisher.open_fetch_stream(&upstream).await;
    let writing = tokio::spawn(async move {
        for byte in 1..=5u8 {
            tokio::time::sleep(Duration::from_secs(4)).await;
            stream.write(&[byte]).await.unwrap();
        }
        stream.finish();
    });
    let mut body = alice.fetch_body(&request).await;
    assert_eq!(
        read_fetch(&mut body).await,
        (vec![1, 2, 3, 4, 5], FetchChunk::End(StreamEnd::Fin))
    );
    writing.await.unwrap();

    // A subscriber that takes a little every half second: 4000 bytes take
    // far longer than the 10 seconds it has for each piece, and no piece
    // takes that long.
    let (_hub, mut publisher, mut alice, mut request, mut upstream) = fetching(None, true).await;
    publisher.accept_relay_fetch(&mut upstream).await;
    alice.answer(&mut request).await;
    let mut stream = publisher.open_fetch_stream(&upstream).await;
    let writing = tokio::spawn(async move {
        stream.write(&[0x5a; 4000]).await.unwrap();
        stream.finish();
    });
    let mut body = alice.fetch_body(&request).await;
    let started = tokio::time::Instant::now();
    let mut received = 0;
    let end = loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        match body.next().await {
            FetchChunk::Bytes(chunk) => received += chunk.len(),
            end => break end,
        }
    };
    assert_eq!((received, end), (4000, FetchChunk::End(StreamEnd::Fin)));
    assert!(
        started.elapsed() > Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    writing.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_publisher_that_answers_a_fetch_and_opens_no_stream_costs_the_subscriber_the_fetch() {
    let (hub, mut publisher, mut alice, mut request, mut upstream) = fetching(None, true).await;
    publisher.accept_relay_fetch(&mut upstream).await;
    alice.answer(&mut request).await;
    // Nothing for the 5 seconds a publisher has to open its stream; then the
    // request stream is reset with UNKNOWN_OBJECT_STATUS (0x6): no fetch
    // stream will come.
    assert!(
        timeout(Duration::from_secs(4), request.reader.next_event())
            .await
            .is_err()
    );
    assert_eq!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Closed(RequestClose::Reset(Some(0x6)))
    );
    // The relay's FETCH is cancelled, with CANCELLED (0x1).
    assert_eq!(
        request_end(&mut upstream.reader).await,
        RequestStreamEvent::Closed(RequestClose::Reset(Some(0x1)))
    );
    let held = hub.held().await;
    assert_eq!(
        (held.relay.fetches, held.own_requests, held.timers),
        (0, 0, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn fetches_nobody_reads_do_not_keep_the_publisher_from_publishing() {
    // Two sessions take the 16 FETCH requests the relay has for a publisher
    // between them, 8 each: neither the limit of a session's own FETCHes (16)
    // nor that of one subscriber's at a publisher is the one looked at.
    let (hub, _task) = hub::spawn(HubConfig {
        fetches_for_one_subscriber: Some(16),
        ..HubConfig::default()
    });
    // The relay lets the publisher have as many unidirectional streams open
    // as this test needs and no more: its control stream, the 16 fetch
    // streams that get stuck, and the subgroup stream below. A stuck stream
    // keeps its place, as on a real connection, so that one more of them
    // would keep the subgroup stream out.
    let (local, remote) = pair_with_stream_limit(18);
    let mut publisher = Peer::connect_over(&hub, local, remote, true, Arrival::First).await;
    let _publication = publisher.publish().await;
    let mut bob = Peer::connect(&hub, true).await;
    let (_bob_request, bob_alias) = bob.subscribe().await;
    // Alice and Carol ask for 8 FETCHes each, read none of them, and the
    // publisher answers each with a stream that gets stuck.
    let mut alice = Peer::connect(&hub, false).await;
    let mut carol = Peer::connect(&hub, false).await;
    let mut stuck = Vec::new();
    for asked in 0..16 {
        let asking = if asked % 2 == 0 {
            &mut alice
        } else {
            &mut carol
        };
        let request = asking.start_fetching(vec![]).await;
        let mut upstream = publisher.relay_fetch().await;
        publisher.accept_relay_fetch(&mut upstream).await;
        let mut stream = publisher.open_fetch_stream(&upstream).await;
        let writing = tokio::spawn(async move { stream.write(&vec![0; 64 * 1024]).await });
        stuck.push((request, upstream, writing));
    }
    tokio::time::sleep(SOON).await;
    assert_eq!(hub.held().await.fetch_pipes, 16);
    // One more is refused for the publisher's limit (16 of the relay's own
    // FETCHes on its session), though Alice has 8: EXCESSIVE_LOAD (0x9).
    let mut one_more = alice.start_fetching(vec![]).await;
    let RequestStreamEvent::Message(response) = request_end(&mut one_more.reader).await else {
        panic!("no answer");
    };
    let answer = alice.session.on_request_message(one_more.id, response);
    assert!(
        matches!(&answer, Ok(RequestEvent::Rejected(error)) if error.error_code == 0x9),
        "{answer:?}"
    );
    // The publisher's subgroup stream is read all the same, and reaches Bob
    // at once: not once the relay has given up a stuck FETCH, whose stream
    // would make room for it. WHY a time is no race: on the paused clock of
    // this test no time passes but what the tasks wait for, and the relay
    // gives up a FETCH later than SOON.
    let sending = async {
        publisher.send_objects(&[object(0, 3)]).await.finish();
        bob.receive_stream(bob_alias).await
    };
    assert_eq!(timeout(SOON, sending).await.expect("in time"), [0]);
}

#[tokio::test(start_paused = true)]
async fn a_publisher_has_the_fill_timeout_to_open_its_stream_after_it_answered() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut alice = Peer::connect(&hub, true).await;
    let mut request = alice
        .start_fetching(vec![Parameter::FillTimeout(20_000)])
        .await;
    let mut upstream = publisher.relay_fetch().await;
    publisher.accept_relay_fetch(&mut upstream).await;
    alice.answer(&mut request).await;
    // 6 seconds after its FETCH_OK, which is more than the 5 a FETCH without
    // FILL_TIMEOUT gets, the publisher opens its stream; it is passed on.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let mut stream = publisher.open_fetch_stream(&upstream).await;
    stream.write(&[7]).await.unwrap();
    stream.finish();
    let mut body = alice.fetch_body(&request).await;
    assert_eq!(
        read_fetch(&mut body).await,
        (vec![7], FetchChunk::End(StreamEnd::Fin))
    );
}

#[tokio::test(start_paused = true)]
async fn a_subscriber_that_takes_no_stream_is_told_that_none_comes() {
    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let (id, message) = publisher
        .session
        .publish(
            track(),
            PUBLISHER_ALIAS,
            vec![Parameter::LargestObject(at(1, 4))],
            vec![],
        )
        .unwrap();
    let _publication = publisher.request(id, message).await;
    // Alice accepts no unidirectional stream. The connection holds four that
    // are not accepted: the relay's control stream and three fetch streams.
    let mut alice = Peer::connect(&hub, false).await;
    let mut requests = Vec::new();
    for _ in 0..6 {
        let mut request = alice.start_fetching(vec![Parameter::FillTimeout(0)]).await;
        assert!(matches!(
            alice.answer(&mut request).await,
            RequestEvent::FetchEstablished { .. }
        ));
        requests.push(request);
    }
    // Three were written, and their request streams finished. For the other
    // three the relay waits 10 seconds for a stream to write on, and then
    // resets the request stream with TOO_FAR_BEHIND (0x5): FETCH_OK was
    // sent, and no fetch stream comes.
    let mut ends = Vec::new();
    for request in &mut requests {
        ends.push(request_end(&mut request.reader).await);
    }
    let fin = || RequestStreamEvent::Closed(RequestClose::Fin);
    let reset = || RequestStreamEvent::Closed(RequestClose::Reset(Some(0x5)));
    assert_eq!(ends, [fin(), fin(), fin(), reset(), reset(), reset()]);
    assert_eq!(hub.held().await.relay.fetches, 0);
}

#[tokio::test(start_paused = true)]
async fn a_subscribe_that_waits_for_a_publisher_is_logged_as_held() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let mut publisher = Peer::connect(&hub, true).await;
    let _namespace = publisher.publish_namespace().await;
    // The publisher announced the namespace and has not answered for the
    // track: the subscriber's SUBSCRIBE waits.
    let mut waiting = Peer::connect(&hub, true).await;
    let _request = waiting.start_subscribing().await;
    let _upstream = publisher.relay_subscribe().await;
    let lines = logged(&mut lines);
    assert!(
        lines.contains(&"HOLD_REQUEST session=1 stream=1 SUBSCRIBE".to_string()),
        "{lines:?}"
    );

    // A SUBSCRIBE that is answered in the step that reads it is not.
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    let mut subscriber = Peer::connect(&hub, true).await;
    let _subscription = subscriber.subscribe().await;
    let lines = logged(&mut lines);
    assert!(
        !lines.iter().any(|line| line.starts_with("HOLD_REQUEST")),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_subscribe_that_closes_its_session_is_not_logged_as_held() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let _client = Peer::connect(&hub, true).await;
    // A SUBSCRIBE with a Request ID of the relay's parity, 1: the session
    // is closed with INVALID_REQUEST_ID, and nothing waits.
    hub.inject(Event::RequestOpened {
        session: SessionKey(0),
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 1,
            full_track_name: track(),
            parameters: vec![],
        }),
    })
    .await;
    assert_eq!(hub.held().await.relay.sessions, 0);
    let lines = logged(&mut lines);
    assert!(
        !lines.iter().any(|line| line.starts_with("HOLD_REQUEST")),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_end_of_a_fetch_that_is_not_passed_on_is_dropped() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let _client = Peer::connect(&hub, true).await;
    // What a task reports that ended while the relay ordered it stopped:
    // the hub has let go of the FETCH by then.
    hub.inject(Event::FetchPipeEnded {
        session: SessionKey(0),
        stream: StreamKey(0),
        how: FetchPipeEnd::UpstreamLost,
    })
    .await;
    let held = hub.held().await;
    assert_eq!((held.relay.sessions, held.fetch_pipes), (1, 0));
    let lines = logged(&mut lines);
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("FETCH_PIPE_ENDED")),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_fetch_whose_stream_cannot_be_passed_on_ends_for_the_subscriber() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, mut publisher, mut alice, mut request, mut upstream) =
        fetching(Some(log), true).await;
    publisher.accept_relay_fetch(&mut upstream).await;
    assert!(matches!(
        alice.answer(&mut request).await,
        RequestEvent::FetchEstablished { .. }
    ));
    // The header of the publisher's fetch stream is reported for a stream
    // the hub has no bytes of: there is nothing to pass on, and the relay
    // has to hear that, or it holds the FETCH for good.
    hub.inject(Event::FetchStreamOpened {
        session: SessionKey(0),
        stream: StreamKey(999),
        request_id: upstream.id.value(),
    })
    .await;
    let end = request_end(&mut request.reader).await;
    assert_eq!(end, RequestStreamEvent::Closed(RequestClose::Fin));
    let held = hub.held().await;
    assert_eq!((held.relay.fetches, held.fetch_pipes), (0, 0));
    let lines = logged(&mut lines);
    assert!(
        lines
            .iter()
            .any(|line| line == "FETCH_PIPE_ENDED session=1 stream=1 upstream_lost"),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_joining_fetch_whose_subscribe_never_comes_is_refused_after_the_time_for_first_bytes() {
    assert_eq!(
        HubConfig::default().streams.first_bytes_timeout,
        Duration::from_secs(10)
    );
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let mut alice = Peer::connect(&hub, false).await;
    // Alice makes a SUBSCRIBE, Request ID 0, and does not send it; her
    // Joining FETCH names it.
    let (subscribe, _unsent) = alice.session.subscribe(track(), vec![]).unwrap();
    let range = FetchRange::RelativeJoining {
        joining_request_id: subscribe.value(),
        joining_start: 0,
    };
    let (id, message) = alice.session.fetch(range, vec![]).unwrap();
    let (mut send, recv) = alice.connection.open_bi().await.unwrap();
    send_message(&mut send, &message, &Trace::default(), "request")
        .await
        .unwrap();
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    // Nothing for 9 seconds: the relay holds the FETCH.
    assert!(
        timeout(Duration::from_secs(9), reader.next_event())
            .await
            .is_err()
    );
    assert_eq!(hub.held().await.relay.fetches, 1);
    let lines = logged(&mut lines);
    assert!(
        lines.contains(&"HOLD_REQUEST session=0 stream=1 FETCH".to_string()),
        "{lines:?}"
    );
    // Then INVALID_JOINING_REQUEST_ID (0x32).
    let RequestStreamEvent::Message(response) = request_end(&mut reader).await else {
        panic!("no answer");
    };
    let answer = alice.session.on_request_message(id, response);
    assert!(
        matches!(&answer, Ok(RequestEvent::Rejected(error)) if error.error_code == 0x32),
        "{answer:?}"
    );
    request_close_time().await;
    let held = hub.held().await;
    assert_eq!((held.relay.fetches, held.timers), (0, 0));
}

/// Waits until the hub holds what `there` asks for.
async fn held_when(hub: &HubHandle<MemoryConnection>, there: impl Fn(&Held) -> bool) -> Held {
    timeout(SOON, async {
        loop {
            let held = hub.held().await;
            if there(&held) {
                return held;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("in time")
}

#[tokio::test(start_paused = true)]
async fn a_fetch_for_the_latest_group_is_answered_from_the_relays_cache() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    // Three Objects of Group 0, with nobody subscribed: 30 bytes cached.
    let objects: Vec<_> = (0..3).map(|id| object(id, 10)).collect();
    let data = publisher.send_objects(&objects).await;
    data.finish();
    held_when(&hub, |held| held.relay.cached_bytes == 30).await;

    // {0, 0} to the end of Group 2: up to the Largest Object {0, 2}.
    let mut alice = Peer::connect(&hub, true).await;
    let mut request = alice.start_fetching(vec![]).await;
    let RequestEvent::FetchEstablished { end_location, .. } = alice.answer(&mut request).await
    else {
        panic!("no FETCH_OK");
    };
    assert_eq!(end_location, at(0, 3));
    let mut body = alice.fetch_body(&request).await;
    let (bytes, end) = read_fetch(&mut body).await;
    assert_eq!(end, FetchChunk::End(StreamEnd::Fin));
    // What follows the header, as `crates/moqt` reads it.
    let mut decoder = FetchStreamDecoder::new(1024, GroupOrder::Ascending);
    decoder.push(&[0x05, 0x00]).unwrap();
    decoder.push(&bytes).unwrap();
    let mut fetched = Vec::new();
    while let Some(event) = decoder.next_event().unwrap() {
        fetched.push(event);
    }
    let expected: Vec<FetchEvent> = std::iter::once(FetchEvent::Header(FetchHeader {
        request_id: request.id.value(),
    }))
    .chain((0..3).map(|object_id| {
        FetchEvent::Object(FetchObject {
            group_id: 0,
            subgroup_id: Some(0),
            object_id,
            publisher_priority: 0,
            properties: vec![],
            payload: vec![0x61; 10],
        })
    }))
    .collect();
    assert_eq!(fetched, expected);
    // The publisher was asked nothing.
    assert!(
        timeout(Duration::from_millis(50), publisher.connection.accept_bi())
            .await
            .is_err()
    );
    // Alice ends her direction, and the relay its own after her.
    request.send.finish();
    assert_eq!(
        request_end(&mut request.reader).await,
        RequestStreamEvent::Closed(RequestClose::Fin)
    );
    let held = held_when(&hub, |held| held.relay.fetch_ends == 0).await;
    assert_eq!(held.relay.fetches, 0);
    // The log says what was written, and how the writing ended.
    let lines = logged(&mut lines);
    for line in [
        "WRITE_FETCH_STREAM session=1 request_id=0 pieces=3",
        "FETCH_WRITE_ENDED session=1 stream=1 finished pieces=3",
    ] {
        assert!(lines.contains(&line.to_string()), "{line}: {lines:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_fetch_the_relay_writes_from_its_cache_stops_when_its_subscriber_cancels() {
    let (log, mut lines) = hub::Log::collect();
    let (hub, _task) = hub::spawn(HubConfig {
        log: Some(log),
        ..HubConfig::default()
    });
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    // Far more than a stream's window lets wait.
    let size = 4 * STREAM_WINDOW;
    let objects: Vec<_> = (0..12).map(|id| object(id, size)).collect();
    let data = publisher.send_objects(&objects).await;
    data.finish();
    held_when(&hub, |held| held.relay.cached_bytes == 12 * size).await;

    let mut alice = Peer::connect(&hub, true).await;
    let mut request = alice.start_fetching(vec![]).await;
    assert!(matches!(
        alice.answer(&mut request).await,
        RequestEvent::FetchEstablished { .. }
    ));
    // Alice has the stream and takes nothing of it: the relay waits in a
    // write. Then she cancels her FETCH by resetting the request stream
    // (§5.2), with CANCELLED (0x1).
    let mut body = alice.fetch_body(&request).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    request.send.reset(0x1);
    // The fetch stream is reset, with less than everything on it.
    let (bytes, end) = read_fetch(&mut body).await;
    assert_eq!(end, FetchChunk::End(StreamEnd::Aborted(Some(0x1))));
    assert!(bytes.len() < 12 * size, "{} bytes", bytes.len());
    let held = held_when(&hub, |held| held.relay.fetches == 0).await;
    assert_eq!(held.relay.fetch_ends, 0);
    let lines = logged(&mut lines);
    assert!(
        lines.contains(&"ABORT_FETCH_WRITE session=1 stream=1 code=0x1".to_string()),
        "{lines:?}"
    );
    // And how the writing ended: some of the 12 pieces, cut by the order.
    let ended = lines
        .iter()
        .find_map(|line| line.strip_prefix("FETCH_WRITE_ENDED session=1 stream=1 aborted pieces="))
        .unwrap_or_else(|| panic!("{lines:?}"));
    let (pieces, code) = ended.split_once(' ').unwrap();
    assert!(pieces.parse::<usize>().unwrap() < 12, "{ended}");
    assert_eq!(code, "code=0x1");
}

/// The next thing on a request stream has to be its end; a message would be
/// an answer to something that gets none.
async fn end_within(
    reader: &mut RequestReader<MemoryRecv>,
    time: Duration,
    name: &str,
) -> Option<RequestClose> {
    match timeout(time, reader.next_event()).await {
        Err(_) => None,
        Ok(Ok(RequestStreamEvent::Closed(how))) => Some(how),
        Ok(other) => panic!("{name}: {other:?}"),
    }
}

/// The relay's FIN that comes when the wait for the subscriber is over
/// ([`PEER_END_WAIT`]), and not before: nothing for `quiet`, then the FIN
/// within a second.
async fn the_late_fin(reader: &mut RequestReader<MemoryRecv>, quiet: Duration, name: &str) {
    assert_eq!(end_within(reader, quiet, name).await, None, "{name}");
    let late = Duration::from_secs(1);
    assert_eq!(
        end_within(reader, late, name).await,
        Some(RequestClose::Fin),
        "{name}"
    );
}

/// A FETCH of a new peer that the relay answers itself (`FILL_TIMEOUT` 0)
/// and has served to the end of its fetch stream. `finished`: the peer
/// finished its direction of the request stream right after the FETCH.
/// Returns the peer, the Request ID, and the two halves of the request
/// stream, without the one that was finished.
async fn served_fetch(
    hub: &HubHandle<MemoryConnection>,
    finished: bool,
    name: &str,
) -> (
    Peer,
    RequestId,
    Option<MemorySend>,
    RequestReader<MemoryRecv>,
) {
    let mut alice = Peer::connect(hub, true).await;
    let request = alice.start_fetching(vec![Parameter::FillTimeout(0)]).await;
    let (id, mut reader) = (request.id, request.reader);
    let mut send = Some(request.send);
    if finished {
        send.take().unwrap().finish();
    }
    let answered = reader.next_event().await.unwrap();
    let RequestStreamEvent::Message(fetch_ok) = answered else {
        panic!("{name}: {answered:?}");
    };
    alice.session.on_request_message(id, fetch_ok).unwrap();
    let UniEvent::FetchOpened { stream, header } = alice.next_data().await else {
        panic!("{name}: no fetch stream");
    };
    alice.session.on_fetch_stream(header.request_id).unwrap();
    let mut body = alice.stops.take_fetch_body(stream).expect("the body");
    assert_eq!(
        read_fetch(&mut body).await.1,
        FetchChunk::End(StreamEnd::Fin),
        "{name}"
    );
    (alice, id, send, reader)
}

#[tokio::test(start_paused = true)]
async fn the_request_stream_of_a_fetch_that_was_served_ends_as_the_shared_table_says() {
    // `fixtures/peer-fetch/served-end.tsv`, which the Rust client and
    // `@xmoq/moqt` read as well: a publisher that served a FETCH to the end
    // of its fetch stream, and what becomes of the request stream.
    let table = include_str!("../../../fixtures/peer-fetch/served-end.tsv");
    let ms = Duration::from_millis;
    let mut rows = 0;
    for row in table.lines() {
        let columns: Vec<&str> = row.split('\t').collect();
        let [name, before, after, own, peer, update] = columns[..] else {
            panic!("a row of 6 columns: {row}");
        };
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut publisher = Peer::connect(&hub, true).await;
        let largest = vec![Parameter::LargestObject(at(1, 4))];
        let (publish, message) = publisher
            .session
            .publish(track(), PUBLISHER_ALIAS, largest, vec![])
            .unwrap();
        let _publication = publisher.request(publish, message).await;
        assert!(matches!(before, "-" | "fin"), "{name}: before {before}");
        let (_alice, id, mut send, reader) = served_fetch(&hub, before == "fin", name).await;
        let mut reader = Some(reader);

        // What the relay did with its direction, where it was seen.
        let mut seen = None;
        let mut asked_again = false;
        for step in after.split(',') {
            match step {
                "nothing" => {}
                // 3 seconds: quiet for 2.5 of them.
                "wait" => {
                    the_late_fin(reader.as_mut().unwrap(), ms(2_500), name).await;
                    seen = Some("wait>fin");
                }
                "fin" => send.take().unwrap().finish(),
                // CANCELLED (0x1).
                "reset" => send.take().unwrap().reset(0x1),
                "stop-sending" => reader.take().unwrap().stop(0x1),
                "update" => {
                    let update = ControlMessage::RequestUpdate(xmoq_moqt::message::RequestUpdate {
                        request_id: id.value() + 2,
                        parameters: vec![],
                    });
                    let send = send.as_mut().unwrap();
                    send_message(send, &update, &Trace::default(), "request")
                        .await
                        .unwrap();
                    asked_again = true;
                }
                other => panic!("{name}: after {other}"),
            }
        }
        // An update got no answer: the end is the next thing on the stream
        // wherever it is read here (`end_within`).
        assert_eq!(asked_again, update == "no", "{name}");
        match (seen, reader.as_mut()) {
            (Some(seen), _) => assert_eq!(seen, own, "{name}"),
            // Within a second, or when the 3 seconds are over.
            (None, Some(reader)) => {
                if let Some(end) = end_within(reader, ms(1_000), name).await {
                    assert_eq!((end, own), (RequestClose::Fin, "fin"), "{name}");
                } else {
                    the_late_fin(reader, ms(1_500), name).await;
                    assert_eq!(own, "wait>fin", "{name}");
                }
            }
            // WHY not read: a subscriber that stopped the relay's direction
            // cannot read what the relay did with it. The row is checked by
            // the other column.
            (None, None) => assert_eq!(own, "reset:0x1", "{name}"),
        }
        // The subscriber's direction, where it is still open: stopped at
        // once, or left alone for now.
        let stopped = match send.as_mut() {
            Some(send) => match timeout(ms(500), send.stopped()).await {
                Ok(WriteError::Stopped(Some(code))) => format!("stop:{code:#x}"),
                Ok(other) => panic!("{name}: {other:?}"),
                Err(_) => "-".to_string(),
            },
            None => "-".to_string(),
        };
        assert_eq!(stopped, peer, "{name}");
        // Nothing waits for this FETCH any more.
        held_when(&hub, |held| held.relay.fetch_ends == 0).await;
        rows += 1;
    }
    // A table that lost rows would pass.
    assert_eq!(rows, 10);
}

#[tokio::test(start_paused = true)]
async fn an_object_from_the_cache_keeps_its_subgroup_its_priority_and_its_properties() {
    use xmoq_moqt::kvp::KeyValuePair;

    let (hub, _task) = hub::spawn(HubConfig::default());
    let mut publisher = Peer::connect(&hub, true).await;
    let _publication = publisher.publish().await;
    // Group 0 on two streams: Subgroup 2 with the Publisher Priority 200
    // has the Objects 0 and 3, Subgroup 5 with the priority 7 the Objects 1
    // and 2. Three of them have an Object Property: a number (0x10) or
    // bytes (0x0D).
    let with = |object_id, properties, payload: &[u8]| SubgroupObject {
        object_id,
        properties,
        status: ObjectStatus::Normal,
        payload: payload.to_vec(),
    };
    let number = |value| vec![KeyValuePair::varint(0x10, value).unwrap()];
    let bytes = || vec![KeyValuePair::bytes(0x0d, vec![9, 9]).unwrap()];
    let streams = [
        (
            (2, 200),
            vec![with(0, number(111), &[1, 2, 3]), with(3, bytes(), &[])],
        ),
        (
            (5, 7),
            vec![with(1, vec![], &[4]), with(2, number(222), &[5, 6])],
        ),
    ];
    // Kept open: a stream that is dropped is not one that was finished.
    let mut streams_kept = Vec::new();
    for ((subgroup_id, priority), objects) in streams {
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias: PUBLISHER_ALIAS,
            group_id: 0,
            subgroup_id: SubgroupId::Explicit(subgroup_id),
            publisher_priority: Some(priority),
            properties: true,
            end_of_group: false,
            first_object: true,
        });
        for object in &objects {
            encoder.object(object).unwrap();
        }
        let mut send = publisher.connection.open_uni().await.unwrap();
        send.write_all(&encoder.take_bytes()).await.unwrap();
        streams_kept.push(send);
    }
    // Six bytes of payload, 16 for each of the three properties, and the
    // two bytes of one (`object_size` of `xmoq-relay-core`).
    held_when(&hub, |held| held.relay.cached_bytes == 56).await;

    let mut alice = Peer::connect(&hub, true).await;
    let mut request = alice.start_fetching(vec![]).await;
    assert!(matches!(
        alice.answer(&mut request).await,
        RequestEvent::FetchEstablished { .. }
    ));
    let mut body = alice.fetch_body(&request).await;
    let (written, end) = read_fetch(&mut body).await;
    assert_eq!(end, FetchChunk::End(StreamEnd::Fin));
    let mut decoder = FetchStreamDecoder::new(1024, GroupOrder::Ascending);
    decoder.push(&[0x05, 0x00]).unwrap();
    decoder.push(&written).unwrap();
    let mut fetched = Vec::new();
    while let Some(event) = decoder.next_event().unwrap() {
        fetched.push(event);
    }
    // In the order of the Object IDs, each as it came.
    let object = |object_id, subgroup_id, publisher_priority, properties, payload: &[u8]| {
        FetchEvent::Object(FetchObject {
            group_id: 0,
            subgroup_id: Some(subgroup_id),
            object_id,
            publisher_priority,
            properties,
            payload: payload.to_vec(),
        })
    };
    assert_eq!(
        fetched,
        [
            FetchEvent::Header(FetchHeader { request_id: 0 }),
            object(0, 2, 200, number(111), &[1, 2, 3]),
            object(1, 5, 7, vec![], &[4]),
            object(2, 5, 7, number(222), &[5, 6]),
            object(3, 2, 200, bytes(), &[]),
        ]
    );
}

/// What the delivery timeout of a subscription and the relay's own flags do
/// to a subgroup stream that is not delivered (§8), on a connection that
/// delivers a finished stream when the test says so.
mod delivery_timeouts {
    use super::*;

    /// `DELIVERY_TIMEOUT` (0x2), as the end of a stream.
    const TIMED_OUT: StreamEnd = StreamEnd::Aborted(Some(0x2));

    /// The Groups whose streams ended among what came on the peer's
    /// unidirectional streams and was not taken yet, each with how.
    fn ended(peer: &mut Peer) -> Vec<(u64, StreamEnd)> {
        let (events, _) = peer.data.as_mut().expect("this peer reads");
        std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                UniEvent::SubgroupClosed { header, end, .. } => Some((header.group_id, end)),
                _ => None,
            })
            .collect()
    }

    struct Scene {
        hub: HubHandle<MemoryConnection>,
        publisher: Peer,
        viewer: Peer,
        /// Notified: the finished streams towards the viewer are delivered.
        told: Arc<tokio::sync::Notify>,
        _requests: (OpenRequest, OpenRequest),
        _task: tokio::task::JoinHandle<()>,
    }

    /// A relay with these settings, a publisher of the track, and a viewer
    /// that subscribed with this `SUBGROUP_DELIVERY_TIMEOUT` (none for
    /// `None`) and to which nothing is delivered until the test says so.
    async fn scene(config: HubConfig, asked: Option<u64>) -> Scene {
        let (hub, task) = hub::spawn(config);
        let mut publisher = Peer::connect(&hub, true).await;
        let publication = publisher.publish().await;
        let (mut viewer, told) = Peer::connect_without_delivery(&hub).await;
        let (subscription, _alias) = viewer.subscribe_with(timeout_of(asked)).await;
        Scene {
            hub,
            publisher,
            viewer,
            told,
            _requests: (publication, subscription),
            _task: task,
        }
    }

    fn timeout_of(asked: Option<u64>) -> Vec<Parameter> {
        asked
            .map(Parameter::SubgroupDeliveryTimeout)
            .into_iter()
            .collect()
    }

    fn with_patience(flag: Option<u64>) -> HubConfig {
        HubConfig {
            send: SendLimits {
                delivery_patience: flag.map(Duration::from_millis),
                ..SendLimits::default()
            },
            ..HubConfig::default()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_is_given_up_after_the_smaller_of_the_flag_and_what_the_subscriber_asked() {
        // (`--delivery-patience-ms`, the SUBGROUP_DELIVERY_TIMEOUT of the
        // SUBSCRIBE, when the stream is reset), in milliseconds; never where
        // the last is `None`.
        let cases: [(Option<u64>, Option<u64>, Option<u64>); 7] = [
            (None, None, None),
            (None, Some(300), Some(300)),
            (Some(500), None, Some(500)),
            (Some(500), Some(300), Some(300)),
            (Some(300), Some(500), Some(300)),
            // §8: "a value of 0 means that there is no timeout set".
            (None, Some(0), None),
            (Some(500), Some(0), Some(500)),
        ];
        for (flag, asked, reset_after) in cases {
            let case = format!("flag {flag:?}, asked {asked:?}");
            let mut scene = scene(with_patience(flag), asked).await;
            scene.publisher.send_objects(&[object(0, 3)]).await.finish();
            // The Object is there; its stream is finished and not delivered.
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(ended(&mut scene.viewer), [], "{case}");
            if let Some(reset_after) = reset_after {
                tokio::time::sleep(Duration::from_millis(reset_after - 100)).await;
                assert_eq!(ended(&mut scene.viewer), [], "{case}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                assert_eq!(ended(&mut scene.viewer), [(0, TIMED_OUT)], "{case}");
            } else {
                tokio::time::sleep(Duration::from_secs(60)).await;
                assert_eq!(ended(&mut scene.viewer), [], "{case}");
                // It is still there to be delivered, and its writer goes
                // when it is.
                scene.told.notify_waiters();
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert_eq!(ended(&mut scene.viewer), [(0, StreamEnd::Fin)], "{case}");
            }
            assert_eq!(scene.hub.held().await.out_streams, 0, "{case}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_is_delivered_within_its_time_is_not_reset() {
        // (`--delivery-patience-ms`, the SUBGROUP_DELIVERY_TIMEOUT of the
        // SUBSCRIBE), in milliseconds; the stream is delivered after 100 ms.
        for (flag, asked) in [(None, Some(300)), (Some(300), None), (Some(300), Some(500))] {
            let case = format!("flag {flag:?}, asked {asked:?}");
            let mut scene = scene(with_patience(flag), asked).await;
            scene.publisher.send_objects(&[object(0, 3)]).await.finish();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(ended(&mut scene.viewer), [], "{case}");
            scene.told.notify_waiters();
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(ended(&mut scene.viewer), [(0, StreamEnd::Fin)], "{case}");
            assert_eq!(scene.hub.held().await.out_streams, 0, "{case}");
            // Nothing happens when its time would have been over.
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert_eq!(ended(&mut scene.viewer), [], "{case}");
        }
    }

    /// How a subscription ends for the streams of it that were written
    /// whole and not delivered: they are with their writers still, and are
    /// taken back (§11.4.3: "If a sender closes the stream before delivering
    /// all such objects to the QUIC stream, it MUST reset the stream. This
    /// includes ... Early termination of subscription due to request
    /// cancellation").
    #[derive(Debug, Clone, Copy)]
    enum Leaves {
        /// The subscriber resets its request stream.
        Resets,
        /// The subscriber finishes its request stream.
        Finishes,
        /// The subscriber stops the relay's half of the request stream.
        Stops,
        /// The subscriber closes its session.
        ClosesTheSession,
    }

    /// What the subscriber does; how its two streams end for it; and what
    /// the relay is left with: streams held, sessions, subscriptions.
    type Leaving = (Leaves, Vec<(u64, StreamEnd)>, (usize, usize, usize));

    #[tokio::test(start_paused = true)]
    async fn a_subscription_that_ends_takes_back_its_streams_that_are_not_delivered() {
        /// `CANCELLED` (0x1).
        const CANCELLED: StreamEnd = StreamEnd::Aborted(Some(0x1));
        let cases: [Leaving; 4] = [
            (
                Leaves::Resets,
                vec![(0, CANCELLED), (1, CANCELLED)],
                (0, 2, 0),
            ),
            (
                Leaves::Stops,
                vec![(0, CANCELLED), (1, CANCELLED)],
                (0, 2, 0),
            ),
            // Finishing its half ends nothing: the subscription goes on,
            // and so do its streams.
            (Leaves::Finishes, vec![], (2, 2, 1)),
            // The subscriber closed the connection itself: its streams are
            // lost with it, and the relay keeps none of them.
            (
                Leaves::ClosesTheSession,
                vec![(0, StreamEnd::Lost), (1, StreamEnd::Lost)],
                (0, 1, 0),
            ),
        ];
        for (leaves, ends, left) in cases {
            let Scene {
                hub,
                publisher,
                mut viewer,
                told: _told,
                _requests: (_publication, subscription),
                _task,
            } = scene(HubConfig::default(), None).await;
            publisher.send_group(0, &[object(0, 3)]).await.finish();
            publisher.send_group(1, &[object(0, 3)]).await.finish();
            tokio::time::sleep(Duration::from_millis(10)).await;
            // Written whole, and held: nothing has ended for the subscriber.
            assert_eq!(ended(&mut viewer), [], "{leaves:?}");
            assert_eq!(hub.held().await.out_streams, 2, "{leaves:?}");
            match leaves {
                Leaves::Resets => subscription.send.reset(0x1),
                Leaves::Finishes => subscription.send.finish(),
                Leaves::Stops => subscription.reader.stop(0x1),
                Leaves::ClosesTheSession => viewer.connection.close(0, b""),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut seen = ended(&mut viewer);
            seen.sort_by_key(|(group, _)| *group);
            assert_eq!(seen, ends, "{leaves:?}");
            let held = hub.held().await;
            assert_eq!(
                (held.out_streams, held.sessions, held.relay.subscriptions),
                left,
                "{leaves:?}"
            );
        }
    }

    /// A subscriber that takes streams and acknowledges none has them
    /// counted against it for as long as they are held, and is cut off as
    /// one that is too slow when another would be one too many.
    #[tokio::test(start_paused = true)]
    async fn streams_that_are_not_delivered_count_until_the_subscriber_is_too_far_behind() {
        /// `TOO_FAR_BEHIND` (0x5).
        const TOO_FAR_BEHIND: StreamEnd = StreamEnd::Aborted(Some(0x5));
        let config = HubConfig {
            send: SendLimits {
                streams: 3,
                ..SendLimits::default()
            },
            ..HubConfig::default()
        };
        let Scene {
            hub,
            publisher,
            mut viewer,
            told: _told,
            _requests: (_publication, mut subscription),
            _task,
        } = scene(config, None).await;
        for group in 0..3 {
            publisher.send_group(group, &[object(0, 3)]).await.finish();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        // Three are held, which is as many as the subscriber may have.
        assert_eq!(ended(&mut viewer), []);
        assert_eq!(hub.held().await.out_streams, 3);
        // The fourth is one too many: the subscription ends with
        // TOO_FAR_BEHIND, and the three are taken back. The Stream Count is
        // the one for "not known" (§10.11, 2^62 - 1): the fourth stream was
        // to be opened and never was.
        publisher.send_group(3, &[object(0, 3)]).await.finish();
        assert_eq!(
            publish_done(&mut subscription).await,
            done(0x5, (1 << 62) - 1)
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut seen = ended(&mut viewer);
        seen.sort_by_key(|(group, _)| *group);
        assert_eq!(
            seen,
            [
                (0, TOO_FAR_BEHIND),
                (1, TOO_FAR_BEHIND),
                (2, TOO_FAR_BEHIND)
            ]
        );
        assert_eq!(hub.held().await.out_streams, 0);
    }

    /// §8: the timer starts when the relay knows that the Subgroup is whole
    /// (the FIN of the publisher's stream), and a stream that is not
    /// delivered when it is over is reset: also one the relay could not
    /// write out, because the subscriber takes nothing.
    #[tokio::test(start_paused = true)]
    async fn a_stream_the_subscriber_does_not_take_is_given_up_when_its_time_after_the_fin_is_over()
    {
        // (the SUBGROUP_DELIVERY_TIMEOUT of the SUBSCRIBE in milliseconds,
        // the streams the relay still holds 400 ms after the FIN).
        for (asked, held_after) in [(Some(300), 0), (None, 1)] {
            let case = format!("asked {asked:?}");
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut publisher = Peer::connect(&hub, true).await;
            let _publication = publisher.publish().await;
            // It never takes a unidirectional stream.
            let mut stalled = Peer::connect(&hub, false).await;
            let _subscription = stalled.subscribe_with(timeout_of(asked)).await;
            // More than a stream's window holds, and within what may wait
            // for one subscriber.
            let objects = [object(0, 4 * STREAM_WINDOW), object(1, 4 * STREAM_WINDOW)];
            publisher.send_objects(&objects).await.finish();
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(hub.held().await.out_streams, 1, "{case}");
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(hub.held().await.out_streams, held_after, "{case}");
        }
    }

    /// §10.9: a `REQUEST_UPDATE` that sets `SUBGROUP_DELIVERY_TIMEOUT` is
    /// answered with `REQUEST_OK`, and holds for the streams the relay
    /// learns the end of after it. A stream whose timer runs keeps the
    /// time it was finished with (docs/moqt-implementation-notes.md,
    /// provisional).
    #[tokio::test(start_paused = true)]
    async fn an_update_of_the_delivery_timeout_holds_for_the_streams_that_end_after_it() {
        // (the timeout of the SUBSCRIBE; that of the update, 0 for none;
        // whether the update comes before the FIN of the stream or 100 ms
        // after it; the streams the relay holds 250 ms and 550 ms after
        // the FIN), in milliseconds. The subscriber takes no stream.
        let cases = [
            (None, 200, true, 0, 0),
            (Some(200), 0, true, 1, 1),
            (Some(200), 500, true, 1, 0),
            (Some(500), 200, true, 0, 0),
            // The timer runs with what was asked when the stream ended.
            (None, 200, false, 1, 1),
            (Some(200), 0, false, 0, 0),
            (Some(500), 100, false, 1, 0),
        ];
        for (asked, updated, before_the_fin, held_at_250, held_at_550) in cases {
            let case = format!("asked {asked:?}, then {updated}, before the FIN: {before_the_fin}");
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut publisher = Peer::connect(&hub, true).await;
            let _publication = publisher.publish().await;
            let mut stalled = Peer::connect(&hub, false).await;
            let (mut subscription, _alias) = stalled.subscribe_with(timeout_of(asked)).await;
            let update = ControlMessage::RequestUpdate(xmoq_moqt::message::RequestUpdate {
                request_id: subscription.id.value() + 2,
                parameters: vec![Parameter::SubgroupDeliveryTimeout(updated)],
            });
            let objects = [object(0, 4 * STREAM_WINDOW), object(1, 4 * STREAM_WINDOW)];
            let mut stream = Some(publisher.send_objects(&objects).await);
            if !before_the_fin {
                stream.take().unwrap().finish();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            send_message(
                &mut subscription.send,
                &update,
                &Trace::default(),
                "request",
            )
            .await
            .unwrap();
            let answer = timeout(SOON, subscription.reader.next_event())
                .await
                .expect(&case)
                .unwrap();
            assert!(
                matches!(
                    answer,
                    RequestStreamEvent::Message(ControlMessage::RequestOk(_))
                ),
                "{case}: {answer:?}"
            );
            // Up to 250 ms after the FIN.
            let more = if let Some(stream) = stream {
                stream.finish();
                250
            } else {
                150
            };
            tokio::time::sleep(Duration::from_millis(more)).await;
            assert_eq!(hub.held().await.out_streams, held_at_250, "{case}");
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(hub.held().await.out_streams, held_at_550, "{case}");
        }
    }
}

/// SUBSCRIBEs that are held for a publisher to appear (§10.2.6).
mod rendezvous {
    use super::*;

    /// A SUBSCRIBE for the track `name` of the namespace `(xmoq)`, with a
    /// `RENDEZVOUS_TIMEOUT` where `wait_ms` has one. Not waited for.
    async fn asks(peer: &mut Peer, name: &[u8], wait_ms: Option<u64>) -> OpenRequest {
        asks_in(peer, b"xmoq", name, wait_ms).await
    }

    /// The same for a track of the namespace `(namespace)`.
    async fn asks_in(
        peer: &mut Peer,
        namespace: &[u8],
        name: &[u8],
        wait_ms: Option<u64>,
    ) -> OpenRequest {
        let namespace = TrackNamespace::new(vec![namespace.to_vec()]).unwrap();
        let full = FullTrackName::new(namespace, name.to_vec()).unwrap();
        let parameters = wait_ms
            .map(Parameter::RendezvousTimeout)
            .into_iter()
            .collect();
        let (id, message) = peer.session.subscribe(full, parameters).unwrap();
        let (mut send, recv) = peer.connection.open_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .unwrap();
        let reader = RequestReader::new(recv, &Trace::default(), "request");
        OpenRequest { id, send, reader }
    }

    /// What the relay answered within `wait`: "ok", the code of its
    /// `REQUEST_ERROR`, or "nothing".
    async fn answered(peer: &mut Peer, request: &mut OpenRequest, wait: Duration) -> String {
        let Ok(event) = timeout(wait, request.reader.next_event()).await else {
            return "nothing".to_string();
        };
        let Ok(RequestStreamEvent::Message(response)) = event else {
            panic!("no message: {event:?}");
        };
        match peer
            .session
            .on_request_message(request.id, response)
            .unwrap()
        {
            RequestEvent::SubscribeEstablished { .. } => "ok".to_string(),
            RequestEvent::Rejected(error) => format!("{:#x}", error.error_code),
            other => panic!("not an answer to a SUBSCRIBE: {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscribe_is_held_for_as_long_as_it_asks_and_the_relay_allows() {
        // (RENDEZVOUS_TIMEOUT in ms; how long the test waits first, in which
        //  nothing comes; what comes in the 200 ms after that)
        let cases = [
            // DOES_NOT_EXIST (0x10) at once without the parameter, and with 0.
            (None, 0, "0x10"),
            (Some(0), 0, "0x10"),
            // TIMEOUT (0x2) when the time is over.
            (Some(5_000), 4_900, "0x2"),
            // No longer than the relay holds one: 30 seconds.
            (Some(3_600_000), 29_900, "0x2"),
        ];
        for (asked, quiet_ms, then) in cases {
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut viewer = Peer::connect(&hub, true).await;
            let mut request = asks(&mut viewer, b"a", asked).await;
            if quiet_ms > 0 {
                assert_eq!(
                    answered(&mut viewer, &mut request, Duration::from_millis(quiet_ms)).await,
                    "nothing",
                    "{asked:?}"
                );
            }
            assert_eq!(
                answered(&mut viewer, &mut request, Duration::from_millis(200)).await,
                then,
                "{asked:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_subscribe_is_accepted_when_a_publisher_sends_publish_for_the_track() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut viewer = Peer::connect(&hub, true).await;
        let mut request = asks(&mut viewer, b"a", Some(20_000)).await;
        assert_eq!(
            answered(&mut viewer, &mut request, Duration::from_secs(10)).await,
            "nothing"
        );
        // The time it is held for is the one timer that runs.
        assert_eq!(hub.held().await.timers, 1);
        let mut publisher = Peer::connect(&hub, true).await;
        let _published = publisher.publish().await;
        assert_eq!(answered(&mut viewer, &mut request, SOON).await, "ok");
        // It is dropped with the answer, and does not run on.
        assert_eq!(hub.held().await.timers, 0);
        // Its time runs out later, and ends nothing.
        tokio::time::sleep(Duration::from_secs(15)).await;
        assert_eq!(viewer.connection.close_info().map(|close| close.code), None);
        assert_eq!(
            answered(&mut viewer, &mut request, Duration::from_millis(100)).await,
            "nothing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_subscribe_is_asked_of_a_publisher_that_announces_its_namespace() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut viewer = Peer::connect(&hub, true).await;
        let mut request = asks(&mut viewer, b"a", Some(20_000)).await;
        assert_eq!(
            answered(&mut viewer, &mut request, Duration::from_secs(10)).await,
            "nothing"
        );
        let mut publisher = Peer::connect(&hub, true).await;
        let _announced = publisher.publish_namespace().await;
        let _serving = publisher.accept_relay_subscribe().await;
        assert_eq!(answered(&mut viewer, &mut request, SOON).await, "ok");
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_subscribe_waits_on_when_a_session_that_announced_would_not_serve_the_track() {
        // (RENDEZVOUS_TIMEOUT of the viewer; what it has 200 ms after the
        //  session that announced gave the relay's SUBSCRIBE up)
        let cases = [
            (Some(20_000), "nothing"),
            // Not held: refused with the others, DOES_NOT_EXIST (0x10).
            (None, "0x10"),
        ];
        for (asked, after_the_refusal) in cases {
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut publisher = Peer::connect(&hub, true).await;
            let _announced = publisher.publish_namespace().await;
            let mut viewer = Peer::connect(&hub, true).await;
            let mut request = asks(&mut viewer, b"a", asked).await;
            // The relay asks the session that announced, which resets that
            // request: CANCELLED (0x1).
            let (_id, send, _reader) = publisher.relay_subscribe().await;
            send.reset(0x1);
            assert_eq!(
                answered(&mut viewer, &mut request, Duration::from_millis(200)).await,
                after_the_refusal,
                "{asked:?}"
            );
            if asked.is_none() {
                continue;
            }
            // A publisher of the track comes after all. WHY another
            // session: this peer's own session still has the relay's
            // SUBSCRIBE, which the test reset beneath it.
            let mut other = Peer::connect(&hub, true).await;
            let _published = other.publish().await;
            assert_eq!(
                answered(&mut viewer, &mut request, Duration::from_millis(200)).await,
                "ok"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_publisher_that_appears_and_does_not_answer_has_the_time_the_relay_gives_it() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut viewer = Peer::connect(&hub, true).await;
        let mut request = asks(&mut viewer, b"a", Some(6_000)).await;
        tokio::time::sleep(Duration::from_secs(4)).await;
        // It announces at 4 s and never answers the relay's SUBSCRIBE.
        let mut publisher = Peer::connect(&hub, true).await;
        let _announced = publisher.publish_namespace().await;
        // The viewer's 6 s are over at 6 s: nothing, a publisher has come.
        assert_eq!(
            answered(&mut viewer, &mut request, Duration::from_millis(4_900)).await,
            "nothing"
        );
        // The publisher's 5 s are over at 9 s: TIMEOUT (0x2).
        assert_eq!(
            answered(&mut viewer, &mut request, Duration::from_millis(200)).await,
            "0x2"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn who_comes_to_a_held_track_waits_by_its_own_time_or_not_at_all() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut first = Peer::connect(&hub, true).await;
        let mut held = asks(&mut first, b"a", Some(10_000)).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut second = Peer::connect(&hub, true).await;
        let mut at_once = asks(&mut second, b"a", None).await;
        assert_eq!(answered(&mut second, &mut at_once, SOON).await, "0x10");
        let mut third = Peer::connect(&hub, true).await;
        let mut shorter = asks(&mut third, b"a", Some(2_000)).await;
        // The third's time is over at 3 s, the first's at 10 s. An answer
        // ends the wait for it, so the clock stands at 3 s after the third.
        assert_eq!(
            answered(&mut third, &mut shorter, Duration::from_millis(1_900)).await,
            "nothing"
        );
        assert_eq!(
            answered(&mut third, &mut shorter, Duration::from_millis(200)).await,
            "0x2"
        );
        assert_eq!(
            answered(&mut first, &mut held, Duration::from_millis(6_900)).await,
            "nothing"
        );
        assert_eq!(
            answered(&mut first, &mut held, Duration::from_millis(200)).await,
            "0x2"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_has_sixteen_subscribes_held_and_the_next_is_not() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut viewer = Peer::connect(&hub, true).await;
        let mut held = Vec::new();
        for number in 0..16u8 {
            held.push(asks(&mut viewer, &[b'a', number], Some(20_000)).await);
        }
        let mut one_more = asks(&mut viewer, b"z", Some(20_000)).await;
        assert_eq!(answered(&mut viewer, &mut one_more, SOON).await, "0x10");
        // The request that was refused is kept until its stream has gone.
        one_more.send.finish();
        for request in &mut held {
            assert_eq!(
                answered(&mut viewer, request, Duration::from_millis(1)).await,
                "nothing"
            );
        }
        // A held SUBSCRIBE that is given up frees its place.
        let given_up = held.pop().unwrap();
        // CANCELLED (0x1).
        given_up.send.reset(0x1);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut again = asks(&mut viewer, b"z", Some(20_000)).await;
        assert_eq!(
            answered(&mut viewer, &mut again, Duration::from_millis(100)).await,
            "nothing"
        );
        let held = hub.held().await;
        assert_eq!(held.relay.requests, 16);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_whose_held_subscribe_timed_out_holds_nothing_and_goes_thirty_seconds_later()
    {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut viewer = Peer::connect(&hub, true).await;
        let mut request = asks(&mut viewer, b"a", Some(5_000)).await;
        // Refused when its 5 seconds are over; from then on the session
        // holds nothing.
        assert_eq!(
            answered(&mut viewer, &mut request, Duration::from_millis(5_100)).await,
            "0x2"
        );
        request.send.finish();
        tokio::time::sleep(Duration::from_secs(29)).await;
        assert_eq!(viewer.connection.close_info().map(|close| close.code), None);
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        assert_eq!(
            viewer.connection.close_info().map(|close| close.code),
            Some(0x0)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_subscribe_that_is_given_up_or_whose_session_goes_leaves_nothing() {
        // (how the viewer goes; the pending tracks and the timers the relay
        //  has 100 ms later)
        for by_closing_the_session in [false, true] {
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut viewer = Peer::connect(&hub, true).await;
            let request = asks(&mut viewer, b"a", Some(20_000)).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let held = hub.held().await;
            assert_eq!((held.relay.pending_tracks, held.timers), (1, 1));
            if by_closing_the_session {
                viewer.connection.close(0, b"");
            } else {
                // CANCELLED (0x1).
                request.send.reset(0x1);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            let held = hub.held().await;
            assert_eq!(
                (held.relay.pending_tracks, held.timers),
                (0, 0),
                "{by_closing_the_session}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_subscribe_of_a_session_for_a_track_it_is_held_for_is_a_duplicate() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut viewer = Peer::connect(&hub, true).await;
        let mut first = asks(&mut viewer, b"a", Some(20_000)).await;
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        let full = FullTrackName::new(namespace, b"a".to_vec()).unwrap();
        // The peer's own session refuses to send it, so it is put on the
        // wire by hand: a SUBSCRIBE with the next Request ID.
        let message = ControlMessage::Subscribe(Subscribe {
            request_id: first.id.value() + 2,
            full_track_name: full,
            parameters: vec![Parameter::RendezvousTimeout(20_000)],
        });
        let (mut send, recv) = viewer.connection.open_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .unwrap();
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        // DUPLICATE_SUBSCRIPTION (0x19).
        match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(ControlMessage::RequestError(error)))) => {
                assert_eq!(error.error_code, 0x19);
            }
            other => panic!("no REQUEST_ERROR: {other:?}"),
        }
        // The first is held as before.
        assert_eq!(
            answered(&mut viewer, &mut first, Duration::from_millis(100)).await,
            "nothing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_relay_holds_subscribes_for_a_thousand_and_twenty_four_tracks_whichever_way_they_come()
     {
        let (hub, _task) = hub::spawn(HubConfig::default());
        // 64 sessions with 16 held SUBSCRIBEs each: 1,024 tracks.
        let mut viewers = Vec::new();
        let mut kept = Vec::new();
        for session in 0..64u8 {
            let mut viewer = Peer::connect(&hub, true).await;
            for number in 0..16u8 {
                let name = [b't', session, number];
                kept.push(asks_in(&mut viewer, b"held", &name, Some(20_000)).await);
            }
            viewers.push(viewer);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(hub.held().await.relay.pending_tracks, 1_024);

        // One more, for a track nobody is asked for: not held.
        let mut late = Peer::connect(&hub, true).await;
        let mut one_more = asks_in(&mut late, b"held", b"one more", Some(20_000)).await;
        assert_eq!(answered(&mut late, &mut one_more, SOON).await, "0x10");
        one_more.send.finish();

        // One more, for a track a session that announced is asked for: it
        // waits for that answer like any other, and is not held when the
        // session will not serve the track.
        let mut announcer = Peer::connect(&hub, true).await;
        let _announced = announcer.publish_namespace().await;
        let mut asked = asks(&mut late, b"a", Some(20_000)).await;
        let (_id, send, _reader) = announcer.relay_subscribe().await;
        // CANCELLED (0x1).
        send.reset(0x1);
        assert_eq!(answered(&mut late, &mut asked, SOON).await, "0x10");
        assert_eq!(hub.held().await.relay.pending_tracks, 1_024);

        // A place that is given back is one that can be taken.
        // CANCELLED (0x1).
        kept.pop().unwrap().send.reset(0x1);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut again = asks_in(&mut late, b"held", b"again", Some(20_000)).await;
        assert_eq!(
            answered(&mut late, &mut again, Duration::from_millis(100)).await,
            "nothing"
        );
    }

    /// What happens a second after a track began to drain, for the test
    /// below.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Then {
        Nothing,
        /// The publisher that said `PUBLISH_DONE` sends PUBLISH anew.
        ThePublisherIsBack,
        /// Another session sends PUBLISH for the track.
        AnotherPublisherComes,
        /// Another session sends PUBLISH three seconds later still, when
        /// the track is over.
        AnotherPublisherComesWhenItIsOver,
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscribe_that_comes_while_a_track_drains_is_held_as_one_for_a_track_without_a_publisher()
     {
        // The publisher says PUBLISH_DONE at 0 s and counts a stream that
        // never comes, so the track drains until 2 s. The viewer asks at
        // 0.5 s.
        // (the viewer's RENDEZVOUS_TIMEOUT; what happens at 1 s; what the
        //  viewer has by 1.8 s; what it has by 11 s, where it had nothing)
        let cases = [
            // Not held: DOES_NOT_EXIST (0x10) at once.
            (None, Then::Nothing, "0x10", None),
            // Its time is over while the track drains: TIMEOUT (0x2).
            (Some(1_000), Then::Nothing, "0x2", None),
            // The track is over at 2 s and nobody comes: TIMEOUT at 10.5 s,
            // counted from the SUBSCRIBE.
            (Some(10_000), Then::Nothing, "nothing", Some("0x2")),
            (Some(10_000), Then::ThePublisherIsBack, "ok", None),
            (Some(10_000), Then::AnotherPublisherComes, "ok", None),
            (
                Some(10_000),
                Then::AnotherPublisherComesWhenItIsOver,
                "nothing",
                Some("ok"),
            ),
        ];
        for (asked, then, by_two_seconds, by_eleven) in cases {
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut publisher = Peer::connect(&hub, true).await;
            let mut publication = publisher.publish().await;
            // Somebody watches, so that the track has a subscriber.
            let mut watcher = Peer::connect(&hub, true).await;
            let (_watching, _) = watcher.subscribe().await;
            let ended = publisher
                .session
                .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
                .unwrap();
            send_message(&mut publication.send, &ended, &Trace::default(), "publish")
                .await
                .unwrap();
            publication.send.finish();
            tokio::time::sleep(Duration::from_millis(500)).await;

            let mut viewer = Peer::connect(&hub, true).await;
            let mut request = asks(&mut viewer, b"a", asked).await;
            let first = answered(&mut viewer, &mut request, Duration::from_millis(500)).await;
            let mut other = Peer::connect(&hub, true).await;
            match then {
                Then::ThePublisherIsBack => {
                    publisher.publish().await;
                }
                Then::AnotherPublisherComes => {
                    other.publish().await;
                }
                Then::Nothing | Then::AnotherPublisherComesWhenItIsOver => {}
            }
            let by_then = if first == "nothing" {
                answered(&mut viewer, &mut request, Duration::from_millis(800)).await
            } else {
                first
            };
            assert_eq!(by_then, by_two_seconds, "{asked:?} {then:?}");
            let Some(later) = by_eleven else {
                continue;
            };
            if then == Then::AnotherPublisherComesWhenItIsOver {
                tokio::time::sleep(Duration::from_secs(3)).await;
                other.publish().await;
            }
            assert_eq!(
                answered(&mut viewer, &mut request, Duration::from_secs(9)).await,
                later,
                "{asked:?} {then:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscribe_held_while_a_track_drains_leaves_nothing_when_it_is_gone_and_the_track_is_over()
     {
        // (how the viewer goes; the tracks and the pending tracks the relay
        //  has when the track is over)
        for by_closing_the_session in [false, true] {
            let (hub, _task) = hub::spawn(HubConfig::default());
            let mut publisher = Peer::connect(&hub, true).await;
            let mut publication = publisher.publish().await;
            let mut watcher = Peer::connect(&hub, true).await;
            let (_watching, _) = watcher.subscribe().await;
            let ended = publisher
                .session
                .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
                .unwrap();
            send_message(&mut publication.send, &ended, &Trace::default(), "publish")
                .await
                .unwrap();
            publication.send.finish();
            let mut viewer = Peer::connect(&hub, true).await;
            let request = asks(&mut viewer, b"a", Some(10_000)).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            if by_closing_the_session {
                viewer.connection.close(0, b"");
            } else {
                // CANCELLED (0x1).
                request.send.reset(0x1);
            }
            // The track is over at 2 s, and nothing waits under its name.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let held = hub.held().await;
            assert_eq!(
                (held.relay.tracks, held.relay.pending_tracks),
                (0, 0),
                "{by_closing_the_session}"
            );
        }
    }

    /// A SUBSCRIBE for the track `a` with a `RENDEZVOUS_TIMEOUT` of 10 s,
    /// put on the wire without the peer's own session, which would refuse a
    /// second one for a track it holds a subscription to. What comes back
    /// within 100 ms: "ok", the code of a `REQUEST_ERROR`, or "nothing".
    async fn asks_by_hand(peer: &Peer, request_id: u64) -> (String, RequestReader<MemoryRecv>) {
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        let message = ControlMessage::Subscribe(Subscribe {
            request_id,
            full_track_name: FullTrackName::new(namespace, b"a".to_vec()).unwrap(),
            parameters: vec![Parameter::RendezvousTimeout(10_000)],
        });
        let (mut send, recv) = peer.connection.open_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .unwrap();
        // WHY forgotten: dropping it would reset the stream, and with it
        // the SUBSCRIBE.
        std::mem::forget(send);
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        let answer = match timeout(Duration::from_millis(100), reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(ControlMessage::SubscribeOk(_)))) => "ok".to_owned(),
            Ok(Ok(RequestStreamEvent::Message(ControlMessage::RequestError(error)))) => {
                format!("{:#x}", error.error_code)
            }
            Err(_) => "nothing".to_owned(),
            other => panic!("neither an answer nor silence: {other:?}"),
        };
        (answer, reader)
    }

    #[tokio::test(start_paused = true)]
    async fn a_viewer_that_asks_anew_is_a_duplicate_until_it_was_sent_publish_done_and_never_after()
    {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut publisher = Peer::connect(&hub, true).await;
        let mut publication = publisher.publish().await;
        let mut viewer = Peer::connect(&hub, true).await;
        let (mut request, _) = viewer.subscribe().await;
        // The track drains for two seconds.
        let ended = publisher
            .session
            .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
            .unwrap();
        send_message(&mut publication.send, &ended, &Trace::default(), "publish")
            .await
            .unwrap();
        publication.send.finish();
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Before its PUBLISH_DONE: the subscription it has goes on, and a
        // second is DUPLICATE_SUBSCRIPTION (0x19).
        let next = request.id.value() + 2;
        let (early, _reader) = asks_by_hand(&viewer, next).await;
        assert_eq!(early, "0x19");

        // With its PUBLISH_DONE read: held, whatever comes first to the
        // relay, since the relay ended the subscription before it sent it.
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(publish_done(&mut request).await, done(0x2, 0));
        let (after, mut reader) = asks_by_hand(&viewer, next + 2).await;
        assert_eq!(after, "nothing");
        publisher.publish().await;
        match timeout(SOON, reader.next_event()).await {
            Ok(Ok(RequestStreamEvent::Message(ControlMessage::SubscribeOk(_)))) => {}
            other => panic!("no SUBSCRIBE_OK: {other:?}"),
        }
    }

    /// A track `a` whose publisher said `PUBLISH_DONE` and counted a stream
    /// that never comes: it drains for two seconds from here.
    async fn a_track_that_drains(hub: &HubHandle<MemoryConnection>) -> Peer {
        let mut publisher = Peer::connect(hub, true).await;
        let mut publication = publisher.publish().await;
        let ended = publisher
            .session
            .publish_done(publication.id, 0x2, 1, ReasonPhrase::default())
            .unwrap();
        send_message(&mut publication.send, &ended, &Trace::default(), "publish")
            .await
            .unwrap();
        publication.send.finish();
        tokio::time::sleep(Duration::from_millis(500)).await;
        publisher
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscribe_held_while_a_track_drains_gets_the_objects_of_the_publisher_that_is_back()
    {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut publisher = a_track_that_drains(&hub).await;
        let mut viewer = Peer::connect(&hub, true).await;
        let mut request = asks(&mut viewer, b"a", Some(10_000)).await;
        // Kept: a PUBLISH whose request stream is let go of is cancelled.
        let _published = publisher.publish().await;
        let RequestEvent::SubscribeEstablished { track_alias, .. } =
            viewer.answer(&mut request).await
        else {
            panic!("the held SUBSCRIBE was not accepted");
        };
        let data = publisher.send_objects(&[object(0, 3), object(1, 3)]).await;
        data.finish();
        assert_eq!(viewer.receive_stream(track_alias).await, [0, 1]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscribe_held_while_a_track_drains_is_asked_of_who_announced_when_the_track_is_over()
     {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut announcer = Peer::connect(&hub, true).await;
        let _announced = announcer.publish_namespace().await;
        let _publisher = a_track_that_drains(&hub).await;
        let mut viewer = Peer::connect(&hub, true).await;
        let mut request = asks(&mut viewer, b"a", Some(10_000)).await;
        // Nobody is asked while the track drains.
        assert!(
            timeout(
                Duration::from_millis(1400),
                announcer.connection.accept_bi()
            )
            .await
            .is_err()
        );
        // It is over at 2 s, and the session that announced is asked.
        let _upstream = announcer.accept_relay_subscribe().await;
        assert_eq!(answered(&mut viewer, &mut request, SOON).await, "ok");
    }

    /// A `PUBLISH_NAMESPACE` for `(xmoq, <name>)`: "ok", or the code of the
    /// `REQUEST_ERROR`. The request is kept open with the peer.
    async fn announces(peer: &mut Peer, name: &[u8], kept: &mut Vec<OpenRequest>) -> String {
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec(), name.to_vec()]).unwrap();
        let (id, message) = peer.session.publish_namespace(namespace, vec![]).unwrap();
        let (request, event) = peer.request(id, message).await;
        kept.push(request);
        match event {
            RequestEvent::PublishNamespaceEstablished => "ok".to_string(),
            RequestEvent::Rejected(error) => format!("{:#x}", error.error_code),
            other => panic!("not an answer to a PUBLISH_NAMESPACE: {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_announces_five_namespaces_at_once_and_one_more_each_second() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut peer = Peer::connect(&hub, true).await;
        let mut kept = Vec::new();
        // (how long the test waits first, in ms; what the announcements
        //  that follow get, one after the other)
        let steps: [(u64, &[&str]); 5] = [
            // Five at once, and the sixth is EXCESSIVE_LOAD (0x9).
            (0, &["ok", "ok", "ok", "ok", "ok", "0x9", "0x9"]),
            // Not yet a second since the first.
            (900, &["0x9"]),
            // One came back, and no more than one.
            (200, &["ok", "0x9"]),
            // Three more in three seconds: the ones refused took nothing.
            (3_000, &["ok", "ok", "ok", "0x9"]),
            // No more than five however long it waits.
            (60_000, &["ok", "ok", "ok", "ok", "ok", "0x9"]),
        ];
        let mut name = 0u8;
        for (wait_ms, answers) in steps {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            for answer in answers {
                name += 1;
                assert_eq!(
                    announces(&mut peer, &[name], &mut kept).await,
                    *answer,
                    "after {wait_ms} ms, announcement {name}"
                );
            }
        }
        // The session goes on: another of its requests is answered.
        let mut request = asks(&mut peer, b"nobody", None).await;
        assert_eq!(answered(&mut peer, &mut request, SOON).await, "0x10");
        // The others have what is theirs.
        let mut other = Peer::connect(&hub, true).await;
        assert_eq!(announces(&mut other, b"o", &mut kept).await, "ok");
        // The timer that gives them back stops when all are back.
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(hub.held().await.timers, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_announcement_that_is_withdrawn_and_sent_again_counts_each_time() {
        let (hub, _task) = hub::spawn(HubConfig::default());
        let mut peer = Peer::connect(&hub, true).await;
        // The same name each time, withdrawn (CANCELLED, 0x1) before the
        // next: (the answers)
        let mut answers = Vec::new();
        for _ in 0..7 {
            let mut kept = Vec::new();
            answers.push(announces(&mut peer, b"same", &mut kept).await);
            for request in kept {
                peer.session.cancel(request.id);
                request.send.reset(0x1);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(answers, ["ok", "ok", "ok", "ok", "ok", "0x9", "0x9"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscribe_read_after_its_session_was_closed_is_not_said_to_be_held() {
        let (log, mut lines) = hub::Log::collect();
        let (hub, _task) = hub::spawn(HubConfig {
            log: Some(log),
            ..HubConfig::default()
        });
        let viewer = Peer::connect(&hub, true).await;
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        let subscribe = |request_id| {
            ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name: FullTrackName::new(namespace.clone(), b"a".to_vec()).unwrap(),
                parameters: vec![Parameter::RendezvousTimeout(20_000)],
            })
        };
        // A Request ID of the wrong parity closes the session (§10.1), and
        // a SUBSCRIBE that is in order is on its way behind it.
        let (mut wrong, _wrong_recv) = viewer.connection.open_bi().await.unwrap();
        let (mut right, _right_recv) = viewer.connection.open_bi().await.unwrap();
        send_message(&mut wrong, &subscribe(1), &Trace::default(), "request")
            .await
            .unwrap();
        // At once, so that it is read after the session was closed.
        let _ = send_message(&mut right, &subscribe(0), &Trace::default(), "request").await;
        tokio::time::sleep(SOON).await;
        let said: Vec<String> = logged(&mut lines)
            .into_iter()
            .filter(|line| line.starts_with("HOLD_REQUEST ") || line.starts_with("CLOSE_SESSION "))
            .map(|line| line.split(' ').next().unwrap_or_default().to_string())
            .collect();
        assert_eq!(said, ["CLOSE_SESSION"]);
        let held = hub.held().await;
        assert_eq!((held.relay.pending_tracks, held.timers), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_that_has_as_many_timers_as_it_may_has_no_more_announcements_for_it() {
        // Two timers for a session, where the relay has 1,024.
        let (hub, _task) = hub::spawn(HubConfig {
            max_timers_per_session: 2,
            ..HubConfig::default()
        });
        let mut peer = Peer::connect(&hub, true).await;
        // Two SUBSCRIBEs held for 20 seconds: the two timers.
        let _first = asks(&mut peer, b"h0", Some(20_000)).await;
        let _second = asks(&mut peer, b"h1", Some(20_000)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(hub.held().await.timers, 2);
        // Five announcements at once, and the sixth and seventh are
        // EXCESSIVE_LOAD (0x9), as for a session that has no timer running.
        let mut kept = Vec::new();
        let mut answers = Vec::new();
        for name in 0..7u8 {
            answers.push(announces(&mut peer, &[name], &mut kept).await);
        }
        assert_eq!(answers, ["ok", "ok", "ok", "ok", "ok", "0x9", "0x9"]);
    }
}

/// §11.4.2: the peer's unidirectional streams, filled and not read, leave the
/// request streams room in the connection's receive window, with the
/// streams the relay reads at once (`quic_limits`).
#[test]
fn the_relay_leaves_its_request_streams_room_in_the_connection_window() {
    let limits = crate::quic_limits(
        &HubConfig::default(),
        Duration::from_secs(6),
        xmoq_moqt_io::wt::Congestion::default(),
    );
    assert!(limits.leaves_room_for_control_streams(), "{limits:?}");
    // 67 streams of 128 KiB, and 1 MiB: 9.375 MiB.
    assert_eq!(
        (limits.stream_receive_window, limits.receive_window),
        (131_072, 9_830_400)
    );
}

/// The peer may open as many bidirectional streams as the relay reads
/// request streams at once, which is above the requests a session may have
/// open, so that one more is read and refused with `EXCESSIVE_LOAD`
/// (`request_streams_for`): with the default (256), and with
/// `--peer-requests-per-session` 10.
#[test]
fn the_peer_may_open_a_request_stream_more_than_it_may_have_requests_open() {
    let cases = [(None, 264), (Some(10), 18)];
    for (peer_requests, bidirectional) in cases {
        let mut config = HubConfig::default();
        if let Some(peer_requests) = peer_requests {
            config.limits_of_a_session.peer_requests = peer_requests;
        }
        let limits = crate::quic_limits(
            &config,
            Duration::from_secs(6),
            xmoq_moqt_io::wt::Congestion::default(),
        );
        assert_eq!(limits.max_bi_streams, bidirectional, "{peer_requests:?}");
    }
}

/// A relay with another relay above it (`xmoq-relay-core`'s `parent.rs`),
/// both on in-memory connections.
mod parent {
    use super::*;

    /// An edge whose parent is `origin`: each session the edge asks for is
    /// a new connection to it, of which the edge is the client.
    fn edge_of(origin: &HubHandle<MemoryConnection>) -> HubHandle<MemoryConnection> {
        edge_of_with(origin, HubConfig::default())
    }

    fn edge_of_with(
        origin: &HubHandle<MemoryConnection>,
        config: HubConfig,
    ) -> HubHandle<MemoryConnection> {
        let origin = origin.clone();
        let connect: hub::Connect<MemoryConnection> = Arc::new(move |_| {
            let origin = origin.clone();
            Box::pin(async move {
                let (local, remote) = pair();
                origin.add_session(remote, Arrival::First).await;
                Ok(local)
            })
        });
        let parent = hub::Parent {
            namespace: TrackNamespace::new(vec![]).unwrap(),
            connect,
        };
        hub::spawn_with_parent(config, Some(parent)).0
    }

    /// An edge whose parent is `first`, and `second` where a GOAWAY of
    /// `first` names the URI `second`.
    fn edge_of_two(
        first: &HubHandle<MemoryConnection>,
        second: &HubHandle<MemoryConnection>,
    ) -> HubHandle<MemoryConnection> {
        let (first, second) = (first.clone(), second.clone());
        let connect: hub::Connect<MemoryConnection> = Arc::new(move |uri: Option<String>| {
            let origin = match uri.as_deref() {
                Some("second") => second.clone(),
                _ => first.clone(),
            };
            Box::pin(async move {
                let (local, remote) = pair();
                origin.add_session(remote, Arrival::First).await;
                Ok(local)
            })
        });
        let parent = hub::Parent {
            namespace: TrackNamespace::new(vec![]).unwrap(),
            connect,
        };
        hub::spawn_with_parent(HubConfig::default(), Some(parent)).0
    }

    /// §3.6 at an edge: the origin's GOAWAY has the edge take the track from
    /// the relay it names as well, and the subscriber goes on with it when
    /// the first origin is gone, with no `PUBLISH_DONE`.
    #[tokio::test(start_paused = true)]
    async fn a_subscriber_of_the_edge_goes_on_when_the_origin_sends_it_elsewhere() {
        let (first, _first_task) = hub::spawn(HubConfig::default());
        let (second, _second_task) = hub::spawn(HubConfig::default());
        let edge = edge_of_two(&first, &second);
        let mut publisher = Peer::connect(&first, true).await;
        let _namespace = publisher.publish_namespace().await;
        let mut moved = Peer::connect(&second, true).await;
        let _moved_namespace = moved.publish_namespace().await;
        let mut alice = Peer::connect(&edge, true).await;
        let mut request = alice.start_subscribing().await;
        let _upstream = publisher.accept_relay_subscribe().await;
        let RequestEvent::SubscribeEstablished { track_alias, .. } =
            alice.answer(&mut request).await
        else {
            panic!("Alice was not accepted");
        };

        let going_away = {
            let first = first.clone();
            tokio::spawn(async move { first.go_away(b"second".to_vec(), 1_000).await })
        };
        let upstream = moved.accept_relay_subscribe().await;
        moved
            .send_group_as(upstream.track_alias, 0, &[object(0, 3)])
            .await
            .finish();
        assert_eq!(alice.receive_stream(track_alias).await, [0]);

        first.stop_with(0x10, "the GOAWAY's time is over").await;
        going_away.abort();
        tokio::time::sleep(Duration::from_secs(3)).await;
        moved
            .send_group_as(upstream.track_alias, 1, &[object(0, 3)])
            .await
            .finish();
        assert_eq!(alice.receive_stream(track_alias).await, [0]);
        assert!(timeout(SOON, request.reader.next_event()).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn the_session_with_the_parent_does_not_take_a_place_of_a_subscriber() {
        let (origin, _task) = hub::spawn(HubConfig::default());
        let edge = edge_of_with(
            &origin,
            HubConfig {
                max_sessions: 2,
                ..HubConfig::default()
            },
        );
        let mut publisher = Peer::connect(&origin, true).await;
        let _namespace = publisher.publish_namespace().await;
        let mut alice = Peer::connect(&edge, true).await;
        let mut request = alice.start_subscribing().await;
        let _upstream = publisher.accept_relay_subscribe().await;
        let RequestEvent::SubscribeEstablished { .. } = alice.answer(&mut request).await else {
            panic!("Alice was not accepted");
        };
        // Alice and the parent are two sessions of the edge; Bob is the
        // second subscriber.
        let bob = Peer::connect(&edge, true).await;
        assert!(timeout(SOON, bob.connection.closed()).await.is_err());
        let carol = Peer::connect(&edge, false).await;
        timeout(SOON, carol.connection.closed())
            .await
            .expect("the third subscriber is refused");
    }

    #[tokio::test(start_paused = true)]
    async fn objects_of_a_publisher_at_the_origin_reach_the_subscribers_of_the_edge() {
        let (origin, _task) = hub::spawn(HubConfig::default());
        let edge = edge_of(&origin);
        let mut publisher = Peer::connect(&origin, true).await;
        let _namespace = publisher.publish_namespace().await;
        let mut alice = Peer::connect(&edge, true).await;
        let mut alice_request = alice.start_subscribing().await;
        let mut bob = Peer::connect(&edge, true).await;
        let mut bob_request = bob.start_subscribing().await;

        // One SUBSCRIBE reaches the publisher, from the origin, for the edge's
        // one subscription (§9.4).
        let upstream = publisher.accept_relay_subscribe().await;
        assert!(
            timeout(SOON, publisher.connection.accept_bi())
                .await
                .is_err()
        );
        let RequestEvent::SubscribeEstablished {
            track_alias: alice_alias,
            ..
        } = alice.answer(&mut alice_request).await
        else {
            panic!("Alice was not accepted");
        };
        let RequestEvent::SubscribeEstablished {
            track_alias: bob_alias,
            ..
        } = bob.answer(&mut bob_request).await
        else {
            panic!("Bob was not accepted");
        };

        publisher
            .send_group_as(upstream.track_alias, 0, &[object(0, 3), object(1, 3)])
            .await
            .finish();
        assert_eq!(alice.receive_stream(alice_alias).await, [0, 1]);
        assert_eq!(bob.receive_stream(bob_alias).await, [0, 1]);
        // The publisher and the edge at the origin; Alice, Bob and the origin
        // at the edge.
        assert_eq!(
            (origin.held().await.sessions, edge.held().await.sessions),
            (2, 3)
        );
    }

    /// §3.6 at an edge that goes away: it is drained once its own clients
    /// are gone, and its session with the origin is closed then, not when
    /// the GOAWAY's Timeout is over.
    #[tokio::test(start_paused = true)]
    async fn an_edge_that_goes_away_is_drained_once_its_clients_are_gone() {
        let (origin, _task) = hub::spawn(HubConfig::default());
        let edge = edge_of(&origin);
        let mut publisher = Peer::connect(&origin, true).await;
        let _namespace = publisher.publish_namespace().await;
        let mut alice = Peer::connect(&edge, true).await;
        let mut request = alice.start_subscribing().await;
        let _upstream = publisher.accept_relay_subscribe().await;
        alice.answer(&mut request).await;

        let going_away = {
            let edge = edge.clone();
            tokio::spawn(async move { edge.go_away(Vec::new(), 10_000).await })
        };
        tokio::time::sleep(SOON).await;
        assert!(!going_away.is_finished());
        alice.connection.close(0, b"");
        assert!(timeout(SOON, going_away).await.is_ok());
        tokio::time::sleep(SOON).await;
        assert_eq!(
            (origin.held().await.sessions, edge.held().await.sessions),
            (1, 0)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_edge_that_held_nothing_connects_again_for_its_next_subscriber() {
        let (origin, _task) = hub::spawn(HubConfig::default());
        let edge = edge_of(&origin);
        let mut publisher = Peer::connect(&origin, true).await;
        let _namespace = publisher.publish_namespace().await;
        let mut alice = Peer::connect(&edge, true).await;
        let mut request = alice.start_subscribing().await;
        let (id, mut send, _reader) = publisher.relay_subscribe().await;
        let (_, ok) = publisher
            .session
            .accept_subscribe(id, vec![], vec![])
            .unwrap();
        send_message(&mut send, &ok, &Trace::default(), "relay")
            .await
            .unwrap();
        alice.answer(&mut request).await;

        alice.connection.close(0, b"");
        // The time a session may hold nothing (30 seconds), at both ends of
        // the session between the relays.
        tokio::time::sleep(Duration::from_secs(35)).await;
        assert_eq!(
            (origin.held().await.sessions, edge.held().await.sessions),
            (1, 0)
        );
        // The origin cancelled its SUBSCRIBE with its last subscriber.
        publisher.session.on_peer_request_closed(id);

        let mut bob = Peer::connect(&edge, true).await;
        let mut request = bob.start_subscribing().await;
        let _upstream = publisher.accept_relay_subscribe().await;
        let event = bob.answer(&mut request).await;
        assert!(
            matches!(event, RequestEvent::SubscribeEstablished { .. }),
            "{event:?}"
        );
    }
}
