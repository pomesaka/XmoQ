//! The lines of stdin, read on a thread of their own.

use std::io::BufRead;

use tokio::sync::mpsc;

/// The most bytes of a line of stdin. A longer one is not read, and is told
/// as too long. WHY a limit of its own, above the chat's 512 bytes: a line
/// is kept whole until its end comes, and the chat's limit is for what is
/// said, which is checked after.
pub const MOST_A_LINE: usize = 4096;

/// A line of stdin, or one that was longer than [`MOST_A_LINE`].
pub enum Line {
    /// The line, without its line break; bytes that are no UTF-8 replaced.
    Read(String),
    /// A line that was too long, of this many bytes.
    TooLong(usize),
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

/// The lines of stdin as they come. The channel closes at the end of
/// stdin, which is no reason to end: a reader run with `&` has none.
pub fn lines() -> mpsc::Receiver<Line> {
    let (sender, receiver) = mpsc::channel(64);
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
