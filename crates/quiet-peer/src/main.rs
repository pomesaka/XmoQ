//! A peer for the interop runs, and for nothing else: it takes WebTransport
//! sessions as a relay does, and stays quiet where a relay answers.
//!
//! WHY a program of its own, and no flag of the relay: a way not to answer
//! has no place in the binary that is run for real.
//!
//! `--quiet setup`: it sends no SETUP. `--quiet requests`: it sends its
//! SETUP, reads every request the client opens, and answers none.
//! `--quiet publish`: as `requests`, but a `PUBLISH_NAMESPACE` is answered
//! with `REQUEST_OK`. `--quiet streams`: it sends its SETUP and allows the
//! client no bidirectional stream, so that no request can be sent.
//! `--quiet subgroups`: it answers a `PUBLISH_NAMESPACE` and a PUBLISH with
//! `REQUEST_OK` and allows the client no unidirectional stream beyond its
//! control stream, so that no subgroup stream can be opened.
//!
//! `--connect <url> --cert-sha256 <hex> --stay-ms <n>`: the other way
//! round. It connects to a relay as a client, sends no SETUP and opens no
//! stream (with `--send-setup yes` it sends its SETUP first, and nothing
//! after it), and says how the relay ended the session
//! (`SESSION_ENDED ...`), or after that long that it did not
//! (`STAYED ms=N`).
//!
//! It prints what it saw, one line each, for a driver to wait for:
//!
//! - `LISTENING port=P`
//! - `SESSION session=N`
//! - `SETUP_RECEIVED session=N`
//! - `REQUEST session=N stream=M <the message>`: a message on a request
//!   stream, `PUBLISH_DONE` among them, with its fields.
//! - `REQUEST_CLOSED session=N stream=M how=FIN` or `how=reset, code 0x..`:
//!   how the client ended its direction of that stream.
//! - `SESSION_ENDED session=N ClosedByPeer { code: .., reason: .. }`: how
//!   the session ended.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{ControlMessage, RequestOk, setup_option};
use xmoq_moqt::session::{Role, Session, Transport};
use xmoq_moqt_io::wt::{Congestion, QuicLimits, WtConnection, WtServer, accept};
use xmoq_moqt_io::{
    Connection, RequestReader, RequestStreamEvent, StreamConfig, StreamStops, Trace, UniEvent,
    read_uni_streams, send_message,
};

/// What the peer leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quiet {
    /// Its SETUP, and everything after it.
    Setup,
    /// Its answers to requests.
    Requests,
    /// Room for a request: it sends its SETUP and allows the client no
    /// bidirectional stream.
    Streams,
    /// Room for a subgroup stream: it answers a `PUBLISH_NAMESPACE` and a
    /// PUBLISH with `REQUEST_OK`, as a subscriber that wants the track, and
    /// allows the client one unidirectional stream, which its control
    /// stream takes.
    Subgroups,
    /// Its answers to requests, but for a `PUBLISH_NAMESPACE`, which gets
    /// its `REQUEST_OK`: a client that announces before it publishes then
    /// comes to its PUBLISH.
    Publish,
}

/// What the program is asked to be.
enum Args {
    /// A server that takes sessions.
    Serve {
        port: u16,
        cert_file: String,
        key_file: String,
        quiet: Quiet,
    },
    /// A client that sends no SETUP.
    Connect {
        url: String,
        cert_sha256: [u8; 32],
        stay: Duration,
        /// `--send-setup yes`: it sends its SETUP, and nothing after it.
        setup: bool,
    },
}

/// 32 bytes from 64 hexadecimal digits.
fn sha256_of(text: &str) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if text.len() != 64 {
        bail!("--cert-sha256 is 64 hexadecimal digits");
    }
    for (byte, index) in bytes.iter_mut().zip((0..64).step_by(2)) {
        let digits = text.get(index..index + 2).context("--cert-sha256")?;
        *byte = u8::from_str_radix(digits, 16).context("--cert-sha256")?;
    }
    Ok(bytes)
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let (mut port, mut cert_file, mut key_file, mut quiet) = (None, None, None, None);
    let (mut url, mut cert_sha256, mut stay, mut setup) = (None, None, None, false);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .with_context(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--port" => port = Some(value.parse().context("--port")?),
            "--cert-file" => cert_file = Some(value),
            "--key-file" => key_file = Some(value),
            "--connect" => url = Some(value),
            "--cert-sha256" => cert_sha256 = Some(sha256_of(&value)?),
            "--send-setup" => setup = value == "yes",
            "--stay-ms" => {
                stay = Some(Duration::from_millis(value.parse().context("--stay-ms")?));
            }
            "--quiet" => {
                quiet = Some(match value.as_str() {
                    "setup" => Quiet::Setup,
                    "requests" => Quiet::Requests,
                    "publish" => Quiet::Publish,
                    "streams" => Quiet::Streams,
                    "subgroups" => Quiet::Subgroups,
                    other => bail!(
                        "--quiet is setup, requests, publish, streams or subgroups, not {other}"
                    ),
                });
            }
            other => bail!("unknown flag {other}"),
        }
    }
    if let Some(url) = url {
        return Ok(Args::Connect {
            url,
            cert_sha256: cert_sha256.context("--cert-sha256 is required")?,
            stay: stay.context("--stay-ms is required")?,
            setup,
        });
    }
    Ok(Args::Serve {
        port: port.context("--port is required")?,
        cert_file: cert_file.context("--cert-file is required")?,
        key_file: key_file.context("--key-file is required")?,
        quiet: quiet.context("--quiet is required")?,
    })
}

/// What a message on a request stream is printed as.
fn described(message: &ControlMessage) -> String {
    match message {
        ControlMessage::PublishDone(done) => format!(
            "PUBLISH_DONE status_code={:#x} stream_count={} reason={:?}",
            done.status_code,
            done.stream_count,
            String::from_utf8_lossy(done.error_reason.as_bytes())
        ),
        ControlMessage::Publish(_) => "PUBLISH".to_string(),
        ControlMessage::PublishNamespace(_) => "PUBLISH_NAMESPACE".to_string(),
        ControlMessage::Subscribe(_) => "SUBSCRIBE".to_string(),
        ControlMessage::Fetch(_) => "FETCH".to_string(),
        other => format!("{other:?}"),
    }
}

/// Reads one request stream to its end and prints what came. The sending
/// half is kept open and unused meanwhile: that is the answer that does
/// not come.
async fn read_request(
    session: u64,
    stream: u64,
    quiet: Quiet,
    halves: (
        <WtConnection as Connection>::Send,
        <WtConnection as Connection>::Recv,
    ),
) {
    let (mut unanswered, recv) = halves;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    loop {
        match reader.next_event().await {
            Ok(RequestStreamEvent::Message(message)) => {
                println!(
                    "REQUEST session={session} stream={stream} {}",
                    described(&message)
                );
                let answered = matches!(
                    (quiet, &message),
                    (
                        Quiet::Publish | Quiet::Subgroups,
                        ControlMessage::PublishNamespace(_)
                    ) | (Quiet::Subgroups, ControlMessage::Publish(_))
                );
                if answered {
                    let ok = ControlMessage::RequestOk(RequestOk::default());
                    let _ = send_message(&mut unanswered, &ok, &Trace::default(), "request").await;
                }
            }
            Ok(RequestStreamEvent::Closed(how)) => {
                println!("REQUEST_CLOSED session={session} stream={stream} how={how}");
                break;
            }
            Err(error) => {
                println!("REQUEST_FAILED session={session} stream={stream} {error}");
                break;
            }
        }
    }
    // WHY it waits here: letting go of the sending half finishes it, which
    // a client reads as the end of the request, and the half is to stay as
    // it is until the session is gone.
    std::future::pending::<()>().await;
}

async fn serve(session: u64, connection: WtConnection, quiet: Quiet) -> Result<()> {
    let connection = Arc::new(connection);
    println!("SESSION session={session}");
    let (queue, mut events) = tokio::sync::mpsc::channel(64);
    // WHY one stream is read where no subgroup stream is to have room: a
    // stream that is not taken keeps its place among the streams the
    // client may have open, so that the few a client has beyond its control
    // stream (how many depends on how its HTTP/3 layer counts) are used up
    // by its first Subgroups and not given back.
    let reading = if quiet == Quiet::Subgroups {
        StreamConfig {
            max_uni_streams: 1,
            ..StreamConfig::default()
        }
    } else {
        StreamConfig::default()
    };
    let _readers = read_uni_streams(
        Arc::clone(&connection),
        reading,
        StreamStops::default(),
        queue,
        |event| event,
    );
    // Kept to the end: a control stream that is let go of is finished,
    // which closes the session for the client (§3.3).
    let mut control = None;
    if quiet != Quiet::Setup {
        let mut moqt = Session::new(Role::Server, Transport::WebTransport);
        let setup = moqt.setup(vec![KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            b"xmoq-quiet-peer/0.0.0".to_vec(),
        )?])?;
        let stream = control.insert(connection.open_uni().await?);
        send_message(stream, &setup, &Trace::default(), "control").await?;
    }
    let mut requests = tokio::task::JoinSet::new();
    let mut streams = 0u64;
    let end = loop {
        tokio::select! {
            end = connection.closed() => break end,
            event = events.recv() => {
                if let Some(UniEvent::Control(ControlMessage::Setup(_))) = event {
                    println!("SETUP_RECEIVED session={session}");
                }
            }
            halves = connection.accept_bi() => {
                let Ok(halves) = halves else { continue };
                requests.spawn(read_request(session, streams, quiet, halves));
                streams += 1;
            }
        }
    };
    // What the requests' tasks still print is of streams that ended with
    // the session.
    requests.abort_all();
    println!("SESSION_ENDED session={session} {end:?}");
    drop(control);
    Ok(())
}

/// Connects, sends nothing, and says how the session ended or that it did
/// not.
async fn stay_quiet(url: &str, cert_sha256: [u8; 32], stay: Duration, setup: bool) -> Result<()> {
    let connection = xmoq_moqt_io::wt::connect(url, cert_sha256, &StreamConfig::default()).await?;
    println!("CONNECTED");
    // Kept to the end: a control stream that is let go of is finished.
    let mut control = None;
    if setup {
        let mut moqt = Session::new(Role::Client, Transport::WebTransport);
        let message = moqt.setup(vec![KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            b"xmoq-quiet-peer/0.0.0".to_vec(),
        )?])?;
        let stream = control.insert(connection.open_uni().await?);
        send_message(stream, &message, &Trace::default(), "control").await?;
        println!("SETUP_SENT");
    }
    if let Ok(end) = tokio::time::timeout(stay, connection.closed()).await {
        println!("SESSION_ENDED {end:?}");
    } else {
        println!("STAYED ms={}", stay.as_millis());
        connection.close(0, b"");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match parse_args(std::env::args().skip(1))? {
        Args::Connect {
            url,
            cert_sha256,
            stay,
            setup,
        } => return stay_quiet(&url, cert_sha256, stay, setup).await,
        Args::Serve {
            port,
            cert_file,
            key_file,
            quiet,
        } => (port, cert_file, key_file, quiet),
    };
    let (port, cert_file, key_file, quiet) = args;
    let streams = StreamConfig::default();
    let (max_uni_streams, max_bi_streams) = xmoq_moqt_io::wt::streams_read_at_once(&streams);
    let limits = QuicLimits {
        max_uni_streams: if quiet == Quiet::Subgroups {
            1
        } else {
            max_uni_streams
        },
        // The stream the session itself is on is allowed beside these.
        max_bi_streams: if quiet == Quiet::Streams {
            0
        } else {
            max_bi_streams
        },
        stream_receive_window: 256 * 1024,
        receive_window: 4 * 1024 * 1024,
        send_window: 4 * 1024 * 1024,
        // WHY a minute, where the relay has 6 seconds: the clients are to
        // give up by their own clocks, and the connection is not to go
        // first.
        idle_timeout: Duration::from_secs(60),
        congestion: Congestion::default(),
    };
    let server = WtServer::bind(port, &cert_file, &key_file, limits)?;
    println!("LISTENING port={port}");
    let mut session = 0u64;
    loop {
        let Some(incoming) = server.incoming().await else {
            bail!("the server stopped accepting");
        };
        let number = session;
        session += 1;
        tokio::spawn(async move {
            match accept(incoming).await {
                Ok((connection, _asked)) => {
                    if let Err(error) = serve(number, connection, quiet).await {
                        println!("SESSION_FAILED session={number} {error:#}");
                    }
                }
                Err(error) => println!("HANDSHAKE_FAILED {error}"),
            }
        });
    }
}
