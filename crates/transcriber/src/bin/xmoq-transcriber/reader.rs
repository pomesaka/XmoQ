//! The one reader of every track: a thread of its own with the model of
//! whisper.cpp, which reads the pieces in the order they were cut.
//!
//! WHY a thread and not a task: whisper.cpp holds the thread it reads on
//! for as long as it reads, a fifth of the piece's length and more
//! (`crates/speech/README.md`), and the session's loop must go on meanwhile.
//! WHY one for every track: the model is loaded once (550 MiB), and one
//! reader that keeps up is all the processor gives.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use xmoq_speech::cut::Piece;
use xmoq_speech::whisper::Whisper;
use xmoq_transcriber::transcript::Said;

/// What the reader has read and the loop has not taken yet; the reader
/// waits when it is full.
const READ_QUEUE: usize = 64;

/// A piece of a broadcast to be read.
pub struct ToRead {
    /// The prefix whose transcript it goes to, by its place in the flags.
    pub prefix: usize,
    /// The namespace of the broadcast, each field as UTF-8.
    pub from: Vec<String>,
    pub piece: Piece,
}

/// A piece that was read.
pub struct Read {
    pub prefix: usize,
    pub said: Said,
    /// How long it waited for the reader, and how long whisper.cpp took.
    pub waited_ms: u128,
    pub took_ms: u128,
}

/// The loop's side of the reader: where pieces are given.
pub struct Reader {
    pieces: SyncSender<(Instant, ToRead)>,
    dropped: u64,
}

impl Reader {
    /// Starts the thread, with at most `waiting` pieces waiting for it.
    /// What it reads comes on the receiver, in order.
    pub fn start(mut whisper: Whisper, waiting: usize) -> (Self, mpsc::Receiver<Read>) {
        let (pieces, to_read) = sync_channel(waiting);
        let (read_by, read) = mpsc::channel(READ_QUEUE);
        std::thread::spawn(move || run(&mut whisper, &to_read, &read_by));
        (Self { pieces, dropped: 0 }, read)
    }

    /// Gives a piece to the reader, or drops it where `waiting` pieces wait
    /// already: says how many were dropped so far, then.
    ///
    /// WHY dropped and not waited for: the loop reads the session, and a
    /// reader that falls behind would stop the reading of every track.
    /// WHY the new piece and not the oldest that waits: the waiting pieces
    /// are in the reader's queue, which takes nothing back.
    pub fn give(&mut self, piece: ToRead) -> Option<u64> {
        match self.pieces.try_send((Instant::now(), piece)) {
            Ok(()) => None,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped += 1;
                Some(self.dropped)
            }
        }
    }
}

/// The transcriber's clock since the epoch, in milliseconds.
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

fn run(whisper: &mut Whisper, to_read: &Receiver<(Instant, ToRead)>, read_by: &mpsc::Sender<Read>) {
    while let Ok((
        given,
        ToRead {
            prefix,
            from,
            piece,
        },
    )) = to_read.recv()
    {
        let began = Instant::now();
        let text = whisper.text(&piece.samples);
        let took_ms = began.elapsed().as_millis();
        let waited_ms = began.duration_since(given).as_millis();
        let text = match text {
            Ok(text) if text.is_empty() => {
                println!(
                    "PIECE_WITHOUT_WORDS from={} start_us={} took_ms={took_ms}",
                    from.join("/"),
                    piece.start_us
                );
                continue;
            }
            Ok(text) => text,
            Err(error) => {
                println!("PIECE_NOT_READ from={} error={error:#}", from.join("/"));
                continue;
            }
        };
        let said = Said {
            from,
            text,
            start_us: piece.start_us,
            end_us: piece.end_us,
            at: wall_ms(),
        };
        let read = Read {
            prefix,
            said,
            waited_ms,
            took_ms,
        };
        // The loop is gone: the transcriber is stopping.
        if read_by.blocking_send(read).is_err() {
            return;
        }
    }
}
