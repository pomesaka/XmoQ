//! What the pet makes of a batch of what it perceived of the garden: a
//! reaction, a change of its mood and a bubble. The decisions API decides it where it is given
//! ([`crate::decisions`]); [`fixed`] decides it otherwise, and where the API
//! gives no answer.

use xmoq_pet_tracks::payloads::{Doing, EventKind, Thing, ThingKind};

use crate::perceive::Happening;

/// What the pet does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reaction {
    /// Walks to the front middle.
    Approach,
    /// Walks away from the front.
    Flee,
    /// Goes on as it was.
    Ignore,
    /// Jumps where it is.
    Jump,
    /// Goes to the rug and sleeps.
    Sleep,
    /// Sits where it is.
    Sit,
    /// Goes to the nearest bowl and eats from it.
    Eat,
    /// Runs to the ball and takes it.
    Chase,
}

impl Reaction {
    /// Every reaction, in the order the API is offered them.
    pub const ALL: [Self; 8] = [
        Self::Approach,
        Self::Flee,
        Self::Ignore,
        Self::Jump,
        Self::Sleep,
        Self::Sit,
        Self::Eat,
        Self::Chase,
    ];

    /// Its word, in the log and to the API.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Approach => "approach",
            Self::Flee => "flee",
            Self::Ignore => "ignore",
            Self::Jump => "jump",
            Self::Sleep => "sleep",
            Self::Sit => "sit",
            Self::Eat => "eat",
            Self::Chase => "chase",
        }
    }

    /// When the pet reacts so, as the API is told of each choice. WHY each
    /// is described: without it the API gave `sit` for praise and `same`
    /// for play (the PM's runs of 2026-10-28).
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Approach => "comes closer when spoken to kindly, called or petted",
            Self::Flee => "backs away when scolded, frightened or treated roughly",
            Self::Ignore => "goes on with what it was doing when nothing done concerns it",
            Self::Jump => "jumps for joy when excited or played with",
            Self::Sleep => "goes to sleep on the rug when tired, cross or left alone",
            Self::Sit => "sits and looks when it is puzzled or told to sit",
            Self::Eat => "eats food that was put out, waking up if asleep",
            Self::Chase => "runs after what was thrown",
        }
    }

    /// The reaction whose word is `word`.
    #[must_use]
    pub fn of(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reaction| reaction.word() == word)
    }

    /// What the pet is doing while it reacts so; `None` for [`Self::Ignore`],
    /// which keeps what it was doing.
    #[must_use]
    pub fn doing(self) -> Option<Doing> {
        match self {
            Self::Approach => Some(Doing::Approach),
            Self::Flee => Some(Doing::Flee),
            Self::Ignore => None,
            Self::Jump => Some(Doing::Jump),
            Self::Sleep => Some(Doing::Sleep),
            Self::Sit => Some(Doing::Sit),
            Self::Eat => Some(Doing::Eat),
            Self::Chase => Some(Doing::Chase),
        }
    }
}

/// What the pet may say, and when it says it, as the API is told of each
/// choice. Each fits `BUBBLE_BYTES` of `xmoq-pet-tracks` (the test below);
/// a viewer shows it as it is. WHY each is described: alike ones (`♪` and
/// `あそぼ`, `zz` and `ねむい…`) came with a low confidence, and `zz` came
/// for food given to a sleeping pet (the PM's runs of 2026-10-28).
pub const BUBBLES: [(&str, &str); 15] = [
    ("♪", "humming, content with nothing in particular"),
    ("!", "surprised by something sudden"),
    ("?", "puzzled by what was done or said"),
    ("♥", "fond of whoever petted it or spoke kindly"),
    ("zz", "staying asleep, not woken by what was done"),
    ("なに？", "\"what is it?\": answering a call"),
    (
        "おなかすいた",
        "\"I'm hungry\": wanting food that was not given",
    ),
    (
        "もっと！",
        "\"more!\": wanting the same play or petting again",
    ),
    ("ねむい…", "\"sleepy...\": awake but tired, about to sleep"),
    ("あそぼ", "\"let's play\": asking to play"),
    ("やった！", "\"yay!\": glad of food or a treat"),
    (
        "こわい…",
        "\"scary...\": frightened by scolding or rough treatment",
    ),
    ("いいこ？", "\"am I good?\": hoping for praise"),
    ("ボール！", "\"ball!\": seeing a ball thrown"),
    ("おかえり", "\"welcome back\": greeting someone who came"),
];

/// The bubble of [`BUBBLES`] that is `text`.
#[must_use]
pub fn bubble_of(text: &str) -> Option<&'static str> {
    BUBBLES
        .into_iter()
        .map(|(bubble, _)| bubble)
        .find(|bubble| *bubble == text)
}

/// What a batch comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// What the pet does.
    pub reaction: Reaction,
    /// How its mood changes, -3 to +3: the API's score with what was said.
    pub mood_change: i8,
    /// What it says; `None` for nothing.
    pub bubble: Option<&'static str>,
}

/// What a decision is made upon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upon {
    /// A batch of happenings.
    Happened,
    /// Nothing new: the pet went back to itself with something on the
    /// floor, and looks at it ([`crate::pet::Pet::look`]).
    Floor,
}

/// The decision of the table for `batch`, with `floor` the things on the
/// floor the pet has not acted on: [`fixed`] for a batch of happenings,
/// [`fixed_floor`] for none.
#[must_use]
pub fn table(batch: &[Happening], floor: &[Thing]) -> Option<Decision> {
    if batch.is_empty() {
        fixed_floor(floor)
    } else {
        fixed(batch)
    }
}

/// The decision of the table where nothing new happened, by what lies on
/// `floor`: a bowl is eaten, else the ball is chased; `None` for nothing.
/// WHY food before the ball: food is what a dog goes to first. The mood
/// does not change: nobody did anything. **Provisional.**
#[must_use]
pub fn fixed_floor(floor: &[Thing]) -> Option<Decision> {
    let lies = |kind: ThingKind| floor.iter().any(|thing| thing.kind == kind);
    let (reaction, bubble) = if lies(ThingKind::Bowl) {
        (Reaction::Eat, "やった！")
    } else if lies(ThingKind::Ball) {
        (Reaction::Chase, "ボール！")
    } else {
        return None;
    };
    Some(Decision {
        reaction,
        mood_change: 0,
        bubble: Some(bubble),
    })
}

/// The decision of the table: the last happening of the batch decides, as
/// the newest the people in the room did. `None` for an empty batch.
#[must_use]
pub fn fixed(batch: &[Happening]) -> Option<Decision> {
    let (reaction, mood_change, bubble) = match batch.last()? {
        Happening::Put(thing) => match thing.kind {
            ThingKind::Bowl => (Reaction::Eat, 1, "やった！"),
            ThingKind::Ball => (Reaction::Chase, 1, "ボール！"),
        },
        Happening::Done(event) => match event.kind {
            EventKind::Pet => (Reaction::Approach, 1, "♥"),
            EventKind::Call => (Reaction::Approach, 0, "なに？"),
            EventKind::Talk => (Reaction::Sit, 0, "?"),
        },
    };
    Some(Decision {
        reaction,
        mood_change,
        bubble: Some(bubble),
    })
}

/// The reactions that make sense with `floor`, the things on the floor of
/// the garden: no eating without a bowl on it, no chase without a ball.
#[must_use]
pub fn sensible(floor: &[Thing]) -> Vec<Reaction> {
    let lies = |kind: ThingKind| floor.iter().any(|thing| thing.kind == kind);
    let (bowl, ball) = (lies(ThingKind::Bowl), lies(ThingKind::Ball));
    Reaction::ALL
        .into_iter()
        .filter(|reaction| match reaction {
            Reaction::Chase => ball,
            Reaction::Eat => bowl,
            _ => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use xmoq_pet_tracks::payloads::{BUBBLE_BYTES, Event, Place};

    use super::*;

    fn put(kind: ThingKind) -> Happening {
        Happening::Put(thing(kind))
    }

    fn thing(kind: ThingKind) -> Thing {
        Thing {
            id: 1,
            kind,
            at: Place { x: 1, y: 2 },
            by: "alice".to_string(),
        }
    }

    fn done(kind: EventKind) -> Happening {
        Happening::Done(Event {
            id: 1,
            kind,
            by: "alice".to_string(),
            text: (kind == EventKind::Talk).then(|| "hi".to_string()),
            at: 0,
        })
    }

    #[test]
    fn the_table_decides_by_the_last_happening() {
        // (case, the batch, the reaction, mood change and bubble)
        type Deciding = (
            &'static str,
            Vec<Happening>,
            Option<(Reaction, i8, &'static str)>,
        );
        let cases: [Deciding; 7] = [
            (
                "pet",
                vec![done(EventKind::Pet)],
                Some((Reaction::Approach, 1, "♥")),
            ),
            (
                "a bowl",
                vec![put(ThingKind::Bowl)],
                Some((Reaction::Eat, 1, "やった！")),
            ),
            (
                "call",
                vec![done(EventKind::Call)],
                Some((Reaction::Approach, 0, "なに？")),
            ),
            (
                "a ball",
                vec![put(ThingKind::Ball)],
                Some((Reaction::Chase, 1, "ボール！")),
            ),
            (
                "talk",
                vec![done(EventKind::Talk)],
                Some((Reaction::Sit, 0, "?")),
            ),
            (
                "the last of several",
                vec![
                    put(ThingKind::Bowl),
                    put(ThingKind::Ball),
                    done(EventKind::Call),
                ],
                Some((Reaction::Approach, 0, "なに？")),
            ),
            ("none", vec![], None),
        ];
        for (case, batch, expected) in cases {
            let expected = expected.map(|(reaction, mood_change, bubble)| Decision {
                reaction,
                mood_change,
                bubble: Some(bubble),
            });
            assert_eq!(fixed(&batch), expected, "{case}");
        }
    }

    #[test]
    fn with_nothing_new_the_table_decides_by_the_floor() {
        // (case, the floor, the batch, the reaction)
        type Looking = (
            &'static str,
            Vec<ThingKind>,
            Vec<Happening>,
            Option<Reaction>,
        );
        let cases: [Looking; 5] = [
            ("a bowl", vec![ThingKind::Bowl], vec![], Some(Reaction::Eat)),
            (
                "the bowl before the ball",
                vec![ThingKind::Ball, ThingKind::Bowl],
                vec![],
                Some(Reaction::Eat),
            ),
            (
                "the ball",
                vec![ThingKind::Ball],
                vec![],
                Some(Reaction::Chase),
            ),
            ("nothing", vec![], vec![], None),
            (
                "a batch is decided by its happenings",
                vec![ThingKind::Bowl],
                vec![done(EventKind::Pet)],
                Some(Reaction::Approach),
            ),
        ];
        for (case, kinds, batch, expected) in cases {
            let floor: Vec<Thing> = kinds.into_iter().map(thing).collect();
            let got = table(&batch, &floor);
            assert_eq!(got.map(|decision| decision.reaction), expected, "{case}");
            if batch.is_empty() {
                assert_eq!(got.map_or(0, |decision| decision.mood_change), 0, "{case}");
                assert!(
                    got.and_then(|decision| decision.bubble)
                        .is_none_or(|bubble| bubble_of(bubble).is_some()),
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn the_bubbles_are_the_list_asked_for_and_each_fits() {
        assert_eq!(
            BUBBLES.map(|(bubble, _)| bubble),
            [
                "♪",
                "!",
                "?",
                "♥",
                "zz",
                "なに？",
                "おなかすいた",
                "もっと！",
                "ねむい…",
                "あそぼ",
                "やった！",
                "こわい…",
                "いいこ？",
                "ボール！",
                "おかえり",
            ]
        );
        for (bubble, when) in BUBBLES {
            assert!(!when.is_empty(), "{bubble}");
            assert!(
                (1..=BUBBLE_BYTES).contains(&bubble.len()),
                "{bubble}: {} bytes",
                bubble.len()
            );
            assert_eq!(bubble_of(bubble), Some(bubble));
        }
        assert_eq!(bubble_of("none"), None);
        // The table's own bubbles are of the list.
        for happening in [
            done(EventKind::Pet),
            done(EventKind::Call),
            done(EventKind::Talk),
            put(ThingKind::Bowl),
            put(ThingKind::Ball),
        ] {
            let decided = fixed(&[happening]).and_then(|decision| decision.bubble);
            assert!(decided.and_then(bubble_of).is_some(), "{decided:?}");
        }
    }

    #[test]
    fn which_reactions_make_sense_with_the_floor() {
        let cases: [(&str, Vec<ThingKind>, bool, bool); 4] = [
            ("nothing on the floor", vec![], false, false),
            ("a ball", vec![ThingKind::Ball], true, false),
            ("a bowl", vec![ThingKind::Bowl], false, true),
            (
                "both",
                vec![ThingKind::Ball, ThingKind::Bowl, ThingKind::Bowl],
                true,
                true,
            ),
        ];
        for (case, kinds, chase, eat) in cases {
            let floor: Vec<Thing> = kinds.into_iter().map(thing).collect();
            let got = sensible(&floor);
            assert_eq!(got.contains(&Reaction::Chase), chase, "{case}");
            assert_eq!(got.contains(&Reaction::Eat), eat, "{case}");
            assert_eq!(
                got.len(),
                6 + usize::from(chase) + usize::from(eat),
                "{case}"
            );
        }
    }

    #[test]
    fn a_reaction_is_read_from_its_word() {
        for reaction in Reaction::ALL {
            assert_eq!(Reaction::of(reaction.word()), Some(reaction));
        }
        assert_eq!(Reaction::of("bite"), None);
    }
}
