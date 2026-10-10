//! Interop peer: a MOQT draft-18 client built on the `moqtail` crate.
//!
//! Connects to a moqtail relay over WebTransport and either subscribes to one
//! track and prints the first Object it receives, or publishes one Object. The
//! driver (`interop/run.ts`) parses the `SUBSCRIBED`, `OBJECT`, `PUBLISHED` and
//! `SENT` lines on stdout, so their format is part of the contract with that script.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::{ControlMessage, ControlMessageTrait};
use moqtail::model::control::publish::Publish;
use moqtail::model::control::setup::{Setup, SetupSender};
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::constant::DEFAULT_PUBLISHER_PRIORITY;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::connection::TransportConnection;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, RecvDataStream, SendDataStream};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, RwLock};
use wtransport::endpoint::ConnectOptions;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Endpoint};

/// WebTransport subprotocol offered in `wt-available-protocols`. The relay at
/// 0.15.0 accepts only this value and answers 403 otherwise.
const MOQT_VERSION: &str = "moqt-18";

/// Track alias this client announces in PUBLISH. Any value works: the relay maps
/// it to its own alias per subscriber.
const PUBLISH_TRACK_ALIAS: u64 = 1;

enum Mode {
  Subscribe,
  /// Publishes one Object carrying this payload.
  Publish { payload: Vec<u8> },
}

struct Args {
  mode: Mode,
  url: String,
  cert_sha256: String,
  namespace: String,
  track: String,
  timeout_secs: u64,
}

fn parse_args() -> Result<Args> {
  let mut mode = None;
  let mut payload_hex = None;
  let mut url = None;
  let mut cert_sha256 = None;
  let mut namespace = None;
  let mut track = None;
  let mut timeout_secs = 20;

  let mut args = std::env::args().skip(1);
  while let Some(flag) = args.next() {
    let value = args
      .next()
      .ok_or_else(|| anyhow!("missing value for {flag}"))?;
    match flag.as_str() {
      "--mode" => mode = Some(value),
      "--payload-hex" => payload_hex = Some(value),
      "--url" => url = Some(value),
      "--cert-sha256" => cert_sha256 = Some(value),
      "--namespace" => namespace = Some(value),
      "--track" => track = Some(value),
      "--timeout-secs" => timeout_secs = value.parse().context("--timeout-secs")?,
      other => bail!("unknown flag {other}"),
    }
  }

  let mode = match mode.context("--mode is required")?.as_str() {
    "subscribe" => Mode::Subscribe,
    "publish" => Mode::Publish {
      payload: decode_hex(&payload_hex.context("--payload-hex is required to publish")?)?,
    },
    other => bail!("unknown --mode {other}"),
  };

  Ok(Args {
    mode,
    url: url.context("--url is required")?,
    cert_sha256: cert_sha256.context("--cert-sha256 is required")?,
    namespace: namespace.context("--namespace is required")?,
    track: track.context("--track is required")?,
    timeout_secs,
  })
}

fn decode_hex(input: &str) -> Result<Vec<u8>> {
  if input.len() % 2 != 0 {
    bail!("hex string has an odd length");
  }
  (0..input.len())
    .step_by(2)
    .map(|i| u8::from_str_radix(&input[i..i + 2], 16).context("invalid hex digit"))
    .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
  bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn connect(args: &Args) -> Result<Arc<TransportConnection>> {
  let digest: [u8; 32] = decode_hex(&args.cert_sha256)?
    .try_into()
    .map_err(|_| anyhow!("--cert-sha256 must be 32 bytes"))?;

  // WHY pin the hash: the relay runs on a self-signed certificate that no root
  // store trusts. Pinning keeps verification on, unlike `with_no_cert_validation`.
  let config = ClientConfig::builder()
    .with_bind_default()
    .with_server_certificate_hashes([Sha256Digest::new(digest)])
    .keep_alive_interval(Some(Duration::from_secs(3)))
    .build();

  let endpoint = Endpoint::client(config)?;
  let options = ConnectOptions::builder(&args.url)
    .add_header("wt-available-protocols", format!("\"{MOQT_VERSION}\""))
    .build();
  let connection = endpoint
    .connect(options)
    .await
    .context("WebTransport connect failed")?;
  println!("CONNECTED url={}", args.url);

  Ok(Arc::new(TransportConnection::WebTransport(connection)))
}

/// Exchanges SETUP on the control stream pair and returns the handler, which must
/// stay alive for the whole session: closing the control stream ends the session.
async fn setup(connection: &Arc<TransportConnection>) -> Result<ControlStreamHandler> {
  let mut send = connection.open_uni().await?;
  let client_setup = Setup::new(Vec::new())
    .serialize()
    .map_err(|e| anyhow!("failed to serialize SETUP: {e:?}"))?;
  send
    .write_all(&client_setup)
    .await
    .map_err(|e| anyhow!("failed to send SETUP: {e:?}"))?;

  let recv = connection.accept_uni().await?;
  let mut control = ControlStreamHandler::new(send, recv);
  let server_setup = control
    .read_setup()
    .await
    .map_err(|e| anyhow!("failed to receive SETUP: {e:?}"))?;
  server_setup
    .validate_incoming(SetupSender::Server, connection.kind())
    .map_err(|e| anyhow!("server SETUP carried a client-only option: {e:?}"))?;
  println!("SETUP_OK server_setup={server_setup:?}");

  Ok(control)
}

/// Sends SUBSCRIBE on its own request stream. The returned handler must stay
/// alive while objects are expected: dropping it cancels the subscription.
async fn subscribe(
  connection: &Arc<TransportConnection>,
  args: &Args,
) -> Result<(u64, ControlStreamHandler)> {
  let message = Subscribe::new_latest_object(
    0,
    Tuple::from_utf8_path(&args.namespace),
    TupleField::from_utf8(&args.track),
    vec![
      MessageParameter::new_subscriber_priority(128),
      MessageParameter::new_group_order(GroupOrder::Ascending),
      MessageParameter::new_forward(true),
    ],
  );

  let (send, recv) = connection.open_bi().await?;
  let mut request = ControlStreamHandler::new(send, recv);
  request
    .send(&ControlMessage::Subscribe(Box::new(message)))
    .await
    .map_err(|e| anyhow!("failed to send SUBSCRIBE: {e:?}"))?;

  match request.next_message().await {
    Ok(ControlMessage::SubscribeOk(ok)) => Ok((ok.track_alias, request)),
    Ok(other) => bail!("expected SUBSCRIBE_OK, got {other:?}"),
    Err(e) => bail!("failed waiting for SUBSCRIBE_OK: {e:?}"),
  }
}

async fn first_object(connection: &Arc<TransportConnection>, track_alias: u64) -> Result<()> {
  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));

  loop {
    let stream = connection
      .accept_uni()
      .await
      .map_err(|e| anyhow!("no data stream arrived: {e:?}"))?;
    let data = RecvDataStream::new(stream, pending_fetches.clone());

    let (_, object) = data.next_object().await;
    let Some(object) = object else {
      println!("STREAM_CLOSED_WITHOUT_OBJECT");
      continue;
    };
    if object.track_alias != track_alias {
      println!("IGNORED track_alias={}", object.track_alias);
      continue;
    }

    let payload = object.payload.as_deref().unwrap_or_default();
    println!(
      "OBJECT group={} object={} status={:?} payload_len={} payload_hex={}",
      object.location.group,
      object.location.object,
      object.status,
      payload.len(),
      encode_hex(payload),
    );
    return Ok(());
  }
}

/// Sends PUBLISH on its own request stream. The returned handler must stay alive
/// while the track is offered: dropping it withdraws the track.
async fn publish(
  connection: &Arc<TransportConnection>,
  args: &Args,
) -> Result<ControlStreamHandler> {
  let message = Publish::new(
    0,
    Tuple::from_utf8_path(&args.namespace),
    TupleField::from_utf8(&args.track),
    PUBLISH_TRACK_ALIAS,
    vec![MessageParameter::new_forward(true)],
    Vec::new(),
  );

  let (send, recv) = connection.open_bi().await?;
  let mut request = ControlStreamHandler::new(send, recv);
  request
    .send(&ControlMessage::Publish(Box::new(message)))
    .await
    .map_err(|e| anyhow!("failed to send PUBLISH: {e:?}"))?;

  match request.next_message().await {
    Ok(ControlMessage::RequestOk(_)) => Ok(request),
    Ok(other) => bail!("expected REQUEST_OK for PUBLISH, got {other:?}"),
    Err(e) => bail!("failed waiting for REQUEST_OK: {e:?}"),
  }
}

/// Sends one Object as group 0, object 0 on a fresh subgroup stream.
async fn send_object(connection: &Arc<TransportConnection>, payload: &[u8]) -> Result<()> {
  let stream = connection.open_uni().await?;
  // Header flags follow `apps/client/src/publisher.rs` at relay@0.15.0, the
  // combination the upstream test client is known to send to this relay.
  let header =
    SubgroupHeader::new_with_explicit_id(PUBLISH_TRACK_ALIAS, 0, 0, Some(0), true, true, true);
  let mut data = SendDataStream::new(
    Arc::new(Mutex::new(stream)),
    HeaderInfo::Subgroup { header },
  )
  .await?;

  let object = Object::try_from_subgroup(
    SubgroupObject {
      object_id: 0,
      properties: Some(Vec::new()),
      object_status: None,
      payload: Some(Bytes::copy_from_slice(payload)),
    },
    PUBLISH_TRACK_ALIAS,
    0,
    Some(0),
    Some(0),
    DEFAULT_PUBLISHER_PRIORITY,
  )?;
  data.send_object(&object, None).await?;
  data.finish().await?;
  println!("SENT group=0 object=0 payload_len={}", payload.len());
  Ok(())
}

async fn run(args: &Args) -> Result<()> {
  let connection = connect(args).await?;
  let _control = setup(&connection).await?;

  match &args.mode {
    Mode::Subscribe => {
      let (track_alias, _request) = subscribe(&connection, args).await?;
      println!("SUBSCRIBED track_alias={track_alias}");
      first_object(&connection, track_alias).await?;
    }
    Mode::Publish { payload } => {
      let _request = publish(&connection, args).await?;
      println!("PUBLISHED track_alias={PUBLISH_TRACK_ALIAS}");

      // The driver writes one line once the subscriber is ready, and closes stdin
      // once it has read the Object; the session has to stay open in between.
      let mut stdin = BufReader::new(tokio::io::stdin()).lines();
      if stdin.next_line().await?.is_none() {
        bail!("stdin closed before the send trigger");
      }
      send_object(&connection, payload).await?;
      while stdin.next_line().await?.is_some() {}
    }
  }

  connection.close(0u32, b"done");
  Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
  let args = parse_args()?;
  tokio::time::timeout(Duration::from_secs(args.timeout_secs), run(&args))
    .await
    .map_err(|_| anyhow!("timed out after {}s", args.timeout_secs))?
}
