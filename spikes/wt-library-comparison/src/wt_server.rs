//! Probe server on `wtransport` 0.7.2, with the paths and codes of
//! `wtq_server.rs`. The `/close`, `/reset`, `/stop`, `/observe` and `/protocol`
//! paths follow the probe of the `wt-error-codes` spike.
//!
//! With `--map`, stream codes go through the WebTransport-over-HTTP/3 mapping by
//! hand before they reach the library, which does not map them. With
//! `--default-transport`, the stream and window limits of `common.rs` are left out.

#[path = "common.rs"]
mod common;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use wtransport::endpoint::IncomingSession;
use wtransport::{Connection, Endpoint, Identity, RecvStream, ServerConfig, VarInt};

use common::{HOLD, RESET_CODE, SESSION_CODE, SESSION_REASON, STOP_CODE};

/// `webtransport_code_to_http_code` of draft-ietf-webtrans-http3, section 4.3:
/// `first + n + floor(n / 0x1e)`, which skips the reserved GREASE codepoints.
fn to_http3(code: u32) -> VarInt {
    const FIRST: u64 = 0x52e4_a40f_a8db;
    let code = u64::from(code);
    VarInt::try_from(FIRST + code + code / 0x1e).unwrap_or(VarInt::MAX)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, port, cert, key, rest @ ..] = args.as_slice() else {
        bail!("usage: <port> <cert.pem> <key.pem> [--map] [--default-transport]");
    };
    let map = rest.iter().any(|flag| flag == "--map");
    // The configuration of the wt-error-codes probe, to repeat its runs as they were.
    let default_transport = rest.iter().any(|flag| flag == "--default-transport");
    let identity = Identity::load_pemfiles(cert, key).await?;
    let builder = ServerConfig::builder().with_bind_default(port.parse().context("port")?);
    let config = if default_transport {
        builder
            .with_identity(identity)
            .keep_alive_interval(Some(Duration::from_secs(3)))
            .build()
    } else {
        builder
            .with_custom_transport(identity, common::transport_config())
            .build()
    };
    let server = Endpoint::server(config)?;
    println!("LISTENING port={port} library=wtransport map={map} default_transport={default_transport}");
    loop {
        let incoming = server.accept().await;
        tokio::spawn(async move {
            if let Err(error) = handle(incoming, map).await {
                println!("ERROR {error:#}");
            }
        });
    }
}

async fn handle(incoming: IncomingSession, map: bool) -> Result<()> {
    let request = incoming.await?;
    let path = request.path().to_string();
    let offered = request.headers().get("wt-available-protocols").cloned();
    println!("REQUEST path={path} wt-available-protocols={offered:?}");
    let stream_code = |code: u32| {
        if map {
            to_http3(code)
        } else {
            VarInt::from_u32(code)
        }
    };
    // The library has no subprotocol API; the header is written by hand.
    let connection = if offered.is_some_and(|list| list.contains(xmoq_moqt::PROTOCOL)) {
        let selected = format!("\"{}\"", xmoq_moqt::PROTOCOL);
        request
            .accept_with_headers([("wt-protocol", selected.as_str())])
            .await?
    } else {
        request.accept().await?
    };
    match path.as_str() {
        "/close" => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            connection.close(VarInt::from_u32(SESSION_CODE), SESSION_REASON);
            connection.closed().await;
        }
        "/reset" => {
            let mut stream = connection.open_uni().await?.await?;
            stream.write_all(b"x").await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            stream.reset(stream_code(RESET_CODE))?;
            connection.closed().await;
        }
        "/stop" => {
            let stream = connection.accept_uni().await?;
            stream.stop(stream_code(STOP_CODE));
            connection.closed().await;
        }
        // Reads what the peer does to its own streams and session.
        "/observe" | "/protocol" => {
            let mut stream = connection.accept_uni().await?;
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
            // `closed()` reports how the QUIC connection ended; the code of a
            // CLOSE_WEBTRANSPORT_SESSION capsule, if the library keeps it, comes
            // back from the next operation on the session.
            println!(
                "OBSERVED accept_uni after close={:?}",
                connection.accept_uni().await.err()
            );
            println!("OBSERVED session closed={:?}", connection.closed().await);
        }
        "/bidi" => bidi(&connection).await?,
        "/streams" => {
            tokio::time::sleep(HOLD).await;
            let mut accepted = 0;
            while let Ok(Ok(_stream)) =
                tokio::time::timeout(Duration::from_millis(200), connection.accept_uni()).await
            {
                accepted += 1;
            }
            println!("STREAMS accepted_after_hold={accepted}");
            connection.closed().await;
        }
        "/window" => {
            let before = connection.quic_connection().stats().udp_rx.bytes;
            tokio::time::sleep(HOLD).await;
            let after = connection.quic_connection().stats().udp_rx.bytes;
            println!("WINDOW udp_rx_bytes_while_unread={}", after - before);
            connection.closed().await;
        }
        other => println!("UNKNOWN path {other}"),
    }
    Ok(())
}

async fn read_to_fin(stream: &mut RecvStream) -> Result<Vec<u8>> {
    let mut received = Vec::new();
    let mut buffer = [0u8; 1024];
    while let Some(len) = stream.read(&mut buffer).await? {
        received.extend_from_slice(&buffer[..len]);
    }
    Ok(received)
}

async fn bidi(connection: &Connection) -> Result<()> {
    let (mut send, mut recv) = connection.accept_bi().await?;
    let received = read_to_fin(&mut recv).await?;
    println!("BIDI accepted, read to FIN: {:?}", String::from_utf8_lossy(&received));
    send.write_all(b"pong").await?;
    send.finish().await?;

    let (mut send, mut recv) = connection.open_bi().await?.await?;
    send.write_all(b"server-opened").await?;
    send.finish().await?;
    let received = read_to_fin(&mut recv).await?;
    println!("BIDI opened, read to FIN: {:?}", String::from_utf8_lossy(&received));
    connection.closed().await;
    Ok(())
}
