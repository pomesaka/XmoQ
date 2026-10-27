//! The readers of every track: threads, each with a reader of `--engine`
//! (`xmoq_speech::engine`), which read the pieces in the order they were
//! cut.
//!
//! WHY a thread and not a task: a reader holds the thread it reads on for
//! as long as it reads (whisper.cpp a twentieth of the piece's length and
//! more, `crates/speech/src/lib.rs`), and the session's loop must go on
//! meanwhile. WHY readers for every track and not one for each: whisper.cpp
//! loads its model once, and each reader is a state of its own over it
//! (`--readers`, 1 by default).

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use xmoq_speech::Words;
use xmoq_speech::cut::Piece;
use xmoq_transcriber::marks::only_marks;
use xmoq_transcriber::transcript::ReadPiece;

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
    pub said: ReadPiece,
    /// How long it waited for the reader, and how long the reader took.
    pub waited_ms: u128,
    pub took_ms: u128,
}

/// The loop's side of the reader: where pieces are given.
pub struct ReaderPool {
    pieces: SyncSender<(Instant, ToRead)>,
    dropped: u64,
}

impl ReaderPool {
    /// Starts a thread for each of `readers`, with at most `waiting`
    /// pieces waiting for them. Each thread takes the next piece that
    /// waits. What they read comes on the receiver, in the order each was
    /// done: with more than one thread, a later piece can be done before
    /// an earlier one, of the same voice as well.
    pub fn start(
        readers: Vec<Box<dyn Words + Send>>,
        waiting: usize,
    ) -> (Self, mpsc::Receiver<Read>) {
        let (pieces, to_read) = sync_channel(waiting);
        let to_read = Arc::new(Mutex::new(to_read));
        let (read_by, read) = mpsc::channel(READ_QUEUE);
        for mut words in readers {
            let (to_read, read_by) = (Arc::clone(&to_read), read_by.clone());
            std::thread::spawn(move || run(words.as_mut(), &to_read, &read_by));
        }
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

/// The next piece that waits; `None` once the loop is gone.
fn next(to_read: &Mutex<Receiver<(Instant, ToRead)>>) -> Option<(Instant, ToRead)> {
    // A thread that panicked while it waited held nothing else.
    let to_read = to_read.lock().unwrap_or_else(PoisonError::into_inner);
    to_read.recv().ok()
}

fn run(
    words: &mut dyn Words,
    to_read: &Mutex<Receiver<(Instant, ToRead)>>,
    read_by: &mpsc::Sender<Read>,
) {
    while let Some((
        given,
        ToRead {
            prefix,
            from,
            piece,
        },
    )) = next(to_read)
    {
        let began = Instant::now();
        let text = words.text(&piece.samples);
        let took_ms = began.elapsed().as_millis();
        let waited_ms = began.duration_since(given).as_millis();
        let text = match text {
            Ok(text) if only_marks(&text) => {
                println!(
                    "PIECE_WITHOUT_WORDS from={} start_us={} took_ms={took_ms} text={text:?}",
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
        let said = ReadPiece {
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
