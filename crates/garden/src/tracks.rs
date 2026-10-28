//! The names of `xmoq-pet-tracks`, which are text, as the session's
//! namespaces and Full Track Names, which are bytes.

use xmoq_moqt::UsageError;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_pet_tracks::names::{
    self, ACTED, Actor, GARDEN, GardenTrack, PREFIX, actor_namespace, room_namespace,
};

/// The fields of a namespace as text; `None` where one is not UTF-8, which
/// no namespace of the garden is.
fn text_of(namespace: &TrackNamespace) -> Option<Vec<String>> {
    namespace
        .fields()
        .iter()
        .map(|field| String::from_utf8(field.clone()).ok())
        .collect()
}

fn namespace_of(fields: Vec<String>) -> Result<TrackNamespace, UsageError> {
    TrackNamespace::new(fields.into_iter().map(String::into_bytes).collect())
}

/// `xmoq/pet`: what the garden asks for the namespaces under.
///
/// # Errors
///
/// None in practice: the prefix is two fields that are not empty.
pub fn prefix() -> Result<TrackNamespace, UsageError> {
    namespace_of(PREFIX.map(str::to_string).to_vec())
}

/// The actor a namespace is of ([`names::actor_of`]).
#[must_use]
pub fn actor_of(namespace: &TrackNamespace) -> Option<Actor> {
    names::actor_of(&text_of(namespace)?)
}

/// `acted` of an actor.
///
/// # Errors
///
/// Where the namespace is longer than §2.4.1 allows.
pub fn acted(actor: &Actor) -> Result<FullTrackName, UsageError> {
    let namespace = namespace_of(actor_namespace(&actor.room, &actor.name))?;
    FullTrackName::new(namespace, ACTED.as_bytes().to_vec())
}

/// `garden` of a room.
///
/// # Errors
///
/// As [`acted`].
pub fn garden(room: &str) -> Result<FullTrackName, UsageError> {
    FullTrackName::new(
        namespace_of(room_namespace(room))?,
        GARDEN.as_bytes().to_vec(),
    )
}

/// The track of the garden a Full Track Name is ([`names::track_of`]).
#[must_use]
pub fn track_of(track: &FullTrackName) -> Option<GardenTrack> {
    let name = std::str::from_utf8(track.name()).ok()?;
    names::track_of(&text_of(track.namespace())?, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(fields: &[&[u8]]) -> TrackNamespace {
        TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
    }

    #[test]
    fn the_bytes_of_a_name_are_read_as_its_text() {
        let alice = Actor {
            room: "r".to_string(),
            name: "alice".to_string(),
        };
        let cases: [(&str, TrackNamespace, Option<Actor>); 3] = [
            (
                "an actor",
                namespace(&[b"xmoq", b"pet", b"r", b"alice"]),
                Some(alice.clone()),
            ),
            (
                "a field not UTF-8",
                namespace(&[b"xmoq", b"pet", b"r", b"\xff"]),
                None,
            ),
            ("a room", namespace(&[b"xmoq", b"pet", b"r"]), None),
        ];
        for (case, namespace, expected) in cases {
            assert_eq!(actor_of(&namespace), expected, "{case}");
        }
        assert_eq!(
            track_of(&acted(&alice).unwrap()),
            Some(GardenTrack::Acted {
                room: "r".to_string(),
                name: "alice".to_string()
            })
        );
        assert_eq!(
            track_of(&garden("r").unwrap()),
            Some(GardenTrack::Garden {
                room: "r".to_string()
            })
        );
        assert_eq!(prefix().unwrap(), namespace(&[b"xmoq", b"pet"]));
    }
}
