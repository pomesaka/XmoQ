//! The lines of stdin and of `<DIR>/inbox.txt`, each read on a thread of
//! its own, in one channel.

use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;
use xmoq_agent::inbox::new_lines;
use xmoq_agent::input::MOST_A_LINE;

/// How often `inbox.txt` is read again. **Provisional.** WHY read again,
/// and not told of a change: the page of a meeting cannot be told
/// (`INBOX_EVERY_MS` of `@xmoq/meeting`), and an agent writes to either in
/// the same way; a second is as long as a person waits for a line of the
/// chat.
const INBOX_EVERY: Duration = Duration::from_secs(1);

/// A line of stdin or of `inbox.txt`, or one that was longer than
/// [`MOST_A_LINE`].
pub enum Line {
    /// The line, without its line break; bytes that are no UTF-8 replaced.
    Read(String),
    /// A line that was too long, of this many bytes.
    TooLong(usize),
    /// `inbox.txt` could not be read, and why; told once until it is read
    /// again.
    InboxFailed(String),
}

/// Reads the next line of `reader`, keeping no more than `most` bytes of it.
/// `None` at the end.
fn next_line(reader: &mut impl BufRead, most: usize) -> std::io::Result<Option<Line>> {
    let mut kept = Vec::new();
    let mut length = 0;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok((length > 0).then(|| ended(&kept, length, most)));
        }
        let (taken, found) = match buffer.iter().position(|byte| *byte == b'\n') {
            Some(at) => (at, true),
            None => (buffer.len(), false),
        };
        if length < most {
            let room = most.saturating_sub(length).min(taken);
            kept.extend_from_slice(buffer.get(..room).unwrap_or_default());
        }
        length += taken;
        reader.consume(taken + usize::from(found));
        if found {
            return Ok(Some(ended(&kept, length, most)));
        }
    }
}

fn ended(kept: &[u8], length: usize, most: usize) -> Line {
    if length > most {
        Line::TooLong(length)
    } else {
        Line::Read(String::from_utf8_lossy(kept).into_owned())
    }
}

/// The lines of stdin and of `inbox` as they come. The end of stdin is no
/// reason to end: a reader run with `&` has none.
pub fn lines(inbox: PathBuf) -> mpsc::Receiver<Line> {
    let (sender, receiver) = mpsc::channel(64);
    let from_inbox = sender.clone();
    std::thread::spawn(move || read_inbox(&inbox, &from_inbox));
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        while let Ok(Some(line)) = next_line(&mut reader, MOST_A_LINE) {
            if sender.blocking_send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// Reads what is added to `inbox` every [`INBOX_EVERY`], until the
/// receiver is gone. The file is made where it is not.
///
/// WHY what is in it when the reader begins is passed over: it was written
/// for a reader before, which did it or did not.
fn read_inbox(inbox: &std::path::Path, lines: &mpsc::Sender<Line>) {
    let made = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(inbox);
    let mut failing = made
        .err()
        .map(|error| format!("{}: {error}", inbox.display()));
    if let Some(why) = &failing
        && lines.blocking_send(Line::InboxFailed(why.clone())).is_err()
    {
        return;
    }
    let first = new_lines(&std::fs::read(inbox).unwrap_or_default(), 0, &[]);
    let (mut offset, mut head) = (first.offset, first.head);
    loop {
        std::thread::sleep(INBOX_EVERY);
        let read = match std::fs::read(inbox) {
            Ok(bytes) => {
                failing = None;
                new_lines(&bytes, offset, &head)
            }
            Err(error) => {
                let why = format!("{}: {error}", inbox.display());
                // WHY once: a file that was taken away fails every second,
                // and the lines an agent reads would be these alone.
                if failing.as_ref() != Some(&why) {
                    failing = Some(why.clone());
                    if lines.blocking_send(Line::InboxFailed(why)).is_err() {
                        return;
                    }
                }
                continue;
            }
        };
        offset = read.offset;
        head = read.head;
        for line in read.lines {
            let line = match line {
                Ok(text) => Line::Read(text),
                Err(length) => Line::TooLong(length),
            };
            if lines.blocking_send(line).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line read, or the length of one that was too long.
    type Read = Result<String, usize>;

    fn read_all(input: &[u8], most: usize) -> Vec<Read> {
        // A small buffer, so that a line comes in several pieces.
        let mut reader = std::io::BufReader::with_capacity(3, input);
        let mut lines = Vec::new();
        while let Some(line) = next_line(&mut reader, most).unwrap() {
            lines.push(match line {
                Line::Read(text) => Ok(text),
                Line::TooLong(length) => Err(length),
                Line::InboxFailed(why) => panic!("{why}"),
            });
        }
        lines
    }

    #[test]
    fn the_lines_of_stdin_are_read_up_to_a_limit() {
        // (what stdin has; the lines read, with at most 5 bytes a line)
        let cases: [(&[u8], Vec<Read>); 6] = [
            (
                b"ab\ncd\n",
                vec![Ok("ab".to_string()), Ok("cd".to_string())],
            ),
            // The last line without its line break.
            (b"ab\ncd", vec![Ok("ab".to_string()), Ok("cd".to_string())]),
            (b"\n\n", vec![Ok(String::new()), Ok(String::new())]),
            // As long as the limit, and one byte more; the next line is
            // read after it.
            (
                b"12345\n123456\nok\n",
                vec![Ok("12345".to_string()), Err(6), Ok("ok".to_string())],
            ),
            (b"1234567890", vec![Err(10)]),
            (b"a\xffb\n", vec![Ok("a\u{fffd}b".to_string())]),
        ];
        for (input, lines) in cases {
            assert_eq!(read_all(input, 5), lines, "{input:?}");
        }
    }
}
