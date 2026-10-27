//! A viewer for the terminal: the picture of a broadcast with the Kitty
//! graphics protocol, its sound on the machine's output, and the chat of
//! its room.
//!
//! This is the part that touches nothing: when an Object is due
//! ([`clock`]), which sample the output gets next ([`playout`]), how a
//! picture is made to fit a terminal ([`fit`]) and written for it
//! ([`kitty`]), which tracks of a catalog are played ([`tracks`]), what a
//! key asks for ([`keys`]), what the flags mean ([`args`]), and what is
//! measured with a publisher that marks its picture with a white square and
//! its sound with a beep at the same moments ([`marks`]; the demo's
//! publishing page has such marks, `marked` and `MARK` in
//! `demo/broadcast/picture.ts`). Each is given its times and its bytes, and
//! none reads a clock, a terminal or a socket. What a room says and which
//! lines of its chat are shown are `xmoq-viewer`'s, which `xmoq-agent`
//! shares, and its input and output are `xmoq-viewer-io`'s.
//!
//! The binary (`src/bin/xmoq-watch/`) does the rest of the input and
//! output, one kind a file: `terminal.rs` (line mode, keys, the window's
//! size), `picture.rs` (the thread that decodes and writes frames),
//! `sound.rs` (the Opus decoder and the queue of the output) and
//! `audio_output.rs` (the output, which may be replaced).

pub mod args;
pub mod clock;
pub mod fit;
pub mod keys;
pub mod kitty;
pub mod marks;
pub mod playout;
pub mod tracks;
