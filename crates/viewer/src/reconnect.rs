//! The waits of a viewer that connects to a room and a relay, which
//! `xmoq-watch` and `xmoq-agent` both have. When and where the next session
//! goes is `xmoq_again::next_session`.

/// How long a room gets for each of its answers (a token, a seat), by
/// default; `--room-wait-ms` of both programs. **Provisional.** WHY a flag:
/// a room can stand far from the viewer.
pub const ROOM_WAIT_MS: u64 = 5_000;

/// How long a relay gets to be connected to, by default; `--connect-wait-ms`
/// of both programs. **Provisional.**
pub const CONNECT_WAIT_MS: u64 = 10_000;

/// The least time from one try at a session to the next, however often a
/// room names another place (`NextSession` of `xmoq-again`).
/// **Provisional.**
pub const BETWEEN_TRIES_MS: u64 = 1_000;
