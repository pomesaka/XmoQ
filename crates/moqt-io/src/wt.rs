//! [`Connection`] over `web-transport-quinn` 0.13.2: WebTransport on HTTP/3.
//!
//! Everything that depends on the transport library is in this file, so that
//! replacing the library changes nothing else. No type of the library or of
//! quinn appears in the public interface.
//!
//! What the library does, so this file does not:
//!
//! * Stream error codes are mapped into the HTTP/3 range of WebTransport on
//!   the way out and back on the way in (`draft-ietf-webtrans-http3-16` §4.4).
//!
//!   FIXME: report a received code on a reserved codepoint as a reset without
//!   a code. The range has codepoints of the form `0x1f * N + 0x21` that are
//!   not WebTransport codes (§4.4, §9.5). `web-transport-proto` 0.6.2 checks
//!   the range only (`error_from_http3`, `src/error.rs`, read 2026-10-10), so
//!   `0x52e4a40fa8f9` arrives here as the code `0x1e`. This file gets the
//!   decoded value and cannot tell.
//! * Closing a session sends its Session Termination Error Code and reason in
//!   a `WT_CLOSE_SESSION` capsule, which is what a browser reads them from.
//! * The subprotocol is negotiated through the `wt-available-protocols` and
//!   `wt-protocol` header fields (`draft-ietf-moq-transport-18` §3.1.3).
//!
//! What a peer that does not map its stream codes looks like: `wtransport`
//! 0.7.2, which the `moqtail` relay and clients are built on, puts a code on
//! the wire as it is. Such a code is outside the WebTransport range, and
//! arrives here as a reset or stop without a code ([`ReadError::Reset`] with
//! `None`), which §4.4 asks for. In the other direction that peer reads the
//! mapped value where it expects a `MoQ` code.
//!
//! One session takes one QUIC connection; the library does not pool.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use web_transport_quinn::generic::poll::SendStream as _;
use web_transport_quinn::http::StatusCode;
use web_transport_quinn::proto::{ConnectRequest, ConnectResponse};
use web_transport_quinn::quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use web_transport_quinn::quinn::rustls;
use web_transport_quinn::quinn::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use web_transport_quinn::quinn::rustls::crypto::CryptoProvider;
use web_transport_quinn::quinn::rustls::pki_types::pem::PemObject;
use web_transport_quinn::quinn::rustls::pki_types::{
    CertificateDer, PrivateKeyDer, ServerName, UnixTime,
};
use web_transport_quinn::quinn::{self, ConnectionError};
use web_transport_quinn::{Request, Session, SessionError, WebTransportError};

use crate::{
    Connection, ConnectionEnd, ConnectionLost, ReadError, RecvStream, SendStream, WriteError,
};

/// Keeps a NAT binding and the idle timer alive while nothing is sent.
/// Provisional.
const KEEP_ALIVE: Duration = Duration::from_secs(3);

/// How long nothing may arrive from a peer before the connection counts as
/// lost, in milliseconds. A peer that vanishes without closing is only noticed
/// this way, and until then its tracks and subscriptions stay.
///
/// WHY three keep-alive periods: the server sends a keep-alive every
/// [`KEEP_ALIVE`] and the peer acknowledges it, whether or not the peer sends
/// keep-alives of its own (a browser does not). So a healthy connection is
/// never silent for that long, and two lost exchanges in a row do not end it.
///
/// Provisional. The default of quinn-proto 0.11.19 is 30 seconds
/// (`TransportConfig::default`, read 2026-10-10).
const IDLE_TIMEOUT_MS: u32 = 9_000;

/// Bytes of datagrams a server keeps for one peer. Nothing here reads
/// datagrams (see the FIXME in the crate root), so what arrives is held up
/// to this many bytes; past that quinn drops the oldest (quinn-proto 0.11.19,
/// `config/transport.rs`; read 2026-10-10).
///
/// WHY not switched off: quinn would then leave datagram support out of its
/// transport parameters, while the library always announces datagrams in its
/// HTTP/3 settings and refuses a peer that does not (web-transport-proto
/// 0.6.2, `src/settings.rs`; read 2026-10-10). Whether a browser takes a
/// server that announces the one without the other was not tried. The value
/// is also the largest datagram frame quinn tells the peer it takes
/// (quinn-proto 0.11.19, `transport_parameters.rs`), so it is kept at the
/// size of one packet. Chromium 156 connects with it.
///
/// Provisional. quinn's default is 1.25 MB.
const DATAGRAM_RECEIVE_BUFFER: usize = 1500;

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
    fn transport(self) -> quinn::TransportConfig {
        // WHY the additions: the limits of QUIC count every stream of the
        // connection. An HTTP/3 peer opens a control stream and may open the
        // two QPACK streams (RFC 9114 §6.2, RFC 9204 §4.2), and the WebTransport
        // session is the bidirectional stream of its CONNECT request. A peer
        // that opens fewer of them gets that many streams more than the limit
        // says.
        const HTTP3_UNI_STREAMS: u32 = 3;
        const CONNECT_STREAMS: u32 = 1;
        let mut transport = quinn::TransportConfig::default();
        transport
            .max_concurrent_uni_streams(
                self.max_uni_streams
                    .saturating_add(HTTP3_UNI_STREAMS)
                    .into(),
            )
            .max_concurrent_bidi_streams(self.max_bi_streams.saturating_add(CONNECT_STREAMS).into())
            .stream_receive_window(self.stream_receive_window.into())
            .receive_window(self.receive_window.into())
            .send_window(u64::from(self.send_window))
            .datagram_receive_buffer_size(Some(DATAGRAM_RECEIVE_BUFFER))
            .keep_alive_interval(Some(KEEP_ALIVE))
            .max_idle_timeout(Some(quinn::VarInt::from_u32(IDLE_TIMEOUT_MS).into()));
        transport
    }
}

/// The sending half of a stream.
pub struct WtSend(web_transport_quinn::SendStream);

/// The receiving half of a stream.
pub struct WtRecv(web_transport_quinn::RecvStream);

fn write_error(error: &web_transport_quinn::WriteError) -> WriteError {
    match error {
        web_transport_quinn::WriteError::Stopped(code) => WriteError::Stopped(Some(*code)),
        // A code outside the WebTransport range: see the module documentation.
        web_transport_quinn::WriteError::InvalidStopped(_) => WriteError::Stopped(None),
        other => WriteError::Lost(other.to_string()),
    }
}

impl WtSend {
    /// Waits until the stream is over for the peer: it sent `STOP_SENDING`
    /// (an error), or it acknowledged everything up to a FIN or took a reset
    /// (`Ok`).
    ///
    /// WHY the poll method of the library and not its `stopped()`: `stopped()`
    /// returns the same `None` for "over" and for a `STOP_SENDING` whose code
    /// is outside the WebTransport range, which is what every `wtransport`
    /// peer sends. `poll_closed` tells the two apart (web-transport-quinn
    /// 0.13.2, `src/send.rs`, read 2026-10-10). The library keeps the
    /// registration inside the stream, so dropping this future loses nothing.
    async fn over(&mut self) -> Result<(), WriteError> {
        std::future::poll_fn(|context| self.0.poll_closed(context))
            .await
            .map_err(|error| write_error(&error))
    }
}

impl SendStream for WtSend {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
        self.0
            .write_all(bytes)
            .await
            .map_err(|error| write_error(&error))
    }

    fn finish(mut self) {
        // An error only says the stream was already closed.
        let _ = self.0.finish();
    }

    async fn finish_and_wait(mut self) -> Result<(), WriteError> {
        let _ = self.0.finish();
        self.over().await
    }

    fn reset(mut self, code: u32) {
        // An error only says the stream was already closed.
        let _ = self.0.reset(code);
    }

    async fn stopped(&mut self) -> WriteError {
        match self.over().await {
            Err(error) => error,
            // `finish` and `reset` take the half, so a stream this is called
            // on is neither finished nor reset. Should the library report it
            // over all the same, nothing more can be written to it.
            Ok(()) => WriteError::Lost("the stream is closed".to_string()),
        }
    }
}

impl RecvStream for WtRecv {
    async fn read(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, ReadError> {
        match self.0.read(buffer).await {
            Ok(read) => Ok(read),
            Err(web_transport_quinn::ReadError::Reset(code)) => Err(ReadError::Reset(Some(code))),
            // A code outside the WebTransport range: see the module
            // documentation.
            Err(web_transport_quinn::ReadError::InvalidReset(_)) => Err(ReadError::Reset(None)),
            Err(other) => Err(ReadError::Lost(other.to_string())),
        }
    }

    fn stop(mut self, code: u32) {
        // An error only says the stream was already closed.
        let _ = self.0.stop(code);
    }
}

/// A WebTransport session.
pub struct WtConnection {
    session: Session,
    /// The endpoint of a session this side opened, which has to outlive it.
    _client: Option<quinn::Endpoint>,
}

fn lost(error: &dyn std::fmt::Display) -> ConnectionLost {
    ConnectionLost(error.to_string())
}

/// The smallest application error code an HTTP/3 stack uses for itself
/// (`H3_NO_ERROR`, RFC 9114 §8.1).
const FIRST_HTTP3_ERROR_CODE: u32 = 0x100;

fn connection_end(error: SessionError) -> ConnectionEnd {
    match error {
        SessionError::WebTransportError(WebTransportError::Closed(code, reason)) => {
            ConnectionEnd::ClosedByPeer { code, reason }
        }
        // WHY read a QUIC close as a session close: a peer that sends no
        // WT_CLOSE_SESSION capsule, which is every `wtransport` 0.7.2 peer,
        // closes the QUIC connection with its Session Termination Error Code
        // as the application code. The library reports that as a connection
        // error because the code is outside the WebTransport range.
        //
        // WHY NOT every code: an HTTP/3 stack closes its connection the same
        // way, with a code of HTTP/3 (0x100 to 0x110, RFC 9114 §8.1) or of
        // QPACK (0x200 to 0x202, RFC 9204 §6). Those are not `MoQ` codes, and
        // no Session Termination Error Code of draft 18 is that large.
        SessionError::ConnectionError(ConnectionError::ApplicationClosed(close)) => {
            match u32::try_from(close.error_code.into_inner()) {
                Ok(code) if code < FIRST_HTTP3_ERROR_CODE => ConnectionEnd::ClosedByPeer {
                    code,
                    reason: String::from_utf8_lossy(&close.reason).into_owned(),
                },
                _ => ConnectionEnd::Lost(close.to_string()),
            }
        }
        SessionError::ConnectionError(ConnectionError::LocallyClosed) => {
            ConnectionEnd::ClosedLocally
        }
        other => ConnectionEnd::Lost(other.to_string()),
    }
}

impl Connection for WtConnection {
    type Send = WtSend;
    type Recv = WtRecv;

    async fn open_uni(&self) -> Result<WtSend, ConnectionLost> {
        let send = self.session.open_uni().await;
        send.map(WtSend).map_err(|error| lost(&error))
    }

    async fn open_bi(&self) -> Result<(WtSend, WtRecv), ConnectionLost> {
        let (send, recv) = self.session.open_bi().await.map_err(|error| lost(&error))?;
        Ok((WtSend(send), WtRecv(recv)))
    }

    async fn accept_uni(&self) -> Result<WtRecv, ConnectionLost> {
        let recv = self.session.accept_uni().await;
        recv.map(WtRecv).map_err(|error| lost(&error))
    }

    async fn accept_bi(&self) -> Result<(WtSend, WtRecv), ConnectionLost> {
        let (send, recv) = self
            .session
            .accept_bi()
            .await
            .map_err(|error| lost(&error))?;
        Ok((WtSend(send), WtRecv(recv)))
    }

    fn close(&self, code: u32, reason: &[u8]) {
        // The library sends the reason as it is given, whatever its length.
        self.session
            .close(code, crate::close_reason(reason).as_bytes());
        // WHY a task that holds the session: the library writes the capsule in
        // a task of its own and closes the connection when the peer answered,
        // or after max(3 x RTT, 100 ms) (`Session::close`, read 2026-10-10).
        // The session also holds the HTTP/3 streams that must stay open until
        // then, and the caller may drop this value right after closing.
        let session = self.session.clone();
        tokio::spawn(async move {
            session.closed().await;
        });
    }

    async fn closed(&self) -> ConnectionEnd {
        connection_end(self.session.closed().await)
    }
}

/// How long a client waits for its session, from the first packet to the
/// answer to its CONNECT request. Provisional; the same 10 seconds a server
/// gives a handshake.
///
/// WHY a limit of its own: quinn gives up on a handshake that gets no answer
/// only after its idle timeout, 30 seconds by default, and says "timed out"
/// without the addresses. A client that cannot connect should say so sooner
/// and say where it was sending from.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Accepts the server certificate whose SHA-256 is the pinned one.
///
/// WHY written here and not taken from the library: `ClientBuilder` of
/// web-transport-quinn 0.13.2 has the same verifier, but builds its endpoint
/// on `[::]:0` in the same call and hands out neither the endpoint nor the
/// configuration (`src/client.rs`, `ClientBuilder::build`, read 2026-10-10);
/// see [`local_address_for`] for why that socket will not do. What the
/// builder sets is repeated in [`client_config`]: TLS 1.3 only, the ALPN
/// `h3`, no client certificate, and quinn's default transport settings.
#[derive(Debug)]
struct PinnedCertificate {
    provider: Arc<CryptoProvider>,
    sha256: [u8; 32],
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let hash = web_transport_quinn::crypto::sha256(&self.provider, end_entity);
        if hash.as_ref() == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    // The two below keep verification on: the server still has to prove that
    // it holds the key of the pinned certificate.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The QUIC and TLS settings of a client that pins the server certificate.
///
/// The QUIC settings are the defaults of quinn, without a keep-alive: a
/// client talks to the one server it chose, and the server's keep-alive keeps
/// the connection up.
fn client_config(cert_sha256: [u8; 32]) -> Result<quinn::ClientConfig, ConnectionLost> {
    // WHY `ring`, named here: see `WtServer::with_certificate`.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    // WHY pin the hash: test relays run on a self-signed certificate that no
    // root store trusts. Pinning keeps verification on: the hash is compared
    // and the handshake signature is still checked.
    let verifier = Arc::new(PinnedCertificate {
        provider: Arc::clone(&provider),
        sha256: cert_sha256,
    });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| lost(&error))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
    let crypto = QuicClientConfig::try_from(tls).map_err(|error| lost(&error))?;
    Ok(quinn::ClientConfig::new(Arc::new(crypto)))
}

/// The address a client binds its UDP socket to, for a server at `remote`: any
/// port, on the wildcard address of the same family.
///
/// WHY not `[::]:0` for every server, which takes IPv4 and IPv6 at once: on
/// macOS the kernel can give such a socket a port that an IPv4 socket of
/// another process already holds, and then delivers what comes from an IPv4
/// server to that other socket. The server's answers never arrive and the
/// handshake does not end (`docs/experiments/2026-10-10-client-socket-family.md`).
/// An IPv4 socket cannot get a port another IPv4 socket holds.
fn local_address_for(remote: SocketAddr) -> SocketAddr {
    let wildcard = match remote {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    SocketAddr::new(wildcard, 0)
}

/// The name the server certificate is checked against, and the address to
/// send to.
///
/// WHY the first address of a host name and no other: the resolver orders
/// its answers by the system's policy (RFC 6724), which is the order the
/// library takes too. Trying the next one after a failure would multiply the
/// time a failed connect takes, and a relay that is reachable over one family
/// only is a setup to fix, not to work around here.
async fn resolve(url: &url::Url) -> Result<(String, SocketAddr), ConnectionLost> {
    let port = url.port().unwrap_or(443);
    match url.host() {
        Some(url::Host::Ipv4(address)) => Ok((address.to_string(), (address, port).into())),
        Some(url::Host::Ipv6(address)) => Ok((address.to_string(), (address, port).into())),
        Some(url::Host::Domain(domain)) => {
            let remote = tokio::net::lookup_host((domain, port))
                .await
                .map_err(|error| lost(&error))?
                .next()
                .ok_or_else(|| ConnectionLost(format!("{domain} has no address")))?;
            Ok((domain.to_string(), remote))
        }
        None => Err(ConnectionLost("the URL has no host".to_string())),
    }
}

/// Connects to a `MoQ` Transport endpoint at an `https` URL, offering the
/// subprotocol of this crate's draft.
///
/// The server certificate is accepted when its SHA-256 equals `cert_sha256`.
///
/// # Errors
///
/// Fails when the URL is not one, when the connection or the WebTransport
/// handshake fails or takes more than 10 seconds, and when the server does
/// not select the subprotocol.
pub async fn connect(url: &str, cert_sha256: [u8; 32]) -> Result<WtConnection, ConnectionLost> {
    connect_within(url, cert_sha256, CONNECT_TIMEOUT, None).await
}

/// [`connect`] with the time limit and the local address of the caller's
/// choice; `None` is [`local_address_for`] the server.
async fn connect_within(
    url: &str,
    cert_sha256: [u8; 32],
    limit: Duration,
    local: Option<SocketAddr>,
) -> Result<WtConnection, ConnectionLost> {
    let url: url::Url = url.parse().map_err(|error| lost(&error))?;
    let (server_name, remote) = resolve(&url).await?;
    let local = local.unwrap_or_else(|| local_address_for(remote));
    let endpoint = quinn::Endpoint::client(local).map_err(|error| lost(&error))?;
    let config = client_config(cert_sha256)?;
    let request = ConnectRequest::new(url).with_protocol(xmoq_moqt::PROTOCOL);
    let establish = async {
        let connection = endpoint
            .connect_with(config, remote, &server_name)
            .map_err(|error| lost(&error))?
            .await
            .map_err(|error| lost(&error))?;
        Session::connect(connection, request)
            .await
            .map_err(|error| lost(&error))
    };
    let Ok(session) = tokio::time::timeout(limit, establish).await else {
        let from = endpoint
            .local_addr()
            .map_or_else(|error| error.to_string(), |address| address.to_string());
        return Err(ConnectionLost(format!(
            "no session with {remote} within {limit:?}, sending from {from}"
        )));
    };
    let session = session?;
    // The library fails the connect when the server selects a protocol that
    // was not offered; a server that selects none is refused here.
    if session.protocol() != Some(xmoq_moqt::PROTOCOL) {
        session.close(0, b"");
        return Err(ConnectionLost(format!(
            "the server did not select {}",
            xmoq_moqt::PROTOCOL
        )));
    }
    Ok(WtConnection {
        session,
        _client: Some(endpoint),
    })
}

/// A WebTransport server that accepts `MoQ` Transport sessions.
pub struct WtServer(quinn::Endpoint);

impl WtServer {
    /// Binds a UDP port on all interfaces with the certificate chain and key
    /// of the PEM files.
    ///
    /// # Errors
    ///
    /// Fails when the files cannot be loaded or the port cannot be bound.
    pub fn bind(
        port: u16,
        cert_pem: &str,
        key_pem: &str,
        limits: QuicLimits,
    ) -> Result<Self, ConnectionLost> {
        let chain = CertificateDer::pem_file_iter(cert_pem)
            .and_then(Iterator::collect::<Result<Vec<_>, _>>)
            .map_err(|error| lost(&error))?;
        let key = PrivateKeyDer::from_pem_file(key_pem).map_err(|error| lost(&error))?;
        Self::with_certificate(port, chain, key, limits)
    }

    /// WHY built by hand and not with the library's `ServerBuilder`: the
    /// builder takes an address and a congestion controller only, so the QUIC
    /// limits could not be set (web-transport-quinn 0.13.2, `src/server.rs`,
    /// read 2026-10-10).
    fn with_certificate(
        port: u16,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        limits: QuicLimits,
    ) -> Result<Self, ConnectionLost> {
        // WHY `ring`, named here: it is the one backend compiled in, and the
        // library's default lookup panics when a dependency elsewhere in the
        // binary turns on a second one.
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut tls = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| lost(&error))?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(|error| lost(&error))?;
        tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
        let crypto = QuicServerConfig::try_from(tls).map_err(|error| lost(&error))?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        config.transport_config(Arc::new(limits.transport()));
        let address = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let endpoint = quinn::Endpoint::server(config, address).map_err(|error| lost(&error))?;
        Ok(Self(endpoint))
    }

    /// The next connection attempt, or `None` when the server is closed. Its
    /// handshake is still to be done, with [`accept`], so that a slow client
    /// does not hold up the others.
    ///
    /// WHY not the library's `Server::accept`: it runs every handshake inside
    /// itself, without a limit on their number or their time.
    pub async fn incoming(&self) -> Option<Incoming> {
        self.0.accept().await.map(Incoming)
    }
}

/// A connection attempt whose handshake is still to be done.
pub struct Incoming(quinn::Incoming);

/// What the client asked for when it opened the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    /// The authority of the URL of the CONNECT request.
    pub authority: String,
    /// The path of that URL.
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
    let connection = incoming.0.await.map_err(|error| lost(&error))?;
    let request = Request::accept(connection)
        .await
        .map_err(|error| lost(&error))?;
    let asked = SessionRequest {
        authority: request.url.authority().to_string(),
        path: request.url.path().to_string(),
    };
    let offered = request
        .protocols
        .iter()
        .any(|protocol| protocol == xmoq_moqt::PROTOCOL);
    if !offered {
        // An error means the client is gone already.
        let _ = request.reject(StatusCode::NOT_FOUND).await;
        return Err(ConnectionLost(format!(
            "the client did not offer {}",
            xmoq_moqt::PROTOCOL
        )));
    }
    let response = ConnectResponse::OK.with_protocol(xmoq_moqt::PROTOCOL);
    let session = request
        .respond(response)
        .await
        .map_err(|error| lost(&error))?;
    Ok((
        WtConnection {
            session,
            _client: None,
        },
        asked,
    ))
}

/// A server on the loopback interface, with the URL and the certificate hash
/// a client needs to reach it.
#[cfg(test)]
fn loopback_server(limits: QuicLimits) -> (WtServer, String, [u8; 32]) {
    use web_transport_quinn::quinn::rustls::pki_types::PrivatePkcs8KeyDer;

    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("a self-signed certificate");
    let certificate = CertificateDer::from(generated.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        generated.signing_key.serialize_der(),
    ));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let hash: [u8; 32] = web_transport_quinn::crypto::sha256(&provider, &certificate)
        .as_ref()
        .try_into()
        .expect("SHA-256 is 32 bytes");
    let server =
        WtServer::with_certificate(0, vec![certificate], key, limits).expect("a free port");
    let port = server.0.local_addr().expect("a bound socket").port();
    (server, format!("https://127.0.0.1:{port}/"), hash)
}

/// A client and a server session connected over the loopback interface, with
/// the server that has to outlive them. For the tests of the contract.
#[cfg(test)]
pub(crate) async fn loopback(limits: QuicLimits) -> (WtConnection, WtConnection, WtServer) {
    let (server, url, hash) = loopback_server(limits);
    let (client, accepted) = tokio::join!(connect(&url, hash), async {
        accept(server.incoming().await.expect("an attempt")).await
    });
    let (accepted, _) = accepted.expect("the server accepts");
    (client.expect("the client connects"), accepted, server)
}

#[cfg(test)]
mod tests {
    use web_transport_quinn::quinn::{ApplicationClose, VarInt};

    use super::*;

    #[test]
    fn a_quic_close_without_a_capsule_is_read_as_a_session_close() {
        let closed_with = |code: u32| {
            connection_end(SessionError::ConnectionError(
                ConnectionError::ApplicationClosed(ApplicationClose {
                    error_code: VarInt::from_u32(code),
                    reason: b"bye".as_slice().into(),
                }),
            ))
        };
        // PROTOCOL_VIOLATION (0x3), as a `wtransport` peer closes.
        assert_eq!(
            closed_with(0x3),
            ConnectionEnd::ClosedByPeer {
                code: 0x3,
                reason: "bye".to_string(),
            }
        );
        // H3_NO_ERROR (0x100) and QPACK_DECOMPRESSION_FAILED (0x200) are an
        // HTTP/3 stack speaking for itself.
        for code in [0x100, 0x200] {
            assert!(
                matches!(closed_with(code), ConnectionEnd::Lost(_)),
                "{code:#x}"
            );
        }
    }

    #[test]
    fn a_client_binds_a_socket_of_the_servers_address_family() {
        // (the server, the local address)
        let cases = [
            ("127.0.0.1:4433", "0.0.0.0:0"),
            ("203.0.113.7:443", "0.0.0.0:0"),
            ("[::1]:4433", "[::]:0"),
            ("[2001:db8::7]:443", "[::]:0"),
        ];
        for (remote, local) in cases {
            let remote: SocketAddr = remote.parse().unwrap();
            assert_eq!(local_address_for(remote).to_string(), local, "{remote}");
        }
    }

    const TEST_LIMITS: QuicLimits = QuicLimits {
        max_uni_streams: 8,
        max_bi_streams: 8,
        stream_receive_window: 65_536,
        receive_window: 262_144,
        send_window: 262_144,
    };

    /// Runs the handshakes of every attempt that reaches the server.
    fn accept_all(server: WtServer) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(incoming) = server.incoming().await {
                tokio::spawn(async move {
                    // The sessions end with the test.
                    if let Ok((connection, _)) = accept(incoming).await {
                        connection.closed().await;
                    }
                });
            }
        })
    }

    #[tokio::test]
    async fn a_port_that_an_ipv4_socket_of_someone_else_holds_does_not_stop_a_client() {
        let (server, url, hash) = loopback_server(TEST_LIMITS);
        let accepting = accept_all(server);
        // Someone else's IPv4 socket, on a port the kernel chose.
        let held = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = held.local_addr().unwrap().port();

        // A socket for both families on that port is what used to be bound.
        // macOS allows it, and gives the server's answers to the IPv4 socket:
        // the client hears nothing, and now says so when its time is up. Other
        // systems may refuse the bind, which is an error at once.
        let both_families = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
        let limit = Duration::from_millis(1500);
        let Err(ConnectionLost(failure)) =
            connect_within(&url, hash, limit, Some(both_families)).await
        else {
            panic!("connected through a port whose IPv4 traffic goes elsewhere");
        };
        #[cfg(target_os = "macos")]
        assert_eq!(
            failure,
            format!(
                "no session with {} within 1.5s, sending from [::]:{port}",
                url.trim_start_matches("https://").trim_end_matches('/')
            )
        );
        // WHY not the same text: where the bind is refused the client never
        // gets as far as waiting. A stall here would mean this system hands
        // the port out like macOS does, which is worth a failing test.
        #[cfg(not(target_os = "macos"))]
        assert!(
            !failure.starts_with("no session with"),
            "the bind was expected to fail, but the client waited: {failure}"
        );

        // The socket a client binds for an IPv4 server cannot get that port,
        // and the client connects while the other socket is still there.
        let client = connect(&url, hash).await.expect("the client connects");
        let WtConnection {
            _client: Some(endpoint),
            ..
        } = &client
        else {
            panic!("a client keeps its endpoint");
        };
        let local = endpoint.local_addr().expect("a bound socket");
        assert!(local.is_ipv4(), "{local}");
        assert_ne!(local.port(), port);
        drop(held);
        accepting.abort();
    }
}
