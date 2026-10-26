//! The history of the chat, kept for good (decided on 2026-10-24) in
//! one SQLite file. The only file of the gatherer that touches `rusqlite`,
//! so that the store can be replaced.
//!
//! WHY SQLite (the user's decision, 2026-10-24): a FETCH asks for a
//! range of a room's lines by their numbers, which an index answers
//! without reading the history before it, and a line is in the file once
//! its transaction is over, also when the process is killed right after.
//! The price, which was accepted: SQLite is C, built by `rusqlite`'s
//! `bundled` feature, so the build needs a C compiler, here and in CI.
//! WHY NOT JSON Lines, which was rejected: a range is read by going through
//! the file from its beginning, or by an index of one's own beside it, and
//! a line cut off by a crash in the middle of a write is the reader's to
//! find and mend.
//!
//! **Provisional, not decided**: the schema.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use xmoq_chat_tracks::payloads::Line;

/// The version of the schema, in `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;

/// The history.
pub struct Store {
    connection: Connection,
}

/// What a store that another connection holds is refused with.
fn in_use(path: &Path) -> String {
    format!(
        "the store {} is in use, by another gatherer perhaps: each runs with a file of its own",
        path.display()
    )
}

/// A number of the gatherer as SQLite keeps it.
fn integer(value: u64) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{value} is too large for the store"))
}

/// A number SQLite gave back. The column's `CHECK` keeps it from being
/// below 0; a file written by something else could have one.
fn number(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}

impl Store {
    /// Opens the file, making it and its table where they are not there.
    ///
    /// WHY `journal_mode = WAL`: a FETCH reads while lines are written.
    ///
    /// WHY `locking_mode = EXCLUSIVE`, set before WAL, and a write at once:
    /// **one gatherer to a file.** Gatherers of different rooms (one a room)
    /// each run with a store of their own. SQLite, read on 2026-10-24:
    /// "When the locking-mode is set to EXCLUSIVE, the database connection
    /// never releases file-locks ... The first time the database is written,
    /// an exclusive lock is obtained and held"
    /// (<https://www.sqlite.org/pragma.html#pragma_locking_mode>); "If
    /// another database connection has the database mode open in exclusive
    /// locking mode then all queries against the database will return
    /// `SQLITE_BUSY`" (<https://www.sqlite.org/wal.html>, §9). So a second
    /// gatherer on the same file fails when it opens it, and says so. WHY
    /// NOT one file for several gatherers of other rooms, which WAL would
    /// allow on one machine ("there can only be one writer at a time", §2.2;
    /// "WAL does not work over a network filesystem", §1): each would have
    /// to wait for the others' writes (`busy_timeout`), and nothing would
    /// keep two of them from holding the same room, whose numbers would
    /// then clash.
    ///
    /// WHY `synchronous = FULL`: a line is published once [`Self::keep`]
    /// returns, so a line that a power loss takes back has been seen, and
    /// its number would be given again to another line when the gatherer
    /// starts. The SQLite documentation, read on 2026-10-24
    /// (<https://www.sqlite.org/pragma.html#pragma_synchronous>): FULL "is
    /// atomic, consistent, isolated, and durable (ACID) in WAL mode"; with
    /// NORMAL "WAL mode does lose durability. A transaction committed in WAL
    /// mode with synchronous=NORMAL might roll back following a power loss
    /// or system crash"; and (<https://www.sqlite.org/wal.html>, §2.3)
    /// "Writers sync the WAL on every transaction commit if PRAGMA
    /// synchronous is set to FULL but omit this sync if PRAGMA synchronous
    /// is set to NORMAL". WHY NOT NORMAL, which the review of the first
    /// version found: the history is kept for good. The price is a
    /// sync of the disk for every line taken, which was not measured.
    pub fn open(path: &Path) -> Result<Self> {
        let connection =
            Connection::open(path).with_context(|| format!("the store {}", path.display()))?;
        // WHY no wait for a lock: the one who holds it is another gatherer,
        // which keeps it for as long as it runs.
        connection.busy_timeout(std::time::Duration::ZERO)?;
        connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .with_context(|| in_use(path))?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                // The numbers count from 0 by one, far below the 2^63 a
                // signed INTEGER holds; `integer` refuses what does not fit.
                connection.execute_batch(
                    "CREATE TABLE IF NOT EXISTS lines (
                        room TEXT NOT NULL,
                        number INTEGER NOT NULL CHECK (number >= 0),
                        name TEXT NOT NULL,
                        text TEXT NOT NULL,
                        sent_at INTEGER NOT NULL CHECK (sent_at >= 0),
                        at INTEGER NOT NULL CHECK (at >= 0),
                        PRIMARY KEY (room, number)
                    ) WITHOUT ROWID;",
                )?;
                connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            SCHEMA_VERSION => {}
            other => bail!(
                "the store {} has the schema {other}, which this gatherer does not know",
                path.display()
            ),
        }
        // A write, which takes the lock and keeps it.
        connection
            .execute_batch("BEGIN IMMEDIATE; COMMIT;")
            .with_context(|| in_use(path))?;
        Ok(Self { connection })
    }

    /// The rooms of the history with the number their next line gets, the
    /// room with the latest line first, `most` of them at the most.
    pub fn rooms(&self, most: usize) -> Result<Vec<(String, u64)>> {
        let mut statement = self.connection.prepare(
            "SELECT room, MAX(number) FROM lines GROUP BY room ORDER BY MAX(at) DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![integer(u64::try_from(most)?)?], |row| {
            Ok((row.get::<_, String>(0)?, number(row.get(1)?)?))
        })?;
        let mut rooms = Vec::new();
        for row in rows {
            let (room, largest) = row?;
            rooms.push((room, largest.saturating_add(1)));
        }
        Ok(rooms)
    }

    /// The number the next line of the room gets: one after the largest it
    /// holds, 0 for a room it holds nothing of.
    pub fn next_number(&self, room: &str) -> Result<u64> {
        let largest: Option<i64> = self
            .connection
            .query_row(
                "SELECT MAX(number) FROM lines WHERE room = ?1",
                params![room],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(match largest {
            Some(largest) => number(largest)?.saturating_add(1),
            None => 0,
        })
    }

    /// Keeps a line of the room. It is in the file when this returns.
    pub fn keep(&self, room: &str, line: &Line) -> Result<()> {
        self.connection
            .prepare_cached(
                "INSERT INTO lines (room, number, name, text, sent_at, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                room,
                integer(line.number)?,
                line.name,
                line.text,
                integer(line.sent_at)?,
                integer(line.at)?,
            ])?;
        Ok(())
    }

    /// The lines of the room numbered `first` to `last`, in their order;
    /// `None` where their payloads on `lines` come to more than
    /// `most_bytes`. The lines are read one by one, and the reading stops
    /// there.
    pub fn lines(
        &self,
        room: &str,
        (first, last): (u64, u64),
        most_bytes: usize,
    ) -> Result<Option<Vec<Line>>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT number, name, text, sent_at, at FROM lines
             WHERE room = ?1 AND number BETWEEN ?2 AND ?3 ORDER BY number",
        )?;
        let rows = statement.query_map(params![room, integer(first)?, integer(last)?], |row| {
            Ok(Line {
                number: number(row.get(0)?)?,
                name: row.get(1)?,
                text: row.get(2)?,
                sent_at: number(row.get(3)?)?,
                at: number(row.get(4)?)?,
            })
        })?;
        let (mut lines, mut bytes) = (Vec::new(), 0_usize);
        for row in rows {
            let line = row?;
            bytes = bytes.saturating_add(line.payload().len());
            if bytes > most_bytes {
                return Ok(None);
            }
            lines.push(line);
        }
        Ok(Some(lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(number: u64, name: &str, at: u64) -> Line {
        Line {
            number,
            name: name.to_string(),
            text: format!("line {number}"),
            sent_at: 5,
            at,
        }
    }

    /// A file of its own for a test, gone when it is dropped.
    struct File(std::path::PathBuf);

    impl File {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("xmoq-chat-{}-{name}.sqlite3", std::process::id()));
            Self::remove(&path);
            Self(path)
        }

        fn remove(path: &Path) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            Self::remove(&self.0);
        }
    }

    #[test]
    fn what_is_kept_is_there_when_the_store_is_opened_again() {
        let file = File::new("again");
        {
            let store = Store::open(&file.0).unwrap();
            assert_eq!(store.next_number("r").unwrap(), 0);
            for (number, name, at) in [(0, "a", 10), (1, "b", 11), (2, "a", 12)] {
                store.keep("r", &line(number, name, at)).unwrap();
            }
            store.keep("s", &line(0, "c", 20)).unwrap();
        }
        let store = Store::open(&file.0).unwrap();
        assert_eq!(store.next_number("r").unwrap(), 3);
        assert_eq!(store.next_number("t").unwrap(), 0);
        assert_eq!(
            store.rooms(10).unwrap(),
            [("s".to_string(), 1), ("r".to_string(), 3)]
        );
        assert_eq!(store.rooms(1).unwrap(), [("s".to_string(), 1)]);
    }

    #[test]
    fn a_range_of_lines_is_read_in_its_order() {
        let file = File::new("range");
        let store = Store::open(&file.0).unwrap();
        for number in 0..6 {
            store.keep("r", &line(number, "a", number)).unwrap();
        }
        store.keep("s", &line(0, "b", 0)).unwrap();
        let cases: [(&str, u64, u64, &[u64]); 4] = [
            ("all", 0, 5, &[0, 1, 2, 3, 4, 5]),
            ("the middle", 2, 3, &[2, 3]),
            ("past the end", 4, 100, &[4, 5]),
            ("nothing", 7, 9, &[]),
        ];
        for (case, first, last, expected) in cases {
            let numbers: Vec<u64> = store
                .lines("r", (first, last), usize::MAX)
                .unwrap()
                .unwrap()
                .iter()
                .map(|line| line.number)
                .collect();
            assert_eq!(numbers, expected, "{case}");
        }
        assert_eq!(
            store.lines("r", (1, 1), usize::MAX).unwrap(),
            Some(vec![line(1, "a", 1)])
        );
        // Each payload here is the same length: two fit, three do not.
        let one = line(0, "a", 0).payload().len();
        assert_eq!(
            store
                .lines("r", (0, 1), one * 2)
                .unwrap()
                .map(|lines| lines.len()),
            Some(2)
        );
        assert_eq!(store.lines("r", (0, 2), one * 2 + 1).unwrap(), None);
    }

    #[test]
    fn a_store_is_one_gatherers() {
        let file = File::new("one");
        let store = Store::open(&file.0).unwrap();
        let error = Store::open(&file.0).err().unwrap();
        assert!(format!("{error:#}").contains("is in use"), "{error:#}");
        drop(store);
        assert!(Store::open(&file.0).is_ok(), "once the first is closed");
    }

    #[test]
    fn a_number_is_kept_once() {
        let file = File::new("once");
        let store = Store::open(&file.0).unwrap();
        store.keep("r", &line(0, "a", 0)).unwrap();
        assert!(store.keep("r", &line(0, "b", 1)).is_err());
    }
}
