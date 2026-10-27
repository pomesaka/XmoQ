//! The names of the files of `events/`: each holds the lines of one write,
//! and is not changed after it is put there. The page of a meeting names
//! its files by the same rule (`@xmoq/meeting`); both read
//! `fixtures/agent-lines/event-files.tsv`.
//!
//! WHY a file a write and not one file that grows: the page can only write
//! a file anew (Chromium copies it and moves the copy over it), so a reader
//! that follows one file by its name (`tail -F`) is given all of it again
//! at each write; and each write would copy all of it. The reader goes by
//! the names, which go up with the time.

/// The directory of the files of events, in the folder of a reader.
pub const EVENTS_DIR: &str = "events";

/// The digits of a name, at the least: the Unix time in milliseconds has
/// 13 until the year 2286, so that the names sort as their numbers.
const DIGITS: usize = 13;

/// The number of a file of `events/` by its name, `<13 or more digits>.jsonl`;
/// `None` for any other name (a file being written, or another's).
#[must_use]
pub fn number_of(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(".jsonl")?;
    if digits.len() < DIGITS || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The number of the next file: the clock `now` (ms since the Unix epoch),
/// or one after `newest`, the number of the newest file there, where that
/// is not before it.
///
/// WHY not the clock alone: two writes in the same millisecond, a clock
/// that went back, or a writer started again in the same folder would put a
/// file before one a reader already read, and a reader that reads the
/// names after the last it read would never read it.
#[must_use]
pub fn next_number(now: u64, newest: Option<u64>) -> u64 {
    newest.map_or(now, |newest| now.max(newest.saturating_add(1)))
}

/// The name of the file of `number`.
#[must_use]
pub fn name_of(number: u64) -> String {
    format!("{number:0DIGITS$}.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `fixtures/agent-lines/event-files.tsv`, which
    /// `@xmoq/meeting` reads too: the names there before, the clock, and
    /// the name of the file written.
    #[test]
    fn the_next_file_is_named_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/agent-lines/event-files.tsv");
        let mut rows = 0;
        for row in table.lines() {
            let [name, before, now, written] = row.split('\t').collect::<Vec<_>>()[..] else {
                panic!("{row}");
            };
            let newest = before
                .split(',')
                .filter(|file| *file != "-")
                .filter_map(number_of)
                .max();
            let now: u64 = now.parse().unwrap();
            assert_eq!(name_of(next_number(now, newest)), written, "{name}");
            rows += 1;
        }
        assert!(rows > 0);
    }
}
