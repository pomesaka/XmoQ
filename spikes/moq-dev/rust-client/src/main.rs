//! Phase 0 spike: publish or receive exactly one MoQ object through a relay.
//!
//! Shaped after `examples/clock.rs` in moq-tokio 0.19.23, so the connection flags
//! (`--connect`, `--connect-version`, ...) are the library's own.

use anyhow::Context;
use moq_net::*;

#[derive(usage::Cli, Clone)]
#[usage(unknown_flags = "error", args_override_self = false)]
#[usage(name = "xmoq-spike-moq-dev")]
#[usage(settings)]
struct Config {
	/// The broadcast (namespace) to publish or subscribe to.
	#[usage(long)]
	broadcast: String,

	/// The track name inside the broadcast.
	#[usage(long, default = "object")]
	track: String,

	/// Publish: the object payload. Subscribe: the payload that must arrive.
	#[usage(long)]
	payload: String,

	/// Give up after this many seconds.
	#[usage(long, default = "20")]
	wait: u64,

	#[usage(flatten)]
	client: moq_tokio::connect::Config,

	#[usage(flatten)]
	log: moq_tokio::Log,

	#[usage(subcommand)]
	role: Command,
}

#[derive(usage::Subcommands, Clone)]
enum Command {
	/// Announce the broadcast, write one group with one frame, then stay up until `--wait` elapses.
	Publish,
	/// Wait for the broadcast, read the first frame of the first group, compare it and exit.
	Subscribe,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let config = Config::parse();
	config.log.init()?;

	let url = config.client.url.clone().context("--connect is required")?;
	let client = config.client.init(Default::default())?;
	let origin = moq_tokio::origin::spawn();
	let wait = std::time::Duration::from_secs(config.wait);

	match config.role {
		Command::Publish => {
			let broadcast = origin
				.create_broadcast(&config.broadcast)
				.context("failed to create broadcast")?;
			let track = broadcast.create_track(config.track, None)?;
			broadcast
				.announce(Default::default())
				.context("failed to announce broadcast")?;

			let mut group = track.create_group(0u32.into()).context("failed to create group")?;
			group.write_frame(Timestamp::now(), config.payload.clone())?;
			group.finish()?;
			println!("PUBLISHED payload={:?}", config.payload);

			let connection = client.with_publisher(&origin).connect(url);
			let result = tokio::select! {
				res = connection.closed() => res.map_err(Into::into),
				_ = tokio::time::sleep(wait) => Ok(()),
			};
			broadcast.close();
			result
		}
		Command::Subscribe => {
			let connection = client.with_subscriber(origin.clone()).connect(url);

			let path: Path<'_> = config.broadcast.into();
			let scope = Patterns::from(Pattern::subtree(path.as_str()).context("invalid broadcast name")?);
			let consumer = origin
				.scope("", &scope)
				.context("not allowed to consume broadcast")?
				.consume();
			let mut announced = consumer.announced();

			let receive = async {
				loop {
					let update = announced.next().await.context("announcements ended")?;
					if !update.kind.is_active() {
						continue;
					}
					println!("ANNOUNCED broadcast={}", update.prefix);
					let broadcast = consumer.request_broadcast(&update.prefix).await?;
					let mut track = broadcast.track(&config.track)?.subscribe(None).await?;
					let mut group = track.recv_group().await?.context("track ended without a group")?;
					let frame = group.read_frame().await?.context("group ended without a frame")?;
					return anyhow::Ok(String::from_utf8_lossy(&frame.payload).into_owned());
				}
			};

			let received = tokio::select! {
				res = receive => res?,
				res = connection.closed() => {
					res?;
					anyhow::bail!("connection closed before an object arrived");
				}
				_ = tokio::time::sleep(wait) => anyhow::bail!("timed out after {}s", wait.as_secs()),
			};

			println!("RECEIVED payload={received:?}");
			anyhow::ensure!(
				received == config.payload,
				"payload mismatch: expected {:?}, got {received:?}",
				config.payload
			);
			println!("MATCH");
			Ok(())
		}
	}
}
