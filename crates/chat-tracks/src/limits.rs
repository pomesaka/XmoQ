//! The limits of the chat (decided on 2026-10-24), with their defaults,
//! and how far a setting may take them.

/// The limits of the chat (decided on 2026-10-24), which flags may
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
    /// `--reaction-batch-ms`: the while the gatherer counts a room's
    /// reactions over before it publishes them on `reactions` (decided on
    /// 2026-10-26).
    pub reaction_batch_ms: u64,
    /// `--reactions-per-second`: the most reactions one person may send in
    /// any second. A reaction over it is dropped (decided on 2026-10-26).
    pub reactions_per_second: usize,
    /// `--reaction-kinds`: the most emoji one while of a room may count. A
    /// reaction with one more is dropped (decided on 2026-10-26).
    pub reaction_kinds: usize,
}

/// The default of [`Limits::text_bytes`]: 512 bytes. A participant uses it to
/// keep back a line the gatherer would drop.
pub const TEXT_BYTES: usize = 512;

/// The default of [`Limits::lines_per_second`].
pub const LINES_PER_SECOND: usize = 5;

/// The default of [`Limits::fetch_lines`].
pub const FETCH_LINES: u64 = 1_000;

/// The default of [`Limits::reaction_batch_ms`]. **Provisional.**
pub const REACTION_BATCH_MS: u64 = 100;

/// The default of [`Limits::reactions_per_second`]. **Provisional.**
pub const REACTIONS_PER_SECOND: usize = 5;

/// The default of [`Limits::reaction_kinds`]. **Provisional.**
pub const REACTION_KINDS: usize = 16;

impl Default for Limits {
    fn default() -> Self {
        Self {
            text_bytes: TEXT_BYTES,
            lines_per_second: LINES_PER_SECOND,
            fetch_lines: FETCH_LINES,
            reaction_batch_ms: REACTION_BATCH_MS,
            reactions_per_second: REACTIONS_PER_SECOND,
            reaction_kinds: REACTION_KINDS,
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

/// The least `--reaction-batch-ms` may be. WHY a bound: each while that had
/// reactions is an Object of `reactions` for every viewer of the room.
/// **Provisional.**
pub const REACTION_BATCH_MS_LEAST: u64 = 10;

/// The most `--reaction-batch-ms` may be: a reaction shown a minute late is
/// no reaction. **Provisional.**
pub const REACTION_BATCH_MS_MOST: u64 = 60_000;

/// The most `--reactions-per-second` may be. WHY a bound: the window of a
/// person keeps the time of each reaction it took in the last second.
pub const REACTIONS_PER_SECOND_MOST: usize = 1_000;

/// The most `--reaction-kinds` may be. WHY a bound: an emoji is up to 8
/// UTF-16 units, 32 bytes of UTF-8, and its count up to 16 digits, so 256 of
/// them are some 16 KiB of `reactions`, which fits in what a reader takes of
/// one Object of a chat's track (64 KiB). **Provisional.**
pub const REACTION_KINDS_MOST: usize = 256;
