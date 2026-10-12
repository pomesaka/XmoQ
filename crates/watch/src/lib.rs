//! A viewer for the terminal: the picture of a broadcast with the Kitty
//! graphics protocol, its sound on the machine's output, and the chat of
//! its room.
//!
//! This is the part that touches nothing: when an Object is due
//! ([`clock`]), which sample the output gets next ([`playout`]), how a
//! picture is made to fit a terminal ([`fit`]) and written for it
//! ([`kitty`]), which tracks of a catalog are played ([`tracks`]), what a
//! key asks for ([`keys`]), and what a room says ([`room`]). Each is given
//! its times and its bytes, and none reads a clock, a terminal or a socket.

pub mod clock;
pub mod fit;
pub mod keys;
pub mod kitty;
pub mod playout;
pub mod room;
pub mod tracks;
