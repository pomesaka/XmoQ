//! The transcriber (`crates/transcriber/README.md`): a client of the relay
//! beside it, as the gatherer of the chat is, that listens to the audio
//! tracks of the broadcasts announced under a prefix and publishes their
//! words.
//!
//! This is the part that touches nothing: what the flags mean ([`args`]),
//! what is decided of each broadcast listened to ([`listen`]), and the
//! numbering and keeping of a transcript ([`transcript`]), and the words
//! that are no words, which it does not publish ([`marks`]). How the
//! sound is cut and read is `xmoq-speech`; the names and the payload of the
//! track are `xmoq-transcript-tracks`, which its readers use too.

pub mod args;
pub mod listen;
pub mod marks;
pub mod transcript;
