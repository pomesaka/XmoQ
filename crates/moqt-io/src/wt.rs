//! [`Connection`] over `wtransport` 0.7.2: WebTransport on HTTP/3.
//!
//! Everything that depends on the transport library is in this file, so that
//! replacing the library changes nothing else.
//!
//! Two things the library leaves to its user are done here:
//!
//! * Stream error codes are remapped into the HTTP/3 range of WebTransport on
//!   the way out and back on the way in (see [`crate::codes`]).
//! * The subprotocol is negotiated through the `wt-available-protocols` and
//!   `wt-protocol` header fields (`draft-ietf-moq-transport-18` §3.1.3).
//!
//! FIXME: send `WT_CLOSE_SESSION` when closing a session. `wtransport` 0.7.2
//! has no way to do it: [`Connection::close`] closes the QUIC connection, and a
//! browser then reports "Connection lost." without the Session Termination
//! Error Code or the reason. Checked on 2026-10-10 with Chromium 156.0.8078.4
//! (`spikes/wt-error-codes`). The code does arrive at a peer that reads the
//! QUIC close, such as another `wtransport` endpoint.

use std::sync::Arc;
use std::time::Duration;

use wtransport::config::QuicTransportConfig;
use wtransport::endpoint::endpoint_side::{Client, Server};
use wtransport::endpoint::{ConnectOptions, IncomingSession};
use wtransport::error::{StreamReadError, StreamWriteError};
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Endpoint, Identity, ServerConfig, VarInt};

use crate::{Connection, ConnectionLost, ReadError, RecvStream, SendStream, WriteError, codes};

/// Keeps a NAT binding and the idle timer alive while nothing is sent.
/// Provisional.
const KEEP_ALIVE: Duration = Duration::from_secs(3);

/// How long nothing may arrive from a peer before the connection counts as
/// lost. A peer that vanishes without closing is only noticed this way, and
/// until then its tracks and subscriptions stay.
///
/// WHY three keep-alive periods: this endpoint sends a keep-alive every
/// [`KEEP_ALIVE`] and the peer acknowledges it, whether or not the peer sends
/// keep-alives of its own (a browser does not). So a healthy connection is
/// never silent for that long, and two lost exchanges in a row do not end it.
///
/// Provisional. The default of quinn-proto 0.11.19 is 30 seconds
/// (`TransportConfig::default`, read 2026-10-10).
const IDLE_TIMEOUT: Duration = Duration::from_secs(9);

/// What one peer can make the QUIC layer of a server hold and open, below the
/// stream handling of this crate. All provisional.
///
/// WHY set at all: the defaults of quinn-proto 0.11.19 are 100 streams of
/// each kind, 1.25 MB of unread data a stream with no bound across streams,
/// and 10 MB of unsent data a connection (`config/transport.rs`,
/// `TransportConfig::default`, read 2026-10-10). One peer could have a server
/// hold about 250 MB that no limit of this crate sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuicLimits {
    /// Unidirectional streams the peer may have open at once, for `MoQ`.
    pub max_uni_streams: u32,
    /// Bidirectional streams the peer may have open at once, for `MoQ`.
    pub max_bi_streams: u32,
    /// Bytes of one stream that arrived and were not read yet.
    pub stream_receive_window: u32,
    /// The same, summed over all streams of the connection.
    pub receive_window: u32,
    /// Bytes written to all streams of the connection that the peer has not
    /// acknowledged yet.
    pub send_window: u32,
}

impl QuicLimits {
    fn transport(self) -> QuicTransportConfig {
        // WHY the additions: the limits of QUIC count every stream of the
        // connection. An HTTP/3 peer opens a control stream and may open the
        // two QPACK streams (RFC 9114 §6.2, RFC 9204 §4.2), and the WebTransport
        // session is the bidirectional stream of its CONNECT request. A
        // wtransport 0.7.2 client opens the control stream only, so it gets
        // two streams more than the limit says (`driver/mod.rs`, read
        // 2026-10-10).
        const HTTP3_UNI_STREAMS: u32 = 3;
        const CONNECT_STREAMS: u32 = 1;
        let mut transport = QuicTransportConfig::default();
        transport
            .max_concurrent_uni_streams(
                self.max_uni_streams
                    .saturating_add(HTTP3_UNI_STREAMS)
                    .into(),
            )
            .max_concurrent_bidi_streams(self.max_bi_streams.saturating_add(CONNECT_STREAMS).into())
            .stream_receive_window(self.stream_receive_window.into())
            .receive_window(self.receive_window.into())
            .send_window(u64::from(self.send_window));
        transport
    }
}

/// The header field a client lists its subprotocols in, and the one a server
/// answers with. Both hold structured-field strings, hence the quotes.
const AVAILABLE_PROTOCOLS: &str = "wt-available-protocols";
const PROTOCOL: &str = "wt-protocol";

fn quoted_protocol() -> String {
    format!("\"{}\"", xmoq_moqt::PROTOCOL)
}

fn stream_code(code: u32) -> VarInt {
    // The largest mapped code, 0x52e5ac983162, is far below the largest varint.
    VarInt::try_from_u64(codes::to_http3(code)).unwrap_or(VarInt::MAX)
}

/// The sending half of a `wtransport` stream.
pub struct WtSend(wtransport::SendStream);

/// The receiving half of a `wtransport` stream.
pub struct WtRecv(wtransport::RecvStream);

fn write_error(error: &StreamWriteError) -> WriteError {
    match error {
        StreamWriteError::Stopped(code) => {
            WriteError::Stopped(codes::from_http3(code.into_inner()))
        }
        other => WriteError::Lost(other.to_string()),
    }
}

impl SendStream for WtSend {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
        self.0
            .write_all(bytes)
            .await
            .map_err(|error| write_error(&error))
    }

    fn finish(self) {
        // WHY a drop and not `wtransport::SendStream::finish`: that method
        // waits until the peer acknowledged everything, and wtransport 0.7.2
        // offers no way to send the FIN without waiting. Dropping the stream
        // does: `impl Drop for SendStream` of quinn 0.11.12, which wtransport
        // wraps without a `Drop` of its own, calls `finish()`, and quinn goes
        // on delivering what was written (read 2026-10-10).
        drop(self.0);
    }

    async fn finish_and_wait(mut self) -> Result<(), WriteError> {
        // Completes when the peer acknowledged everything (wtransport 0.7.2,
        // `SendStream::finish`).
        self.0.finish().await.map_err(|error| write_error(&error))
    }

    fn reset(mut self, code: u32) {
        // An error only says the stream was already closed.
        let _ = self.0.reset(stream_code(code));
    }

    async fn stopped(&mut self) -> WriteError {
        write_error(&self.0.stopped().await)
    }
}

impl RecvStream for WtRecv {
    async fn read(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, ReadError> {
        match self.0.read(buffer).await {
            Ok(read) => Ok(read),
            Err(StreamReadError::Reset(code)) => {
                Err(ReadError::Reset(codes::from_http3(code.into_inner())))
            }
            Err(other) => Err(ReadError::Lost(other.to_string())),
        }
    }

    fn stop(self, code: u32) {
        self.0.stop(stream_code(code));
    }
}

/// A WebTransport session on `wtransport`.
pub struct WtConnection(wtransport::Connection);

fn lost(error: &dyn std::fmt::Display) -> ConnectionLost {
    ConnectionLost(error.to_string())
}

impl Connection for WtConnection {
    type Send = WtSend;
    type Recv = WtRecv;

    async fn open_uni(&self) -> Result<WtSend, ConnectionLost> {
        let opening = self.0.open_uni().await.map_err(|error| lost(&error))?;
        opening.await.map(WtSend).map_err(|error| lost(&error))
    }

    async fn open_bi(&self) -> Result<(WtSend, WtRecv), ConnectionLost> {
        let opening = self.0.open_bi().await.map_err(|error| lost(&error))?;
        let (send, recv) = opening.await.map_err(|error| lost(&error))?;
        Ok((WtSend(send), WtRecv(recv)))
    }

    async fn accept_uni(&self) -> Result<WtRecv, ConnectionLost> {
        self.0
            .accept_uni()
            .await
            .map(WtRecv)
            .map_err(|error| lost(&error))
    }

    async fn accept_bi(&self) -> Result<(WtSend, WtRecv), ConnectionLost> {
        let (send, recv) = self.0.accept_bi().await.map_err(|error| lost(&error))?;
        Ok((WtSend(send), WtRecv(recv)))
    }

    fn close(&self, code: u32, reason: &[u8]) {
        // See the FIXME at the top: this is not WT_CLOSE_SESSION.
        self.0.close(VarInt::from_u32(code), reason);
    }

    async fn closed(&self) {
        self.0.closed().await;
    }
}

/// Connects to a `MoQ` Transport endpoint at an `https` URL, offering the
/// subprotocol of this crate's draft.
///
/// The server certificate is accepted when its SHA-256 equals `cert_sha256`.
///
/// The QUIC windows and stream limits are the defaults of quinn: a client
/// talks to the one server it chose, and wtransport 0.7.2 has no way to set
/// them together with a pinned certificate hash.
///
/// # Errors
///
/// Fails when the connection or the WebTransport handshake fails.
pub async fn connect(url: &str, cert_sha256: [u8; 32]) -> Result<WtConnection, ConnectionLost> {
    // WHY pin the hash: test relays run on a self-signed certificate that no
    // root store trusts. Pinning keeps verification on.
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes([Sha256Digest::new(cert_sha256)])
        .keep_alive_interval(Some(KEEP_ALIVE))
        .max_idle_timeout(Some(IDLE_TIMEOUT))
        .map_err(|error| lost(&error))?
        .build();
    let options = ConnectOptions::builder(url)
        .add_header(AVAILABLE_PROTOCOLS, quoted_protocol())
        .build();
    let endpoint: Endpoint<Client> = Endpoint::client(config).map_err(|error| lost(&error))?;
    let connection = endpoint
        .connect(options)
        .await
        .map_err(|error| lost(&error))?;
    Ok(WtConnection(connection))
}

/// A WebTransport server that accepts `MoQ` Transport sessions.
pub struct WtServer(Arc<Endpoint<Server>>);

impl WtServer {
    /// Binds a UDP port on all interfaces with the certificate and key of the
    /// PEM files.
    ///
    /// # Errors
    ///
    /// Fails when the files cannot be loaded or the port cannot be bound.
    pub async fn bind(
        port: u16,
        cert_pem: &str,
        key_pem: &str,
        limits: QuicLimits,
    ) -> Result<Self, ConnectionLost> {
        let identity = Identity::load_pemfiles(cert_pem, key_pem)
            .await
            .map_err(|error| lost(&error))?;
        Self::with_identity(port, identity, limits)
    }

    fn with_identity(
        port: u16,
        identity: Identity,
        limits: QuicLimits,
    ) -> Result<Self, ConnectionLost> {
        let config = ServerConfig::builder()
            .with_bind_default(port)
            .with_custom_transport(identity, limits.transport())
            .keep_alive_interval(Some(KEEP_ALIVE))
            .max_idle_timeout(Some(IDLE_TIMEOUT))
            .map_err(|error| lost(&error))?
            .build();
        let endpoint = Endpoint::server(config).map_err(|error| lost(&error))?;
        Ok(Self(Arc::new(endpoint)))
    }

    /// The next connection attempt. Its handshake is still to be done, with
    /// [`accept`], so that a slow client does not hold up the others.
    pub async fn incoming(&self) -> Incoming {
        Incoming(self.0.accept().await)
    }
}

/// A client and a server session connected over the loopback interface, with
/// the server that has to outlive them. For the tests of the contract.
#[cfg(test)]
pub(crate) async fn loopback(limits: QuicLimits) -> (WtConnection, WtConnection, WtServer) {
    let identity = Identity::self_signed(["localhost", "127.0.0.1"]).expect("valid names");
    let hash = *identity.certificate_chain().as_slice()[0].hash().as_ref();
    let server = WtServer::with_identity(0, identity, limits).expect("a free port");
    let port = server.0.local_addr().expect("a bound socket").port();
    let url = format!("https://127.0.0.1:{port}");
    let (client, accepted) = tokio::join!(connect(&url, hash), async {
        accept(server.incoming().await).await
    });
    let (accepted, _) = accepted.expect("the server accepts");
    (client.expect("the client connects"), accepted, server)
}

/// A connection attempt whose handshake is still to be done.
pub struct Incoming(IncomingSession);

/// What the client asked for when it opened the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    /// The `:authority` of the CONNECT request.
    pub authority: String,
    /// The `:path` of the CONNECT request.
    pub path: String,
}

/// Completes the handshake of one connection attempt and returns the session
/// with what the client asked for.
///
/// A client that does not offer the subprotocol of this crate's draft is
/// answered with 404 and reported as an error.
///
/// # Errors
///
/// Fails when the handshake fails or the subprotocol is not offered.
pub async fn accept(incoming: Incoming) -> Result<(WtConnection, SessionRequest), ConnectionLost> {
    let request = incoming.0.await.map_err(|error| lost(&error))?;
    let asked = SessionRequest {
        authority: request.authority().to_string(),
        path: request.path().to_string(),
    };
    // WHY a substring test: the field is a structured-field list of strings.
    // The quoted token cannot appear inside another quoted token, and a full
    // parser for one comparison would be the larger risk.
    let offered = request
        .headers()
        .get(AVAILABLE_PROTOCOLS)
        .is_some_and(|protocols| protocols.contains(&quoted_protocol()));
    if !offered {
        request.not_found().await;
        return Err(ConnectionLost(format!(
            "the client did not offer {}",
            xmoq_moqt::PROTOCOL
        )));
    }
    let connection = request
        .accept_with_headers([(PROTOCOL, quoted_protocol())])
        .await
        .map_err(|error| lost(&error))?;
    Ok((WtConnection(connection), asked))
}
