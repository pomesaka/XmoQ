//! Probe server on `web-transport-quinn` 0.13.2. The request path picks the
//! behaviour; the paths and codes are the ones `wt_server.rs` uses, so the two
//! libraries can be read side by side.
//!
//! The endpoint is built by hand instead of through `ServerBuilder`, because the
//! builder offers no way to set stream or window limits (it only takes an
//! address and a congestion controller; src/server.rs, read 2026-10-10).

#[path = "common.rs"]
mod common;

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use web_transport_quinn::proto::ConnectResponse;
use web_transport_quinn::quinn::crypto::rustls::QuicServerConfig;
use web_transport_quinn::quinn::rustls;
use web_transport_quinn::quinn::rustls::pki_types::pem::PemObject;
use web_transport_quinn::quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use web_transport_quinn::quinn::{self, Endpoint};
use web_transport_quinn::{Request, Server, Session};
use xmoq_moqt::message::MessageDecoder;
use xmoq_moqt::session::Role;

use common::{HOLD, RESET_CODE, SESSION_CODE, SESSION_REASON, STOP_CODE};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, port, cert, key, rest @ ..] = args.as_slice() else {
        bail!("usage: <port> <cert.pem> <key.pem> [--default-transport]");
    };
    // Leaves out the limits of `common.rs`, as `wt_server.rs` does with this flag.
    let default_transport = rest.iter().any(|flag| flag == "--default-transport");
    let chain = CertificateDer::pem_file_iter(cert)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    if !default_transport {
        config.transport_config(Arc::new(common::transport_config()));
    }
    let address = format!("127.0.0.1:{port}").parse().context("port")?;
    let mut server = Server::new(Endpoint::server(config, address)?);
    println!("LISTENING port={port} library=web-transport-quinn default_transport={default_transport}");
    while let Some(request) = server.accept().await {
        tokio::spawn(async move {
            if let Err(error) = handle(request).await {
                println!("ERROR {error:#}");
            }
        });
    }
    Ok(())
}

async fn handle(request: Request) -> Result<()> {
    let path = request.url.path().to_string();
    println!("REQUEST path={path} protocols={:?}", request.protocols);
    let mut response = ConnectResponse::OK;
    if request.protocols.iter().any(|offered| offered == xmoq_moqt::PROTOCOL) {
        response = response.with_protocol(xmoq_moqt::PROTOCOL);
    }
    let session = request.respond(response).await?;
    match path.as_str() {
        "/close" => {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            session.close(SESSION_CODE, SESSION_REASON);
            session.closed().await;
        }
        "/reset" => {
            let mut stream = session.open_uni().await?;
            stream.write_all(b"x").await?;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            stream.reset(RESET_CODE)?;
            session.closed().await;
        }
        "/stop" => {
            let mut stream = session.accept_uni().await?;
            stream.stop(STOP_CODE)?;
            session.closed().await;
        }
        // Reads what the peer does to its own streams and session.
        "/observe" | "/protocol" => observe(&session).await,
        "/bidi" => bidi(&session).await?,
        "/streams" => {
            tokio::time::sleep(HOLD).await;
            let mut accepted = 0;
            while let Ok(Ok(_stream)) =
                tokio::time::timeout(std::time::Duration::from_millis(200), session.accept_uni()).await
            {
                accepted += 1;
            }
            println!("STREAMS accepted_after_hold={accepted}");
            session.closed().await;
        }
        "/window" => {
            let before = quinn::Connection::stats(&session).udp_rx.bytes;
            tokio::time::sleep(HOLD).await;
            let after = quinn::Connection::stats(&session).udp_rx.bytes;
            println!("WINDOW udp_rx_bytes_while_unread={}", after - before);
            session.closed().await;
        }
        "/" => moqt(&session).await?,
        other => println!("UNKNOWN path {other}"),
    }
    Ok(())
}

async fn observe(session: &Session) {
    match session.accept_uni().await {
        Ok(mut stream) => {
            let mut buffer = [0u8; 64];
            loop {
                match stream.read(&mut buffer).await {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        println!("OBSERVED stream fin");
                        break;
                    }
                    Err(error) => {
                        println!("OBSERVED stream error={error:?}");
                        break;
                    }
                }
            }
            println!(
                "OBSERVED accept_uni after close={:?}",
                session.accept_uni().await.err()
            );
        }
        // A stream that is reset before its WebTransport header was read is
        // dropped inside the library (`log_header_error`, src/session.rs).
        Err(error) => println!("OBSERVED no stream surfaced, accept_uni error={error:?}"),
    }
    println!("OBSERVED session closed={:?}", session.closed().await);
}

async fn bidi(session: &Session) -> Result<()> {
    let (mut send, mut recv) = session.accept_bi().await?;
    let received = recv.read_to_end(1024).await?;
    println!("BIDI accepted, read to FIN: {:?}", String::from_utf8_lossy(&received));
    send.write_all(b"pong").await?;
    send.finish()?;

    let (mut send, mut recv) = session.open_bi().await?;
    send.write_all(b"server-opened").await?;
    send.finish()?;
    let received = recv.read_to_end(1024).await?;
    println!("BIDI opened, read to FIN: {:?}", String::from_utf8_lossy(&received));
    session.closed().await;
    Ok(())
}

/// The server side of a MOQT SETUP exchange, for moqtail's clients.
async fn moqt(session: &Session) -> Result<()> {
    println!("MOQT protocol={:?}", session.protocol());
    let (mut moqt, setup) = common::moqt_setup(Role::Server)?;
    // §3.3: kept open for the whole session.
    let mut control = session.open_uni().await?;
    control.write_all(&setup).await?;

    let mut peer_control = session.accept_uni().await?;
    let mut decoder = MessageDecoder::new();
    let mut buffer = [0u8; 4096];
    let message = loop {
        if let Some(message) = decoder.next_message()? {
            break message;
        }
        let Some(len) = peer_control.read(&mut buffer).await? else {
            bail!("the peer closed its control stream before SETUP");
        };
        decoder.push(&buffer[..len])?;
    };
    println!("MOQT SETUP_OK peer_setup={:?}", moqt.on_control_message(message)?);

    // The first request, if the peer sends one. Nothing answers it.
    let waited = tokio::time::timeout(std::time::Duration::from_secs(2), session.accept_bi()).await;
    if let Ok(Ok((_send, mut recv))) = waited {
        let mut decoder = MessageDecoder::new();
        while let Ok(Some(len)) = recv.read(&mut buffer).await {
            decoder.push(&buffer[..len])?;
            if let Some(message) = decoder.next_message()? {
                println!("MOQT REQUEST {message:?}");
                break;
            }
        }
    }
    session.close(0, b"spike done");
    println!("MOQT closed={:?}", session.closed().await);
    Ok(())
}
