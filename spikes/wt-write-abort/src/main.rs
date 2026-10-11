//! Spike, outside lint and review. A WebTransport server on
//! `web-transport-quinn` 0.13.2, the library of the own relay. The request
//! path picks what it does to the streams the client opens:
//!
//! - `/stop`: sends `STOP_SENDING` (code 7) for each, 100 ms after it came
//! - `/stop-now`: the same at once, as a relay does that decides on the
//!   first bytes of a stream
//! - `/close`: holds them, and closes the session (code 9) after 400 ms
//! - `/hold`: holds them and does nothing, for a client that closes the
//!   session itself, and for a server that is killed
//!
//! Nothing is read from a stream, so a large write of the client stays
//! pending.
//!
//! The paths that begin with `/peer-` are about a bidirectional stream the
//! server opens, as a relay opens one for a SUBSCRIBE of its own. The server
//! writes one byte on it, so that the client sees the stream, and then:
//!
//! - `/peer-stop`: sends `STOP_SENDING` (code 7) for the client's direction
//!   after 100 ms; `/peer-stop-now`: at once
//! - `/peer-cancel`: after 100 ms resets its own direction (code 5) and
//!   sends `STOP_SENDING` (code 7), as a relay does that cancels a request;
//!   `/peer-cancel-now`: at once
//! - `/peer-close`: closes the session (code 9) after 400 ms
//! - `/peer-hold`: does nothing
//!
//! The paths that begin with `/moqt-` are for a page that runs `@xmoq/moqt`:
//! the server sends a SETUP (`XMOQ_SETUP_HEX`) on a control stream, opens a
//! bidirectional stream with a request, and reads the answer to its FIN.
//!
//! - `/moqt-refused`: a TRACK_STATUS, which the session refuses by itself;
//!   `/moqt-rejected`: a SUBSCRIBE (`XMOQ_SUBSCRIBE_HEX`), for the page to
//!   reject. The server then leaves its direction open and prints when
//!   it is stopped, and with what code
//! - `/moqt-refused-fin`: the TRACK_STATUS, and the server finishes its
//!   direction when it has the answer

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use web_transport_quinn::proto::ConnectResponse;
use web_transport_quinn::quinn::crypto::rustls::QuicServerConfig;
use web_transport_quinn::quinn::rustls;
use web_transport_quinn::quinn::rustls::pki_types::pem::PemObject;
use web_transport_quinn::quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use web_transport_quinn::quinn::{self, Endpoint};
use web_transport_quinn::{RecvStream, Request, Server, Session};

const STOP_CODE: u32 = 7;
const RESET_CODE: u32 = 5;
const SESSION_CODE: u32 = 9;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, port, cert, key] = args.as_slice() else {
        bail!("usage: <port> <cert.pem> <key.pem>");
    };
    let chain = CertificateDer::pem_file_iter(cert)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    tls.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    let mut transport = quinn::TransportConfig::default();
    // Short, so that a client notices a killed server within seconds.
    // `XMOQ_IDLE_SECS` is for the `/moqt-` paths, where a session sends
    // nothing for longer than that.
    let idle = match std::env::var("XMOQ_IDLE_SECS") {
        Ok(seconds) => seconds.parse().context("XMOQ_IDLE_SECS")?,
        Err(_) => 3,
    };
    transport.max_idle_timeout(Some(Duration::from_secs(idle).try_into()?));
    config.transport_config(Arc::new(transport));
    let address = format!("127.0.0.1:{port}").parse().context("port")?;
    let mut server = Server::new(Endpoint::server(config, address)?);
    println!("LISTENING port={port}");
    while let Some(request) = server.accept().await {
        tokio::spawn(async move {
            if let Err(error) = handle(request).await {
                println!("ERROR {error:#}");
            }
        });
    }
    Ok(())
}

/// The receiving sides of the streams the client opens, as they come.
async fn next_stream(session: &Session) -> Result<RecvStream> {
    tokio::select! {
        uni = session.accept_uni() => Ok(uni?),
        bidi = session.accept_bi() => Ok(bidi?.1),
    }
}

async fn handle(request: Request) -> Result<()> {
    let path = request.url.path().to_string();
    // The `/moqt-` paths are for `@xmoq/moqt`, which asks for this protocol.
    let response = if path.starts_with("/moqt-") {
        ConnectResponse::OK.with_protocol("moqt-18")
    } else {
        ConnectResponse::OK
    };
    let session = request.respond(response).await?;
    let mut held = Vec::new();
    match path.as_str() {
        "/stop" | "/stop-now" => {
            let wait = if path == "/stop" { 100 } else { 0 };
            while let Ok(mut stream) = next_stream(&session).await {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(wait)).await;
                    let _ = stream.stop(STOP_CODE);
                    // Kept, so that nothing but the stop reaches the client.
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    drop(stream);
                });
            }
        }
        "/close" => {
            let closing = tokio::time::sleep(Duration::from_millis(400));
            tokio::pin!(closing);
            loop {
                tokio::select! {
                    () = &mut closing => break,
                    stream = next_stream(&session) => held.push(stream?),
                }
            }
            session.close(SESSION_CODE, b"bye from the server");
            session.closed().await;
        }
        "/peer-stop" | "/peer-stop-now" | "/peer-cancel" | "/peer-cancel-now" => {
            let wait = if path.ends_with("-now") { 0 } else { 100 };
            let cancel = path.starts_with("/peer-cancel");
            let (mut send, mut recv) = session.open_bi().await?;
            send.write_all(&[1]).await?;
            tokio::time::sleep(Duration::from_millis(wait)).await;
            if cancel {
                let _ = send.reset(RESET_CODE);
            }
            let _ = recv.stop(STOP_CODE);
            // Kept, so that nothing else reaches the client.
            let ended = session.closed().await;
            println!("ENDED {path} {ended:?}");
            drop((send, recv));
        }
        "/peer-close" => {
            let (mut send, recv) = session.open_bi().await?;
            send.write_all(&[1]).await?;
            tokio::time::sleep(Duration::from_millis(400)).await;
            session.close(SESSION_CODE, b"bye from the server");
            session.closed().await;
            drop((send, recv));
        }
        "/peer-hold" => {
            let (mut send, recv) = session.open_bi().await?;
            send.write_all(&[1]).await?;
            session.closed().await;
            drop((send, recv));
        }
        "/moqt-refused" | "/moqt-rejected" | "/moqt-refused-fin" => {
            let hex = |name: &str| -> Result<Vec<u8>> {
                let text: String = std::env::var(name)
                    .with_context(|| format!("{name} is not set"))?
                    .split_whitespace()
                    .collect();
                (0..text.len() / 2)
                    .map(|at| Ok(u8::from_str_radix(&text[at * 2..at * 2 + 2], 16)?))
                    .collect()
            };
            let mut control = session.open_uni().await?;
            control.write_all(&hex("XMOQ_SETUP_HEX")?).await?;
            let request = if path == "/moqt-rejected" {
                hex("XMOQ_SUBSCRIBE_HEX")?
            } else {
                vec![0x0d, 0x00, 0x02, 0x01, 0x00]
            };
            let (mut send, mut recv) = session.open_bi().await?;
            send.write_all(&request).await?;
            let answer = recv.read_to_end(1024).await?;
            let answered = std::time::Instant::now();
            let shown: Vec<String> = answer.iter().map(|byte| format!("{byte:02x}")).collect();
            println!("ANSWER {path} {} and a FIN", shown.join(" "));
            if path == "/moqt-refused-fin" {
                send.finish()?;
                println!("FINISHED {path}");
            } else {
                let code = send.stopped().await;
                println!(
                    "STOPPED {path} code={code:?} after_ms={}",
                    answered.elapsed().as_millis()
                );
            }
            let ended = session.closed().await;
            println!("ENDED {path} {ended:?}");
            drop((control, send, recv));
        }
        "/hold" => {
            while let Ok(stream) = next_stream(&session).await {
                held.push(stream);
            }
        }
        other => println!("UNKNOWN path {other}"),
    }
    Ok(())
}
