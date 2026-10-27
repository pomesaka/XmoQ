//! What is new in `inbox.txt`, which an agent adds lines to and the reader
//! reads again and again. The page of a meeting reads its own the same way
//! (`newInboxLines` of `@xmoq/meeting`); both read
//! `fixtures/agent-lines/inbox-reads.tsv`.

use crate::input::MOST_A_LINE;

/// The name of the file in the folder of a reader.
pub const INBOX_FILE: &str = "inbox.txt";

/// How many first bytes of the file are kept to tell a file written anew
/// from one added to.
pub const HEAD_BYTES: usize = 64;

/// What is new in the file since the read before.
#[derive(Debug, PartialEq, Eq)]
pub struct InboxRead {
    /// The whole lines after the offset; a line over [`MOST_A_LINE`] bytes
    /// is `Err` with its length.
    pub lines: Vec<Result<String, usize>>,
    /// Where the next read begins: after the last line feed.
    pub offset: usize,
    /// The first bytes of the file as it was read ([`HEAD_BYTES`] at most),
    /// to tell a file written anew at the next read.
    pub head: Vec<u8>,
}

/// The whole lines of `bytes` (the file as it is now) after `offset`. A last
/// line without its line feed is left for the next read: the agent may be
/// writing it. **A file shorter than `offset`, or one that does not begin as
/// `head` did, was written anew** (`>` in place of `>>`), and is read from
/// its beginning. WHY by bytes: a count of characters would split a
/// character that a read cut.
#[must_use]
pub fn new_lines(bytes: &[u8], offset: usize, head: &[u8]) -> InboxRead {
    let same = bytes.starts_with(head);
    let from = if bytes.len() < offset || !same {
        0
    } else {
        offset
    };
    let mut lines = Vec::new();
    let mut start = from;
    for (at, byte) in bytes.iter().enumerate().skip(from) {
        if *byte != b'\n' {
            continue;
        }
        let line = bytes.get(start..at).unwrap_or_default();
        lines.push(if line.len() > MOST_A_LINE {
            Err(line.len())
        } else {
            Ok(String::from_utf8_lossy(line).into_owned())
        });
        start = at + 1;
    }
    InboxRead {
        lines,
        offset: start,
        head: bytes
            .get(..HEAD_BYTES.min(start))
            .unwrap_or_default()
            .to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `fixtures/agent-lines/inbox-reads.tsv`, which
    /// `@xmoq/meeting` reads too: the file, the offset and the head of the
    /// read before, the lines read (`null` for one too long), and the next
    /// offset.
    #[test]
    fn what_is_new_in_the_inbox_is_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/agent-lines/inbox-reads.tsv");
        let mut rows = 0;
        for row in table.lines() {
            let [name, file, offset, head, lines, next] = row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("{row}");
            };
            let file: String = serde_json::from_str(file).unwrap();
            let head: String = serde_json::from_str(head).unwrap();
            let lines: Vec<Option<String>> = serde_json::from_str(lines).unwrap();
            let read = new_lines(file.as_bytes(), offset.parse().unwrap(), head.as_bytes());
            let read_lines: Vec<Option<String>> = read.lines.into_iter().map(Result::ok).collect();
            assert_eq!(
                (read_lines, read.offset),
                (lines, next.parse().unwrap()),
                "{name}"
            );
            rows += 1;
        }
        assert!(rows > 0);
    }
}
