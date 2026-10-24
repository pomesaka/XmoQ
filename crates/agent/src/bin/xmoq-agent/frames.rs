//! The pictures of the broadcast: `xmoq_picture::frames::Frames` on a thread
//! of its own, which writes the PNG files and says what it wrote.
//!
//! WHY a thread: decoding a Group's frames and writing a PNG take a moment
//! of a core (a Group of 1080 lines, 4 ms a frame), and the session's loop
//! reads the chat meanwhile.

use std::path::PathBuf;

use tokio::sync::mpsc;
use xmoq_agent::events::{Asked, Event};
use xmoq_picture::frames::{Frames, Settings, Taken, VideoObject};

/// The jobs that may wait for the thread. **Provisional.** Beyond them an
/// Object of the video is left out (and the rest of its Group with it, which
/// `Frames` can no longer decode), and counted.
pub const MOST_WAITING: usize = 256;

/// What the thread is given.
pub enum Job {
    /// An Object of the video track, in the order it came.
    Object {
        group: u64,
        object: u64,
        timestamp_us: u64,
        annex_b: Vec<u8>,
    },
    /// `/frame`.
    Ask,
    /// The picture asked for came not in time.
    Forgo,
}

/// The thread of one video track.
pub struct Pictures {
    jobs: mpsc::Sender<Job>,
    /// What the thread wrote, and what failed.
    pub told: mpsc::Receiver<Event>,
    /// Objects left out because the thread was behind.
    pub left_out: u64,
}

/// Writes `taken` into `dir` under `prefix`, and removes what it says to.
fn write(dir: &std::path::Path, prefix: &str, taken: &Taken, asked: Asked) -> Event {
    let path = dir.join(format!("{prefix}{}", taken.file_name));
    if let Err(error) = std::fs::write(&path, &taken.png) {
        return Event::Error {
            message: format!("frame: {}: {error}", path.display()),
        };
    }
    for removed in &taken.removed {
        // WHY nothing is said where it fails: the agent may have moved or
        // removed the file itself.
        let _ = std::fs::remove_file(dir.join(format!("{prefix}{removed}")));
    }
    Event::Frame {
        path: path.display().to_string(),
        group: taken.of.group,
        object: taken.of.object,
        timestamp_us: taken.of.timestamp_us,
        width: taken.width,
        height: taken.height,
        asked,
    }
}

impl Pictures {
    /// Starts the thread, which writes into `dir` (made where it is not)
    /// with `prefix` before each file's name.
    ///
    /// WHY a prefix of the session: a `Frames` is of one track, and names
    /// its files from 0; a new session begins a new one.
    pub fn start(settings: Settings, dir: PathBuf, prefix: String) -> anyhow::Result<Self> {
        let mut frames = Frames::new(settings)?;
        std::fs::create_dir_all(&dir)?;
        let (jobs, mut waiting) = mpsc::channel::<Job>(MOST_WAITING);
        let (tell, told) = mpsc::channel(64);
        std::thread::spawn(move || {
            while let Some(job) = waiting.blocking_recv() {
                let was_asked = frames.is_asked();
                let (taken, asked) = match job {
                    Job::Object {
                        group,
                        object,
                        timestamp_us,
                        annex_b,
                    } => (
                        frames.take(VideoObject {
                            group,
                            object,
                            timestamp_us,
                            annex_b: &annex_b,
                        }),
                        if was_asked {
                            Asked::Command
                        } else {
                            Asked::Every
                        },
                    ),
                    Job::Ask => (frames.ask(), Asked::Command),
                    Job::Forgo => {
                        frames.forgo();
                        (None, Asked::Command)
                    }
                };
                if let Some(taken) = taken
                    && tell
                        .blocking_send(write(&dir, &prefix, &taken, asked))
                        .is_err()
                {
                    break;
                }
            }
            let counts = frames.counts();
            eprintln!(
                "XMOQ_AGENT_FRAMES given={} left_out={} decoded={} no_picture={} too_large={} passed_over={} taken={} not_written={}",
                counts.given,
                counts.left_out,
                counts.decoded,
                counts.no_picture,
                counts.too_large,
                counts.passed_over,
                counts.taken,
                counts.not_written
            );
        });
        Ok(Self {
            jobs,
            told,
            left_out: 0,
        })
    }

    /// Hands an Object to the thread. `false` where it was left out, the
    /// thread being behind.
    pub fn object(&mut self, job: Job) -> bool {
        let given = self.jobs.try_send(job).is_ok();
        if !given {
            self.left_out += 1;
        }
        given
    }

    /// Asks for a picture, or gives the asking up.
    ///
    /// WHY waited for where the queue is full, and not left out as an
    /// Object is: the agent asked, and the wait is for one job of the
    /// thread to be done.
    pub async fn tell(&self, job: Job) {
        let _ = self.jobs.send(job).await;
    }
}
