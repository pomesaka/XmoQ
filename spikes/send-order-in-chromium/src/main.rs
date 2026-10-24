//! Spike, outside lint and review. A WebTransport server on
//! `web-transport-quinn` 0.13.2 whose connection receive window is smaller
//! than what the client's unidirectional streams can carry together, so that
//! the client waits for connection credit. It reads the unidirectional
//! streams at `RATE` bytes a second together, round robin, and a
//! bidirectional stream at once; for the bidirectional stream it prints
//! when its whole request (`REQUEST` bytes) had arrived, in milliseconds
//! since the Unix epoch, for the page to compare with when it wrote it.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use tokio::sync::Mutex;
use web_transport_quinn::quinn::crypto::rustls::QuicServerConfig;
use web_transport_quinn::quinn::rustls;
use web_transport_quinn::quinn::rustls::pki_types::pem::PemObject;
use web_transport_quinn::quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use web_transport_quinn::quinn::{self, Endpoint};
use web_transport_quinn::{Request, Server};

const CONNECTION_WINDOW: u32 = 256 * 1024;
const STREAM_WINDOW: u32 = 64 * 1024;
/// Bytes a second the unidirectional streams are read at, together.
const RATE: usize = 512 * 1024;
/// A read of a unidirectional stream; `RATE / CHUNK` of them a second.
const CHUNK: usize = 4 * 1024;
const REQUEST: usize = 64 * 1024;

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

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
    transport
        .receive_window(CONNECTION_WINDOW.into())
        .stream_receive_window(STREAM_WINDOW.into())
        .max_idle_timeout(Some(Duration::from_secs(10).try_into()?));
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

async fn handle(request: Request) -> Result<()> {
    let session = request.respond(web_transport_quinn::proto::ConnectResponse::OK).await?;
    // The streams being read, and the one whose turn it is.
    let streams = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let streams = Arc::clone(&streams);
        tokio::spawn(async move {
            let mut turn = 0usize;
            let mut buffer = vec![0u8; CHUNK];
            let pause = Duration::from_secs_f64(CHUNK as f64 / RATE as f64);
            loop {
                tokio::time::sleep(pause).await;
                let mut streams = streams.lock().await;
                if streams.is_empty() {
                    continue;
                }
                turn %= streams.len();
                let stream: &mut web_transport_quinn::RecvStream = &mut streams[turn];
                // A read that has nothing at once is left for the next turn.
                let _ = tokio::time::timeout(Duration::from_millis(1), stream.read(&mut buffer)).await;
                turn += 1;
            }
        })
    };
    loop {
        tokio::select! {
            uni = session.accept_uni() => streams.lock().await.push(uni?),
            bidi = session.accept_bi() => {
                let (_send, mut recv) = bidi?;
                tokio::spawn(async move {
                    let mut got = 0usize;
                    let mut buffer = vec![0u8; 16 * 1024];
                    while got < REQUEST {
                        match recv.read(&mut buffer).await {
                            Ok(Some(read)) => got += read,
                            _ => break,
                        }
                    }
                    println!("REQUEST bytes={got} at_ms={}", now_ms());
                });
            }
        }
    }
    #[allow(unreachable_code)]
    {
        reader.abort();
        Ok(())
    }
}
