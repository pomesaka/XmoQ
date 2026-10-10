//! Client on `web-transport-quinn` 0.13.2.
//!
//! `setup <url> <cert-sha256>` connects with the subprotocol `moqt-18` and
//! exchanges MOQT SETUP, which is what the moqtail relay is asked for.
//! `probe <base-url> <cert-sha256>` runs the `/close`, `/reset`, `/stop` and
//! `/observe` paths of the probe servers and prints what this side sees.

#[path = "common.rs"]
mod common;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use url::Url;
use web_transport_quinn::proto::ConnectRequest;
use web_transport_quinn::{Client, ClientBuilder, Session};
use xmoq_moqt::message::MessageDecoder;
use xmoq_moqt::session::Role;

use common::{RESET_CODE, SESSION_CODE};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, mode, url, cert_sha256] = args.as_slice() else {
        bail!("usage: setup|probe <url> <cert-sha256-hex>");
    };
    // The certificate stays verified: only this hash is accepted
    // (`ServerFingerprints`, src/client.rs).
    let client = ClientBuilder::new()
        .with_server_certificate_hashes(vec![common::parse_sha256(cert_sha256)?])?;
    match mode.as_str() {
        "setup" => setup(&client, url).await,
        "probe" => probe(&client, url).await,
        other => bail!("unknown mode {other}"),
    }
}

async fn connect(client: &Client, url: &str) -> Result<Session> {
    let request = ConnectRequest::new(Url::parse(url)?).with_protocol(xmoq_moqt::PROTOCOL);
    let session = client
        .connect(request)
        .await
        .context("WebTransport connect failed")?;
    println!("CONNECTED url={url} protocol={:?}", session.protocol());
    Ok(session)
}

async fn setup(client: &Client, url: &str) -> Result<()> {
    let session = connect(client, url).await?;
    let (mut moqt, setup) = common::moqt_setup(Role::Client)?;
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
    println!("SETUP_OK server_setup={:?}", moqt.on_control_message(message)?);

    session.close(0, b"");
    println!("CLOSED {:?}", session.closed().await);
    Ok(())
}

async fn probe(client: &Client, base: &str) -> Result<()> {
    {
        let session = connect(client, &format!("{base}/close")).await?;
        println!("PROBE close: closed={:?}", session.closed().await);
    }
    {
        let session = connect(client, &format!("{base}/reset")).await?;
        let mut stream = session.accept_uni().await?;
        let mut buffer = [0u8; 64];
        let end = loop {
            match stream.read(&mut buffer).await {
                Ok(Some(_)) => {}
                Ok(None) => break "fin".to_string(),
                Err(error) => break format!("{error:?}"),
            }
        };
        println!("PROBE reset: read ended with {end}");
        session.close(0, b"");
    }
    {
        let session = connect(client, &format!("{base}/stop")).await?;
        let mut stream = session.open_uni().await?;
        let mut end = "never failed".to_string();
        for _ in 0..200 {
            if let Err(error) = stream.write_all(&[0u8; 1024]).await {
                end = format!("{error:?}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        println!("PROBE stop: write ended with {end}");
        session.close(0, b"");
    }
    {
        let session = connect(client, &format!("{base}/observe")).await?;
        let mut stream = session.open_uni().await?;
        stream.write_all(&[1]).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        stream.reset(RESET_CODE)?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        session.close(SESSION_CODE, b"bye");
        println!("PROBE observe: closed={:?}", session.closed().await);
    }
    Ok(())
}
