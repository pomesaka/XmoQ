//! The contract of [`Connection`], [`SendStream`] and [`RecvStream`] as tests
//! that run against every implementation: the in-memory pair, and a
//! `wtransport` client and server over the loopback interface.
//!
//! WHY both: the stream handling and the relay are tested on the in-memory
//! connection. Such a test says something about a real connection only where
//! the two behave alike, and that is what these cases pin down. A connection
//! over another library is added to the list at the bottom and has to pass
//! the same cases.
//!
//! The cases run in real time, since a real connection does not follow tokio's
//! paused clock.

use std::future::Future;
use std::time::Duration;

use tokio::time::timeout;

use crate::{Connection, ReadError, RecvStream, SendStream, WriteError};

/// Longer than anything here takes on a loopback interface.
const IN_TIME: Duration = Duration::from_secs(5);
/// Long enough to tell "waits" from "is about to complete".
const A_WHILE: Duration = Duration::from_millis(300);

async fn soon<T>(what: &str, future: impl Future<Output = T>) -> T {
    let Ok(value) = timeout(IN_TIME, future).await else {
        panic!("timed out: {what}");
    };
    value
}

/// Reads until the stream ends, and returns what came and how it ended.
async fn read_to_end(recv: &mut impl RecvStream) -> (Vec<u8>, Result<(), ReadError>) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 64];
    loop {
        match soon("a read", recv.read(&mut buffer)).await {
            Ok(Some(read)) => bytes.extend_from_slice(&buffer[..read]),
            Ok(None) => return (bytes, Ok(())),
            Err(error) => return (bytes, Err(error)),
        }
    }
}

async fn read_exactly(recv: &mut impl RecvStream, len: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 64];
    while bytes.len() < len {
        let want = (len - bytes.len()).min(buffer.len());
        let read = soon("a read", recv.read(&mut buffer[..want]))
            .await
            .expect("the stream is open")
            .expect("the stream has not ended");
        bytes.extend_from_slice(&buffer[..read]);
    }
    bytes
}

/// Opens a unidirectional stream from `a`, writes `bytes`, and accepts it at
/// `b`. A stream shows at the peer once something was written to it.
async fn uni<C: Connection>(a: &C, b: &C, bytes: &[u8]) -> (C::Send, C::Recv) {
    let mut send = soon("open_uni", a.open_uni()).await.expect("open");
    soon("a write", send.write_all(bytes)).await.expect("write");
    let recv = soon("accept_uni", b.accept_uni()).await.expect("accept");
    (send, recv)
}

fn is_lost_read<T: std::fmt::Debug>(result: &Result<T, ReadError>) -> bool {
    matches!(result, Err(ReadError::Lost(_)))
}

pub(crate) async fn bytes_arrive_in_order_and_the_fin_after_them<C: Connection>(a: C, b: C) {
    let (mut send, mut recv) = uni(&a, &b, b"abc").await;
    send.write_all(b"def").await.expect("write");
    send.finish();
    assert_eq!(read_to_end(&mut recv).await, (b"abcdef".to_vec(), Ok(())));

    let (mut send, mut recv) = soon("open_bi", a.open_bi()).await.expect("open");
    send.write_all(b"ping").await.expect("write");
    let (mut peer_send, mut peer_recv) = soon("accept_bi", b.accept_bi()).await.expect("accept");
    assert_eq!(read_exactly(&mut peer_recv, 4).await, b"ping");
    peer_send.write_all(b"pong").await.expect("write");
    peer_send.finish();
    send.finish();
    assert_eq!(read_to_end(&mut recv).await, (b"pong".to_vec(), Ok(())));
    assert_eq!(read_to_end(&mut peer_recv).await, (vec![], Ok(())));
}

pub(crate) async fn a_reset_and_a_stop_carry_their_codes<C: Connection>(a: C, b: C) {
    // CANCELLED (0x1) one way, MALFORMED_TRACK (0x12) the other.
    let (send, mut recv) = uni(&a, &b, b"abc").await;
    send.reset(0x1);
    let (_, end) = read_to_end(&mut recv).await;
    assert_eq!(end, Err(ReadError::Reset(Some(0x1))));

    let (mut send, recv) = uni(&a, &b, b"abc").await;
    recv.stop(0x12);
    assert_eq!(
        soon("stopped", send.stopped()).await,
        WriteError::Stopped(Some(0x12))
    );
    assert_eq!(
        soon("a write", send.write_all(b"d")).await,
        Err(WriteError::Stopped(Some(0x12)))
    );
}

pub(crate) async fn dropping_a_read_loses_nothing<C: Connection>(a: C, b: C) {
    let (mut send, mut recv) = uni(&a, &b, b"abc").await;
    assert_eq!(read_exactly(&mut recv, 3).await, b"abc");
    // Nothing is there to read: the call waits, and is dropped.
    assert!(timeout(A_WHILE, recv.read(&mut [0; 8])).await.is_err());
    send.write_all(b"def").await.expect("write");
    send.finish();
    assert_eq!(read_to_end(&mut recv).await, (b"def".to_vec(), Ok(())));
}

pub(crate) async fn stopped_waits_until_the_peer_stops<C: Connection>(a: C, b: C) {
    let (mut send, recv) = uni(&a, &b, b"abc").await;
    assert!(timeout(A_WHILE, send.stopped()).await.is_err());
    // Dropping that call lost nothing either.
    recv.stop(0x1);
    assert_eq!(
        soon("stopped", send.stopped()).await,
        WriteError::Stopped(Some(0x1))
    );
}

/// What reached the peer before the connection went can still be read there.
/// `finish_and_wait` is what tells the sender that it did.
pub(crate) async fn what_arrived_outlives_the_connection<C: Connection>(a: C, b: C) {
    let (send, mut recv) = uni(&a, &b, b"abc").await;
    soon("finish_and_wait", send.finish_and_wait())
        .await
        .expect("everything arrives");
    a.close(0, b"done");
    soon("closed", b.closed()).await;
    assert_eq!(read_to_end(&mut recv).await, (b"abc".to_vec(), Ok(())));
}

/// The same for a stream nobody has accepted yet.
pub(crate) async fn a_stream_that_arrived_can_be_accepted_after_the_connection_went<
    C: Connection,
>(
    a: C,
    b: C,
) {
    let mut send = soon("open_uni", a.open_uni()).await.expect("open");
    send.write_all(b"abc").await.expect("write");
    soon("finish_and_wait", send.finish_and_wait())
        .await
        .expect("everything arrives");
    a.close(0, b"done");
    soon("closed", b.closed()).await;
    let mut recv = soon("accept_uni", b.accept_uni())
        .await
        .expect("the stream had arrived");
    assert_eq!(read_to_end(&mut recv).await, (b"abc".to_vec(), Ok(())));
    // Nothing else had: now the connection shows as gone.
    assert!(soon("accept_uni", b.accept_uni()).await.is_err());
}

pub(crate) async fn a_read_fails_once_the_connection_is_gone<C: Connection>(a: C, b: C) {
    // Closed by the peer, with nothing left to read: no FIN was sent.
    let (_send, mut recv) = uni(&a, &b, b"abc").await;
    let (_other_send, mut other_recv) = uni(&a, &b, b"abc").await;
    assert_eq!(read_exactly(&mut recv, 3).await, b"abc");
    assert_eq!(read_exactly(&mut other_recv, 3).await, b"abc");
    a.close(0, b"");
    soon("closed", b.closed()).await;
    assert!(is_lost_read(&soon("a read", recv.read(&mut [0; 8])).await));
    // A read that was already waiting ends the same way.
    assert!(is_lost_read(
        &soon("a read", other_recv.read(&mut [0; 8])).await
    ));
}

pub(crate) async fn a_read_fails_after_closing_the_connection_oneself<C: Connection>(a: C, b: C) {
    let (_send, mut recv) = uni(&a, &b, b"abc").await;
    assert_eq!(read_exactly(&mut recv, 3).await, b"abc");
    b.close(0, b"");
    soon("closed", b.closed()).await;
    assert!(is_lost_read(&soon("a read", recv.read(&mut [0; 8])).await));
}

pub(crate) async fn nothing_is_opened_or_accepted_once_the_connection_is_gone<C: Connection>(
    a: C,
    b: C,
) {
    a.close(0, b"");
    soon("closed", a.closed()).await;
    soon("closed", b.closed()).await;
    for end in [&a, &b] {
        assert!(soon("accept_uni", end.accept_uni()).await.is_err());
        assert!(soon("accept_bi", end.accept_bi()).await.is_err());
        assert!(soon("open_uni", end.open_uni()).await.is_err());
        assert!(soon("open_bi", end.open_bi()).await.is_err());
    }
}

pub(crate) async fn an_accept_that_waits_fails_when_the_connection_goes<C: Connection>(a: C, b: C) {
    let (accepted, ()) = tokio::join!(soon("accept_uni", b.accept_uni()), async {
        tokio::time::sleep(A_WHILE).await;
        a.close(0, b"");
    });
    assert!(accepted.is_err());
}

pub(crate) async fn writing_fails_once_the_connection_is_gone<C: Connection>(a: C, b: C) {
    let (mut send, _recv) = uni(&a, &b, b"abc").await;
    let (mut waiting, _other_recv) = uni(&a, &b, b"abc").await;
    let (stopped, ()) = tokio::join!(soon("stopped", waiting.stopped()), async {
        tokio::time::sleep(A_WHILE).await;
        b.close(0, b"");
    });
    assert!(matches!(stopped, WriteError::Lost(_)), "{stopped:?}");
    soon("closed", a.closed()).await;
    let stopped = soon("stopped", send.stopped()).await;
    assert!(matches!(stopped, WriteError::Lost(_)), "{stopped:?}");
    let written = soon("a write", send.write_all(b"d")).await;
    assert!(matches!(written, Err(WriteError::Lost(_))), "{written:?}");
    let finished = soon("finish_and_wait", send.finish_and_wait()).await;
    assert!(matches!(finished, Err(WriteError::Lost(_))), "{finished:?}");
}

pub(crate) async fn finish_and_wait_fails_on_a_stopped_stream<C: Connection>(a: C, b: C) {
    let (mut send, recv) = uni(&a, &b, b"abc").await;
    recv.stop(0x1);
    assert_eq!(
        soon("stopped", send.stopped()).await,
        WriteError::Stopped(Some(0x1))
    );
    assert_eq!(
        soon("finish_and_wait", send.finish_and_wait()).await,
        Err(WriteError::Stopped(Some(0x1)))
    );
}

/// Opening waits once the peer has `at_most` streams of this endpoint that it
/// has not dealt with, or fewer, and goes on when the peer has dealt with them.
pub(crate) async fn opening_waits_at_the_peers_stream_limit<C: Connection>(
    a: C,
    b: C,
    at_most: usize,
) {
    let mut open = Vec::new();
    let waited = loop {
        assert!(
            open.len() <= at_most,
            "{} streams were opened without waiting",
            open.len()
        );
        match timeout(A_WHILE, a.open_uni()).await {
            Ok(stream) => {
                let mut send = stream.expect("open");
                send.write_all(b"x").await.expect("write");
                open.push(send);
            }
            Err(_) => break open.len(),
        }
    };
    assert!(waited > 0);
    // The peer takes every stream to its end, which makes room.
    // WHY not while the next open is waiting: the peer would then see that
    // stream among the others, in no order this test could rely on.
    for send in open {
        send.finish();
    }
    for _ in 0..waited {
        let mut recv = soon("accept_uni", b.accept_uni()).await.expect("accept");
        assert_eq!(read_to_end(&mut recv).await, (b"x".to_vec(), Ok(())));
    }
    assert!(soon("open_uni", a.open_uni()).await.is_ok());
}

/// Writing waits while the peer does not read, and goes on when it does.
pub(crate) async fn writing_waits_while_the_peer_does_not_read<C: Connection>(
    a: C,
    b: C,
    more_than_a_window: usize,
) {
    let (mut send, mut recv) = uni(&a, &b, b"abc").await;
    let bytes = vec![0x61; more_than_a_window];
    assert!(timeout(A_WHILE, send.write_all(&bytes)).await.is_err());
    // Some prefix was written, and the window is full: a further write waits
    // as well, until the peer reads.
    assert!(timeout(A_WHILE, send.write_all(b"z")).await.is_err());
    let reading = async {
        let mut buffer = [0; 1024];
        while let Ok(Some(_)) = recv.read(&mut buffer).await {}
    };
    tokio::select! {
        written = soon("a write", send.write_all(b"z")) => assert_eq!(written, Ok(())),
        () = reading => panic!("the stream ended"),
    }
}

/// Runs every case against one implementation. `$pair` makes two connected
/// ends and whatever has to outlive them.
macro_rules! contract_tests {
    (
        $module:ident,
        pair: $pair:expr,
        streams_before_opening_waits: $streams:expr,
        more_than_a_window: $window:expr $(,)?
    ) => {
        mod $module {
            contract_tests!(@plain $pair, [
                bytes_arrive_in_order_and_the_fin_after_them,
                a_reset_and_a_stop_carry_their_codes,
                dropping_a_read_loses_nothing,
                stopped_waits_until_the_peer_stops,
                what_arrived_outlives_the_connection,
                a_stream_that_arrived_can_be_accepted_after_the_connection_went,
                a_read_fails_once_the_connection_is_gone,
                a_read_fails_after_closing_the_connection_oneself,
                nothing_is_opened_or_accepted_once_the_connection_is_gone,
                an_accept_that_waits_fails_when_the_connection_goes,
                writing_fails_once_the_connection_is_gone,
                finish_and_wait_fails_on_a_stopped_stream,
            ]);

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn opening_waits_at_the_peers_stream_limit() {
                let (a, b, _keep) = $pair.await;
                super::opening_waits_at_the_peers_stream_limit(a, b, $streams).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn writing_waits_while_the_peer_does_not_read() {
                let (a, b, _keep) = $pair.await;
                super::writing_waits_while_the_peer_does_not_read(a, b, $window).await;
            }
        }
    };
    (@plain $pair:expr, [$($case:ident),* $(,)?]) => {
        $(
            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn $case() {
                let (a, b, _keep) = $pair.await;
                super::$case(a, b).await;
            }
        )*
    };
}

contract_tests!(
    memory,
    pair: async {
        let (a, b) = crate::memory::pair();
        (a, b, ())
    },
    streams_before_opening_waits: crate::memory::ACCEPT_QUEUE,
    more_than_a_window: 4 * crate::memory::STREAM_WINDOW,
);

/// Small limits, so that the cases reach them with little data.
#[cfg(feature = "wtransport")]
const LOOPBACK_LIMITS: crate::wt::QuicLimits = crate::wt::QuicLimits {
    max_uni_streams: 8,
    max_bi_streams: 8,
    stream_receive_window: 4096,
    receive_window: 16 * 1024,
    send_window: 16 * 1024,
};

#[cfg(feature = "wtransport")]
contract_tests!(
    wtransport,
    pair: crate::wt::loopback(crate::contract::LOOPBACK_LIMITS),
    // The 8 of the limits, and the 2 that a wtransport client leaves unused of
    // the 3 streams HTTP/3 is given room for (it opens no QPACK streams).
    streams_before_opening_waits: 10,
    more_than_a_window: 64 * 1024,
);
