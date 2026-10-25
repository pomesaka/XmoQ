//! The pictures of the broadcast: `xmoq_picture::frames::Frames` on a thread
//! of its own, which writes the PNG files and says what it wrote.
//!
//! WHY a thread: decoding a Group's frames and writing a PNG take a moment
//! of a core (a Group of 1080 lines, 4 ms a frame), and the session's loop
//! reads the chat meanwhile. WHY one for the whole run, and not one a
//! session: `--frames-kept` bounds the files of every session together,
//! which one `Frames` keeps the names of (`Frames::begin_again`).

use std::path::PathBuf;

use tokio::sync::{mpsc, oneshot};
use xmoq_agent::events::{Asked, Event};
use xmoq_picture::frames::{Frames, Settings, Taken, VideoObject};

/// The jobs that may wait for the thread. **Provisional.** Beyond them an
/// Object of the video is left out (and the rest of its Group with it, which
/// `Frames` can no longer decode), and counted.
pub const MOST_WAITING: usize = 256;

/// What the thread is given.
pub enum Job {
    /// The video of a new session follows: another track, or the same one
    /// numbered anew.
    BeginAgain,
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

/// The thread of the pictures, and what the session's loop keeps of them.
pub struct Pictures {
    jobs: mpsc::Sender<Job>,
    /// What the thread wrote, and what failed.
    pub told: mpsc::Receiver<Event>,
    /// Objects left out because the thread was behind.
    pub left_out: u64,
    /// Whether Objects are being left out, which is told once each time it
    /// begins.
    pub leaving_out: bool,
    /// Told when the thread has said its counts and ended.
    ended: oneshot::Receiver<()>,
}

/// How long the thread gets to end when the reader ends: what is queued is
/// cheap to take (a picture is decoded only when asked for), and a picture
/// being written takes a moment. **Provisional.** WHY waited for at all:
/// the process ends when `main` returns, and the thread's last line
/// (`XMOQ_AGENT_FRAMES`) was lost with it now and then (CI of #339).
const END_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Writes `taken` into `dir`, and removes what it says to.
fn write(dir: &std::path::Path, taken: &Taken, asked: Asked) -> Event {
    let path = dir.join(&taken.file_name);
    if let Err(error) = std::fs::write(&path, &taken.png) {
        return Event::Error {
            message: format!("frame: {}: {error}", path.display()),
        };
    }
    for removed in &taken.removed {
        // WHY nothing is said where it fails: the agent may have moved or
        // removed the file itself.
        let _ = std::fs::remove_file(dir.join(removed));
    }
    Event::Frame {
        path: path.display().to_string(),
        group: taken.of.group,
        object: taken.of.object,
        timestamp_us: taken.of.timestamp_us,
        width: taken.width,
        height: taken.height,
        asked,
        of: None,
    }
}

impl Pictures {
    /// Starts the thread, which writes into `dir` (made where it is not).
    pub fn start(settings: Settings, dir: PathBuf) -> anyhow::Result<Self> {
        let mut frames = Frames::new(settings)?;
        std::fs::create_dir_all(&dir)?;
        let (jobs, mut waiting) = mpsc::channel::<Job>(MOST_WAITING);
        let (tell, told) = mpsc::channel(64);
        let (end, ended) = oneshot::channel();
        std::thread::spawn(move || {
            while let Some(job) = waiting.blocking_recv() {
                let was_asked = frames.is_asked();
                let told = match job {
                    Job::BeginAgain => frames.begin_again().err().map(|error| Event::Error {
                        message: format!("frame: {error}"),
                    }),
                    Job::Object {
                        group,
                        object,
                        timestamp_us,
                        annex_b,
                    } => frames
                        .take(VideoObject {
                            group,
                            object,
                            timestamp_us,
                            annex_b: &annex_b,
                        })
                        .map(|taken| {
                            let asked = if was_asked {
                                Asked::Command
                            } else {
                                Asked::Every
                            };
                            write(&dir, &taken, asked)
                        }),
                    Job::Ask => frames
                        .ask()
                        .map(|taken| write(&dir, &taken, Asked::Command)),
                    Job::Forgo => {
                        frames.forgo();
                        None
                    }
                };
                if let Some(event) = told
                    && tell.blocking_send(event).is_err()
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
            let _ = end.send(());
        });
        Ok(Self {
            jobs,
            told,
            left_out: 0,
            leaving_out: false,
            ended,
        })
    }

    /// Ends the thread and waits for it, [`END_WAIT`] at the most: what is
    /// queued is taken, and its counts are said.
    pub async fn end(self) {
        let Self {
            jobs, told, ended, ..
        } = self;
        // Both, so that a thread waiting to tell gives up as well.
        drop(told);
        drop(jobs);
        let _ = tokio::time::timeout(END_WAIT, ended).await;
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

    /// Asks for a picture, gives the asking up, or begins a session's video.
    ///
    /// WHY waited for where the queue is full, and not left out as an
    /// Object is: the agent asked, or the video that follows is decoded
    /// wrongly without it; the wait is for one job of the thread to be done.
    pub async fn tell(&self, job: Job) {
        let _ = self.jobs.send(job).await;
    }
}
