//! The namespaces and tracks of a room's garden: a room is `xmoq/pet/<room>`,
//! where the garden publishes [`GARDEN`]; an actor announces
//! `xmoq/pet/<room>/<name>` and publishes [`ACTED`] there; the actor named
//! [`PET_NAME`] also publishes [`STATE`], its body.

/// The fields every namespace of the garden begins with.
pub const PREFIX: [&str; 2] = ["xmoq", "pet"];

/// The name of the actor that is the pet.
pub const PET_NAME: &str = "pet";

/// The track of a room: the garden as it is now, one Object a Group.
pub const GARDEN: &str = "garden";

/// The track of an actor: what it does to the garden, one Object a Group.
pub const ACTED: &str = "acted";

/// The track of the pet alone: its body, one Object a Group.
pub const STATE: &str = "state";

/// `xmoq/pet/<room>`: where the garden publishes [`GARDEN`]. The caller
/// keeps `room` from being empty; an empty one is nobody's ([`track_of`]).
#[must_use]
pub fn room_namespace(room: &str) -> Vec<String> {
    vec![
        PREFIX[0].to_string(),
        PREFIX[1].to_string(),
        room.to_string(),
    ]
}

/// `xmoq/pet/<room>/<name>`: what an actor announces, the pet's with
/// [`PET_NAME`]. The caller keeps `room` and `name` from being empty, as
/// [`room_namespace`].
#[must_use]
pub fn actor_namespace(room: &str, name: &str) -> Vec<String> {
    let mut fields = room_namespace(room);
    fields.push(name.to_string());
    fields
}

/// One actor of one room.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Actor {
    /// The third field of the namespace.
    pub room: String,
    /// The fourth.
    pub name: String,
}

/// A track of the garden.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GardenTrack {
    /// [`GARDEN`] of a room.
    Garden {
        /// The room.
        room: String,
    },
    /// [`ACTED`] of an actor.
    Acted {
        /// Its room.
        room: String,
        /// Its name.
        name: String,
    },
    /// [`STATE`] of the pet of a room.
    State {
        /// The room.
        room: String,
    },
}

/// Whether the two first fields are [`PREFIX`].
fn ours(xmoq: &str, pet: &str) -> bool {
    [xmoq, pet] == PREFIX
}

/// The room of `xmoq/pet/<room>`; `None` for any other namespace.
fn room_of(namespace: &[String]) -> Option<&str> {
    let [xmoq, pet, room] = namespace else {
        return None;
    };
    (ours(xmoq, pet) && !room.is_empty()).then_some(room.as_str())
}

/// The actor a namespace is of: exactly four fields, `xmoq`, `pet`, a room
/// and a name, neither empty. The pet is one, named [`PET_NAME`]. `None` for
/// any other.
///
/// WHY any other is passed over and not refused: the prefix is the garden's,
/// and what else is announced under it (the namespace of a room, a field
/// more) is nobody's action. WHY an empty room or name is nobody's: the name
/// is the `by` of what the actor puts in the garden, where an empty one is
/// refused.
#[must_use]
pub fn actor_of(namespace: &[String]) -> Option<Actor> {
    let [xmoq, pet, room, name] = namespace else {
        return None;
    };
    (ours(xmoq, pet) && !room.is_empty() && !name.is_empty()).then(|| Actor {
        room: room.clone(),
        name: name.clone(),
    })
}

/// The track of the garden `track` of `namespace` is; `None` for any other,
/// such as [`GARDEN`] in an actor's namespace or [`STATE`] in that of an
/// actor other than the pet.
#[must_use]
pub fn track_of(namespace: &[String], track: &str) -> Option<GardenTrack> {
    if let Some(Actor { room, name }) = actor_of(namespace) {
        return match track {
            ACTED => Some(GardenTrack::Acted { room, name }),
            STATE if name == PET_NAME => Some(GardenTrack::State { room }),
            _ => None,
        };
    }
    let room = room_of(namespace)?;
    (track == GARDEN).then(|| GardenTrack::Garden {
        room: room.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fixtures/pet-tracks/names.tsv`, which `@xmoq/pet` reads as well.
    /// Where a row is somebody's, the namespace made of what was read is the
    /// row's.
    #[test]
    fn a_namespace_and_a_track_are_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/pet-tracks/names.tsv");
        assert!(table.ends_with('\n') && !table.ends_with("\n\n"));
        let mut read = 0;
        for row in table.lines() {
            let [case, fields, track, expected] = row.split('\t').collect::<Vec<_>>()[..] else {
                panic!("a row of four columns: {row}");
            };
            let fields: Vec<String> = serde_json::from_str(fields).expect("a JSON array");
            let (got, made) = if track == "-" {
                actor_of(&fields).map_or_else(
                    || ("none".to_string(), None),
                    |Actor { room, name }| {
                        (
                            format!("actor {room} {name}"),
                            Some(actor_namespace(&room, &name)),
                        )
                    },
                )
            } else {
                match track_of(&fields, track) {
                    None => ("none".to_string(), None),
                    Some(GardenTrack::Garden { room }) => {
                        (format!("{GARDEN} {room}"), Some(room_namespace(&room)))
                    }
                    Some(GardenTrack::Acted { room, name }) => (
                        format!("{ACTED} {room} {name}"),
                        Some(actor_namespace(&room, &name)),
                    ),
                    Some(GardenTrack::State { room }) => (
                        format!("{STATE} {room}"),
                        Some(actor_namespace(&room, PET_NAME)),
                    ),
                }
            };
            assert_eq!(got, expected, "{case}");
            if let Some(made) = made {
                assert_eq!(made, fields, "{case}");
            }
            read += 1;
        }
        // A table that lost its rows would pass.
        assert!(read > 0);
    }
}
