//! Probe: what a browser sees when a `wtransport` 0.7.2 server closes a session,
//! resets a stream or stops one, each with an error code.
//!
//! The request path picks the behaviour. With `--map`, stream codes are sent
//! through the WebTransport-over-HTTP/3 mapping first, to see whether doing the
//! mapping by hand is enough.

use std::time::Duration;

use anyhow::{Context, Result};
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, ServerConfig, VarInt};

const SESSION_CODE: u32 = 0x3;
const RESET_CODE: u32 = 0x12;
const STOP_CODE: u32 = 0x5;

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
        anyhow::bail!("usage: <port> <cert.pem> <key.pem> [--map]");
    };
    let map = rest.iter().any(|flag| flag == "--map");
    let identity = Identity::load_pemfiles(cert, key).await?;
    let config = ServerConfig::builder()
        .with_bind_default(port.parse().context("port")?)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    let server = Endpoint::server(config)?;
    println!("LISTENING port={port} map={map}");
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
    println!(
        "REQUEST path={path} wt-available-protocols={:?}",
        request.headers().get("wt-available-protocols")
    );
    let stream_code = |code: u32| {
        if map {
            to_http3(code)
        } else {
            VarInt::from_u32(code)
        }
    };
    let connection = if path == "/protocol" {
        request
            .accept_with_headers([("wt-protocol", "\"moqt-18\"")])
            .await?
    } else {
        request.accept().await?
    };
    match path.as_str() {
        "/close" => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            connection.close(VarInt::from_u32(SESSION_CODE), b"protocol violation");
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
        // Reads what the browser does to its own streams and session.
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
            // CLOSE_WEBTRANSPORT_SESSION capsule, if wtransport keeps it, comes
            // back from the next operation on the session.
            println!(
                "OBSERVED accept_uni after close={:?}",
                connection.accept_uni().await.err()
            );
            println!("OBSERVED session closed={:?}", connection.closed().await);
        }
        other => println!("UNKNOWN path {other}"),
    }
    Ok(())
}
