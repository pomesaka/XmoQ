//! The pet of a room: an actor of the room's garden, named `pet`. It is
//! told of every room (`SUBSCRIBE_NAMESPACE` for `xmoq/pet`), perceives the
//! room's `garden`, decides what the pet does, publishes the pet's body on
//! `state` and what it does to the garden on `acted`, in its namespace of
//! the room.
//!
//! This is the part that touches nothing: the pet and how it wanders
//! ([`pet`]), its whims ([`chance`]), what it perceives of a garden
//! ([`perceive`]), what a batch of it comes to by the table ([`decide`]) or
//! by the decisions API ([`decisions`], the request and the reading of its
//! answer), how it is gathered into a batch ([`batch`]), how often the API
//! is called ([`rate`]), the rooms and their states ([`rooms`]), the names
//! as the session has them ([`tracks`]), and what the flags mean
//! ([`args`]). Each is given its times and its bytes, and none reads a
//! clock, a socket or the environment. The names and payloads of the
//! tracks are `xmoq-pet-tracks`, which the other sides of the garden use
//! too.

pub mod args;
pub mod batch;
pub mod chance;
pub mod decide;
pub mod decisions;
pub mod perceive;
pub mod pet;
pub mod rate;
pub mod rooms;
pub mod tracks;
