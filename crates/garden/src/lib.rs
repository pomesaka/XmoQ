//! The garden of a room: it is told of every actor of every room
//! (`SUBSCRIBE_NAMESPACE` for `xmoq/pet`), takes what each does on its track
//! `acted`, changes the room's garden by its rules, and publishes the garden
//! on the track `garden` of the room.
//!
//! This is the part that touches nothing: the rules of a garden
//! ([`rules`]), the rooms and their gardens ([`rooms`]), the names as the
//! session has them ([`tracks`]), and what the flags mean ([`args`]). Each
//! is given its times and its bytes, and none reads a clock, a socket or the
//! environment. The names and payloads of the tracks are `xmoq-pet-tracks`,
//! which the pet and the pages use too.

pub mod args;
pub mod rooms;
pub mod rules;
pub mod tracks;
