//! The names of `xmoq-pet-tracks`, which are text, as the session's
//! namespaces and Full Track Names, which are bytes.

use xmoq_moqt::UsageError;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_pet_tracks::names::{
    self, ACTED, Actor, GARDEN, GardenTrack, PET_NAME, PREFIX, STATE, actor_namespace,
    room_namespace,
};

/// A track the pet publishes in its namespace of a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mine {
    /// `state`, its body.
    State,
    /// `acted`, what it does to the garden.
    Acted,
}

impl Mine {
    /// Its name, as the log and the track have it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::State => STATE,
            Self::Acted => ACTED,
        }
    }
}

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

/// `xmoq/pet`: what the pet asks for the namespaces under.
///
/// # Errors
///
/// None in practice: the prefix is two fields that are not empty.
pub fn prefix() -> Result<TrackNamespace, UsageError> {
    namespace_of(PREFIX.map(str::to_string).to_vec())
}

/// The actor other than the pet a namespace is of
/// ([`names::actor_of`]): a person's page, by which the pet learns of a
/// room. `None` for any other, the pet's own among them.
///
/// WHY a room is learned of by its actors, and not by a namespace of the
/// room: the garden announces none (a PUBLISH of `garden`, why at the top of
/// `crates/garden/src/bin/xmoq-garden/session.rs`), and the relay tells a
/// `SUBSCRIBE_NAMESPACE` of announced namespaces alone. WHY the pet's is
/// passed over: the pet does not come into a room for its own sake, and
/// its own announcement would keep the room for ever.
#[must_use]
pub fn person_of(namespace: &TrackNamespace) -> Option<Actor> {
    names::actor_of(&text_of(namespace)?).filter(|actor| actor.name != PET_NAME)
}

/// `xmoq/pet/<room>/pet`: what the pet announces in a room, as text, the
/// fields joined with `/` as a token names a namespace.
#[must_use]
pub fn mine_text(room: &str) -> String {
    actor_namespace(room, PET_NAME).join("/")
}

/// `xmoq/pet/<room>/pet`.
///
/// # Errors
///
/// Where the namespace is longer than §2.4.1 allows.
pub fn mine(room: &str) -> Result<TrackNamespace, UsageError> {
    namespace_of(actor_namespace(room, PET_NAME))
}

/// The track `track` of the pet in `room`.
///
/// # Errors
///
/// As [`mine`].
pub fn my_track(room: &str, track: Mine) -> Result<FullTrackName, UsageError> {
    FullTrackName::new(mine(room)?, track.word().as_bytes().to_vec())
}

/// `garden` of a room.
///
/// # Errors
///
/// As [`mine`].
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
    fn a_room_is_learned_of_by_a_person_in_it() {
        let ann = Actor {
            room: "r".to_string(),
            name: "ann".to_string(),
        };
        let cases: [(&str, TrackNamespace, Option<Actor>); 5] = [
            (
                "a person",
                namespace(&[b"xmoq", b"pet", b"r", b"ann"]),
                Some(ann),
            ),
            ("the pet", namespace(&[b"xmoq", b"pet", b"r", b"pet"]), None),
            ("a room", namespace(&[b"xmoq", b"pet", b"r"]), None),
            (
                "a field not UTF-8",
                namespace(&[b"xmoq", b"pet", b"r", b"\xff"]),
                None,
            ),
            (
                "another prefix",
                namespace(&[b"xmoq", b"chat", b"r", b"ann"]),
                None,
            ),
        ];
        for (case, namespace, expected) in cases {
            assert_eq!(person_of(&namespace), expected, "{case}");
        }
    }

    #[test]
    fn the_bytes_of_a_name_are_read_as_its_text() {
        let cases: [(FullTrackName, GardenTrack); 3] = [
            (
                garden("r").unwrap(),
                GardenTrack::Garden {
                    room: "r".to_string(),
                },
            ),
            (
                my_track("r", Mine::State).unwrap(),
                GardenTrack::State {
                    room: "r".to_string(),
                },
            ),
            (
                my_track("r", Mine::Acted).unwrap(),
                GardenTrack::Acted {
                    room: "r".to_string(),
                    name: PET_NAME.to_string(),
                },
            ),
        ];
        for (track, expected) in cases {
            assert_eq!(track_of(&track), Some(expected));
        }
        assert_eq!(
            mine("r").unwrap(),
            namespace(&[b"xmoq", b"pet", b"r", b"pet"])
        );
        assert_eq!(mine_text("r"), "xmoq/pet/r/pet");
        assert_eq!(prefix().unwrap(), namespace(&[b"xmoq", b"pet"]));
    }
}
