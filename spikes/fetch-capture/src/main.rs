//! Sends bytes that were captured elsewhere and prints the bytes that come
//! back, so that fixtures can be taken from a peer for messages and streams
//! the own client cannot ask for yet.
//!
//! Nothing here encodes or decodes MoQ Transport: what is sent is given in
//! hexadecimal, stream by stream, and what is received is printed as it came.
//!
//! ```text
//! xmoq-fetch-capture --url https://127.0.0.1:48600 --cert-sha256 <hex> \
//!     --control af000000 --request <hex of a SUBSCRIBE> --request <hex of a FETCH> \
//!     --wait-ms 2000
//! ```
//!
//! `--control` is written on a unidirectional stream that stays open;
//! each `--request` on a bidirectional stream of its own, in the order
//! given, 300 ms apart. After `--wait-ms` every stream is printed:
//!
//! ```text
//! RECV bidi 0 end=open 18 00 04 ...
//! RECV uni 1 end=fin 05 02 ...
//! ```
//!
//! `bidi n` is the answer on the n-th `--request`; `uni n` the n-th
//! unidirectional stream the peer opened, the first of which is its control
//! stream.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{Connection, ReadError, RecvStream, SendStream};

/// What was read from a stream, and how the stream stands.
struct Received {
    label: String,
    bytes: Vec<u8>,
    end: &'static str,
}

type Log = Arc<Mutex<Vec<Received>>>;

fn hex(text: &str) -> Result<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    if digits.len() % 2 != 0 {
        bail!("an odd number of hexadecimal digits");
    }
    digits
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)?;
            u8::from_str_radix(pair, 16).with_context(|| format!("{pair} is not hexadecimal"))
        })
        .collect()
}

/// Reads a stream to its end, keeping what came in the log as it comes.
async fn read_all(mut stream: impl RecvStream, label: String, log: Log) {
    let at = {
        let mut log = log.lock().expect("the log is not poisoned");
        log.push(Received { label, bytes: Vec::new(), end: "open" });
        log.len() - 1
    };
    let mut buffer = [0u8; 4096];
    loop {
        let read = stream.read(&mut buffer).await;
        let mut log = log.lock().expect("the log is not poisoned");
        match read {
            Ok(Some(count)) => log[at].bytes.extend_from_slice(&buffer[..count]),
            Ok(None) => {
                log[at].end = "fin";
                return;
            }
            Err(ReadError::Reset(_)) => {
                log[at].end = "reset";
                return;
            }
            Err(_) => {
                log[at].end = "lost";
                return;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let (mut url, mut cert, mut control, mut requests, mut wait_ms) = (None, None, None, Vec::new(), 2000);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--url" => url = Some(value),
            "--cert-sha256" => cert = Some(hex(&value)?),
            "--control" => control = Some(hex(&value)?),
            "--request" => requests.push(hex(&value)?),
            "--wait-ms" => wait_ms = value.parse().context("--wait-ms")?,
            other => bail!("unknown flag {other}"),
        }
    }
    let url = url.context("--url is required")?;
    let cert: [u8; 32] = cert
        .context("--cert-sha256 is required")?
        .try_into()
        .ok()
        .context("--cert-sha256 is 32 bytes")?;
    let control = control.context("--control is required")?;

    let connection = Arc::new(wt::connect(&url, cert).await.map_err(|lost| anyhow::anyhow!("{lost}"))?);
    let log: Log = Arc::default();

    let accepting = {
        let (connection, log) = (Arc::clone(&connection), Arc::clone(&log));
        tokio::spawn(async move {
            let mut count = 0;
            while let Ok(stream) = connection.accept_uni().await {
                tokio::spawn(read_all(stream, format!("uni {count}"), Arc::clone(&log)));
                count += 1;
            }
        })
    };

    // Kept, so that the stream stays open for the whole session.
    let mut control_stream = connection.open_uni().await.map_err(|lost| anyhow::anyhow!("{lost}"))?;
    control_stream.write_all(&control).await.map_err(|error| anyhow::anyhow!("{error:?}"))?;

    let mut kept = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (mut send, recv) = connection.open_bi().await.map_err(|lost| anyhow::anyhow!("{lost}"))?;
        send.write_all(request).await.map_err(|error| anyhow::anyhow!("{error:?}"))?;
        tokio::spawn(read_all(recv, format!("bidi {index}"), Arc::clone(&log)));
        kept.push(send);
    }

    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    accepting.abort();
    let mut log = log.lock().expect("the log is not poisoned");
    log.sort_by(|a, b| a.label.cmp(&b.label));
    for received in log.iter() {
        let bytes: Vec<String> = received.bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        println!("RECV {} end={} {}", received.label, received.end, bytes.join(" "));
    }
    connection.close(0, b"");
    Ok(())
}
