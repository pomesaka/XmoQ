//! The limits of the chat (Q43, decided on 2026-10-24), with their defaults,
//! and how far a setting may take them.

/// The limits of the chat (Q43, decided on 2026-10-24), which flags may
/// change. What they bound is the chat's; what the gatherer reads from the
/// relay is bounded elsewhere, and no flag removes that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// `--text-bytes`: the most bytes the text of a line may have. A longer
    /// line is dropped and counted.
    pub text_bytes: usize,
    /// `--lines-per-second`: the most lines one person may say in any
    /// second. A line over it is dropped and counted.
    pub lines_per_second: usize,
    /// `--fetch-lines`: the most lines one FETCH gets (the gatherer's `fetch` module).
    pub fetch_lines: u64,
}

/// The default of [`Limits::text_bytes`]: 512 bytes. A participant uses it to
/// keep back a line the gatherer would drop.
pub const TEXT_BYTES: usize = 512;

/// The default of [`Limits::lines_per_second`].
pub const LINES_PER_SECOND: usize = 5;

/// The default of [`Limits::fetch_lines`].
pub const FETCH_LINES: u64 = 1_000;

impl Default for Limits {
    fn default() -> Self {
        Self {
            text_bytes: TEXT_BYTES,
            lines_per_second: LINES_PER_SECOND,
            fetch_lines: FETCH_LINES,
        }
    }
}

/// The most `--text-bytes` may be.
///
/// WHY a bound on the flag: a line has to fit in what the gatherer reads of
/// one stream of `said` (`PAYLOAD_MOST` of `xmoq-chat`, 64 KiB). JSON writes a byte of text in
/// six bytes at the most (`\u001f`), so 8,192 bytes of text are 49,152 of
/// JSON, and the two members and the stream's header fit beside them.
pub const TEXT_BYTES_MOST: usize = 8_192;

/// The most `--lines-per-second` may be. WHY a bound: the window of a
/// person keeps the time of each line it took in the last second.
pub const LINES_PER_SECOND_MOST: usize = 1_000;

/// The most `--fetch-lines` may be. WHY a bound: the lines of one FETCH are
/// read from the store and held until they are written.
pub const FETCH_LINES_MOST: u64 = 100_000;
