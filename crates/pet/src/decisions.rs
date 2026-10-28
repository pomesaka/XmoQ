//! The request to the decisions API for a batch, and what is read of its
//! answer. The API is of the form of `OpenAI`'s Decisions
//! (`POST /v1/decisions`; `pomesaka/claude-decisions`, its README and
//! `openapi/decisions.openapi.json` as they were on 2026-10-28): `input`,
//! and `questions` of the types `choice`, `score` and `predicate`, each
//! answered by `name` with `choice`, `score` (the expected 0-based level)
//! or `probability`, or with a `refusal`.
//!
//! WHY one request for the batch with four questions, and not a free text
//! answer: each question has its answers fixed (the reactions that make
//! sense, the five levels, the bubbles), so that nothing the API says can
//! become a state the pet cannot have, and a person's text cannot make the
//! pet say anything but a bubble of the list.

use serde_json::{Value, json};
use xmoq_pet_tracks::payloads::{Event, EventKind, PetState, Place, Thing, ThingKind};

use crate::decide::{BUBBLES, Decision, Reaction, bubble_of, sensible};
use crate::perceive::Happening;

/// What the instructions of every question begin with.
const ABOUT: &str = "The pet is a small dog in a garden. The input has its state (what it is doing, its mood from 0, cross, to 4, glad, and where it is on a floor of 0 to 1000 across and into the garden), the things on the floor (a bowl of food or a ball, where it lies, and who put it there), and what the people in the room just did in the garden, the newest last.";

/// What the instructions say after [`ABOUT`] where nothing new happened: the
/// pet went back to itself and looks at the floor
/// ([`crate::pet::Pet::look`]); `happened` is empty then.
const NOTHING_NEW: &str = "Nothing new happened: the dog has finished what it was doing and looks at what is on the floor.";

/// The answer of the bubble question that is no bubble.
pub const NO_BUBBLE: &str = "none";

/// What the bubble question says of [`NO_BUBBLE`].
const NO_BUBBLE_WHEN: &str = "says nothing";

/// What the mood question asks. WHY play and food are named: without it
/// they came out as "same" (the PM's runs of 2026-10-28).
const MOOD_INSTRUCTIONS: &str = "How the pet's mood moves with what was done in this second. Play, food and kind words raise it; scolding lowers it.";

/// The levels of the mood question, worst first, with when each is: the
/// 0-based expected value of the answer is 2 above the change.
pub const MOOD_LEVELS: [(&str, &str); 5] = [
    ("much worse", "scolded harshly, hurt or frightened"),
    (
        "worse",
        "scolded, ignored when it wanted something, or teased",
    ),
    ("same", "nothing that matters to the pet"),
    ("better", "petted, called kindly, played with or fed"),
    (
        "much better",
        "several kind things at once, or a favourite game or treat",
    ),
];

/// A probability of kind words at or above this adds 1 to the mood's change.
pub const KIND_AT_LEAST: f64 = 0.5;

/// One at or below this takes 1 from it.
pub const UNKIND_AT_MOST: f64 = 0.2;

/// A made-up floor and batch for the call that warms the API up at the
/// start, its answer thrown away: a bowl and a ball on the floor and one
/// happening of each kind, so that its request has the four questions and
/// every reaction. WHY: the API compiles the schema of a set of questions
/// at its first call (4 to 11 s, kept for 24 h, the PM's runs of
/// 2026-10-28), which would be a room's first batch. A batch with fewer
/// kinds asks another set, which is compiled at its own first call.
#[must_use]
pub fn warm_up() -> (Vec<Thing>, Vec<Happening>) {
    let by = "warm-up".to_string();
    let at = Place { x: 500, y: 500 };
    let floor: Vec<Thing> = [ThingKind::Bowl, ThingKind::Ball]
        .into_iter()
        .zip(1..)
        .map(|(kind, id)| Thing {
            id,
            kind,
            at,
            by: by.clone(),
        })
        .collect();
    let events = [EventKind::Pet, EventKind::Call, EventKind::Talk]
        .into_iter()
        .zip(1..)
        .map(|(kind, id)| {
            Happening::Done(Event {
                id,
                kind,
                by: by.clone(),
                text: (kind == EventKind::Talk).then(|| "good dog".to_string()),
                at: 0,
            })
        });
    let batch = events
        .chain(floor.iter().cloned().map(Happening::Put))
        .collect();
    (floor, batch)
}

/// Why an answer gave no decision. The batch is decided by the table then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotDecided {
    /// A status that is not 2xx.
    Status,
    /// A body that is no JSON object with `answers`.
    Body,
    /// The API refused a question.
    Refusal,
    /// An answer is missing, or none of the answers offered.
    Answer,
}

impl NotDecided {
    /// One word for the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Body => "body",
            Self::Refusal => "refusal",
            Self::Answer => "answer",
        }
    }
}

/// Whether the batch has anything said, for the question of kind words.
fn talked(batch: &[Happening]) -> bool {
    batch.iter().any(
        |happening| matches!(happening, Happening::Done(event) if event.kind == EventKind::Talk),
    )
}

/// What a thing is called in the input.
fn thing_word(kind: ThingKind) -> &'static str {
    match kind {
        ThingKind::Bowl => "a bowl of food",
        ThingKind::Ball => "the ball",
    }
}

/// A thing on the floor as the input has it.
fn on_the_floor(thing: &Thing) -> Value {
    json!({"thing": thing_word(thing.kind), "at": [thing.at.x, thing.at.y], "by": thing.by})
}

/// A happening as the input has it: who did what, in words.
fn happened(happening: &Happening) -> Value {
    match happening {
        Happening::Put(Thing {
            kind: ThingKind::Bowl,
            at,
            by,
            ..
        }) => json!({"who": by, "did": "put a bowl of food down", "at": [at.x, at.y]}),
        Happening::Put(Thing {
            kind: ThingKind::Ball,
            at,
            by,
            ..
        }) => json!({"who": by, "did": "threw the ball", "to": [at.x, at.y]}),
        Happening::Done(event) => match event.kind {
            EventKind::Call => json!({"who": event.by, "did": "called the pet"}),
            EventKind::Pet => json!({"who": event.by, "did": "petted the pet"}),
            EventKind::Talk => json!({"who": event.by, "did": "said", "said": event.text}),
        },
    }
}

/// The body of the request for `batch`, of a pet in `state` that is at
/// `place`, with `floor` the things on the floor of the garden. An empty
/// batch asks for a decision upon the floor, and says that nothing new
/// happened.
#[must_use]
pub fn request(state: &PetState, place: Place, floor: &[Thing], batch: &[Happening]) -> Value {
    let about = if batch.is_empty() {
        format!("{ABOUT} {NOTHING_NEW}")
    } else {
        ABOUT.to_string()
    };
    let data = json!({
        "pet": {
            "doing": state.doing.word(),
            "mood": state.mood,
            "where": [place.x, place.y],
        },
        "floor": floor.iter().map(on_the_floor).collect::<Vec<_>>(),
        "happened": batch.iter().map(happened).collect::<Vec<_>>(),
    });
    let reactions: Vec<Value> = sensible(floor)
        .into_iter()
        .map(|reaction| json!({"value": reaction.word(), "description": reaction.description()}))
        .collect();
    let bubbles: Vec<Value> = BUBBLES
        .into_iter()
        .chain([(NO_BUBBLE, NO_BUBBLE_WHEN)])
        .map(|(bubble, when)| json!({"value": bubble, "description": when}))
        .collect();
    let mut questions = vec![
        json!({
            "type": "choice",
            "name": "reaction",
            "instructions": format!("{about} What does the dog do next?"),
            "choices": reactions,
        }),
        json!({
            "type": "score",
            "name": "mood_change",
            "instructions": format!("{about} {MOOD_INSTRUCTIONS}"),
            "levels": MOOD_LEVELS.map(|(label, when)| json!({"label": label, "description": when})),
        }),
    ];
    if talked(batch) {
        questions.push(json!({
            "type": "predicate",
            "name": "kind_words",
            "instructions": format!("{about} Is what was said kind to the pet?"),
        }));
    }
    questions.push(json!({
        "type": "choice",
        "name": "bubble",
        "instructions": format!("{about} What does the dog say in its speech bubble? \"{NO_BUBBLE}\" for nothing."),
        "choices": bubbles,
    }));
    json!({
        "input": [{
            "role": "user",
            "content": [{"type": "input_json", "data": data}],
        }],
        "questions": questions,
    })
}

/// The change of the mood of a score: the nearest of the levels, less 2.
/// The API's score is an expected value with fractions (2.9000000000000004
/// was seen on 2026-10-28), so a score a little off the levels is taken to
/// the nearest; one more than half a level off is no answer. WHY the
/// nearest level and not a rounding of the float: no `as` from a float, and
/// a half between two levels takes the lower, the calmer.
fn mood_change(score: f64) -> Option<i8> {
    if !(-0.5..=4.5).contains(&score) {
        return None;
    }
    let level = (0_i8..=4).min_by(|one, other| {
        (f64::from(*one) - score)
            .abs()
            .total_cmp(&(f64::from(*other) - score).abs())
    })?;
    Some(level - 2)
}

/// The decision of an answer of `status` and `body` to the request of
/// `batch` with `floor` ([`request`]).
///
/// # Errors
///
/// Why it gave none.
pub fn read(
    status: u16,
    body: &[u8],
    floor: &[Thing],
    batch: &[Happening],
) -> Result<Decision, NotDecided> {
    if !(200..300).contains(&status) {
        return Err(NotDecided::Status);
    }
    let body: Value = serde_json::from_slice(body).map_err(|_| NotDecided::Body)?;
    let answers = body
        .get("answers")
        .and_then(Value::as_array)
        .ok_or(NotDecided::Body)?;
    let answer = |name: &str| -> Result<&Value, NotDecided> {
        let found = answers
            .iter()
            .find(|answer| answer.get("name").and_then(Value::as_str) == Some(name))
            .ok_or(NotDecided::Answer)?;
        if found.get("type").and_then(Value::as_str) == Some("refusal") {
            return Err(NotDecided::Refusal);
        }
        Ok(found)
    };
    let reaction = answer("reaction")?
        .get("choice")
        .and_then(Value::as_str)
        .and_then(Reaction::of)
        .filter(|reaction| sensible(floor).contains(reaction))
        .ok_or(NotDecided::Answer)?;
    let mut change = answer("mood_change")?
        .get("score")
        .and_then(Value::as_f64)
        .and_then(mood_change)
        .ok_or(NotDecided::Answer)?;
    if talked(batch) {
        let kind = answer("kind_words")?
            .get("probability")
            .and_then(Value::as_f64)
            .filter(|probability| (0.0..=1.0).contains(probability))
            .ok_or(NotDecided::Answer)?;
        if kind >= KIND_AT_LEAST {
            change += 1;
        } else if kind <= UNKIND_AT_MOST {
            change -= 1;
        }
    }
    let bubble = match answer("bubble")?.get("choice").and_then(Value::as_str) {
        Some(NO_BUBBLE) => None,
        Some(text) => Some(bubble_of(text).ok_or(NotDecided::Answer)?),
        None => return Err(NotDecided::Answer),
    };
    Ok(Decision {
        reaction,
        mood_change: change,
        bubble,
    })
}

#[cfg(test)]
mod tests {
    use xmoq_pet_tracks::payloads::{Doing, Facing};

    use super::*;

    fn state() -> PetState {
        PetState {
            from: Place { x: 1, y: 2 },
            to: Place { x: 1, y: 2 },
            began: 5,
            arrives: 5,
            facing: Facing::Left,
            doing: Doing::Sit,
            mood: 3,
            bubble: None,
            by: None,
            at: 5,
        }
    }

    fn thing(id: u64, kind: ThingKind, by: &str) -> Thing {
        Thing {
            id,
            kind,
            at: Place { x: 9, y: 8 },
            by: by.to_string(),
        }
    }

    fn done(kind: EventKind, by: &str) -> Happening {
        Happening::Done(Event {
            id: 1,
            kind,
            by: by.to_string(),
            text: (kind == EventKind::Talk).then(|| "いい子だね".to_string()),
            at: 0,
        })
    }

    fn talk() -> Happening {
        done(EventKind::Talk, "alice")
    }

    /// (case, the floor, the batch, the questions asked, the reactions
    /// offered)
    type Asking<'a> = (
        &'a str,
        Vec<Thing>,
        Vec<Happening>,
        &'a [&'a str],
        &'a [&'a str],
    );

    #[test]
    fn the_request_asks_what_makes_sense_of_the_garden() {
        let ball = thing(1, ThingKind::Ball, "bob");
        let bowl = thing(2, ThingKind::Bowl, "ann");
        let cases: [Asking; 4] = [
            (
                "a pet, nothing on the floor",
                vec![],
                vec![done(EventKind::Pet, "alice")],
                &["reaction", "mood_change", "bubble"],
                &["approach", "flee", "ignore", "jump", "sleep", "sit"],
            ),
            (
                "a ball thrown and a talk",
                vec![ball.clone()],
                vec![Happening::Put(ball.clone()), talk()],
                &["reaction", "mood_change", "kind_words", "bubble"],
                &[
                    "approach", "flee", "ignore", "jump", "sleep", "sit", "chase",
                ],
            ),
            (
                "a bowl put down",
                vec![bowl.clone()],
                vec![Happening::Put(bowl.clone())],
                &["reaction", "mood_change", "bubble"],
                &["approach", "flee", "ignore", "jump", "sleep", "sit", "eat"],
            ),
            (
                "a call with a bowl put down before",
                vec![bowl.clone()],
                vec![done(EventKind::Call, "alice")],
                &["reaction", "mood_change", "bubble"],
                &["approach", "flee", "ignore", "jump", "sleep", "sit", "eat"],
            ),
        ];
        for (case, floor, batch, names, reactions) in cases {
            let body = request(&state(), Place { x: 7, y: 6 }, &floor, &batch);
            let questions = body["questions"].as_array().unwrap();
            let got: Vec<&str> = questions
                .iter()
                .map(|q| q["name"].as_str().unwrap())
                .collect();
            assert_eq!(got, names, "{case}");
            let offered: Vec<&str> = questions[0]["choices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|choice| choice["value"].as_str().unwrap())
                .collect();
            assert_eq!(offered, reactions, "{case}");
            assert_eq!(
                questions.last().unwrap()["choices"]
                    .as_array()
                    .unwrap()
                    .len(),
                16,
                "{case}"
            );
        }
    }

    #[test]
    fn with_nothing_new_the_request_says_so_and_offers_what_lies_on_the_floor() {
        let floor = [thing(2, ThingKind::Bowl, "ann")];
        let body = request(&state(), Place { x: 7, y: 6 }, &floor, &[]);
        assert_eq!(
            body["input"][0]["content"][0]["data"]["happened"],
            json!([])
        );
        let questions = body["questions"].as_array().unwrap();
        let names: Vec<&str> = questions
            .iter()
            .map(|q| q["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["reaction", "mood_change", "bubble"]);
        for question in questions {
            let instructions = question["instructions"].as_str().unwrap();
            assert!(instructions.contains(NOTHING_NEW), "{instructions}");
        }
        let offered: Vec<&str> = questions[0]["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|choice| choice["value"].as_str().unwrap())
            .collect();
        assert_eq!(
            offered,
            ["approach", "flee", "ignore", "jump", "sleep", "sit", "eat"]
        );
        // A batch of happenings is not told so.
        let body = request(
            &state(),
            Place { x: 7, y: 6 },
            &floor,
            &[done(EventKind::Call, "ann")],
        );
        assert!(
            !body["questions"][0]["instructions"]
                .as_str()
                .unwrap()
                .contains(NOTHING_NEW)
        );
        // Its answer is read as any, with no bubble.
        assert_eq!(
            read(200, &answers("eat", 2.0, None, "none"), &floor, &[]),
            Ok(decided(Reaction::Eat, 0, None))
        );
    }

    #[test]
    fn the_warm_up_asks_every_question_and_every_reaction() {
        let (floor, batch) = warm_up();
        let body = request(&state(), Place { x: 7, y: 6 }, &floor, &batch);
        assert_eq!(body["questions"].as_array().map(Vec::len), Some(4));
        assert_eq!(
            body["questions"][0]["choices"].as_array().map(Vec::len),
            Some(8)
        );
    }

    #[test]
    fn the_input_has_the_pet_the_floor_and_what_happened() {
        let ball = thing(1, ThingKind::Ball, "bob");
        let bowl = thing(2, ThingKind::Bowl, "ann");
        let body = request(
            &state(),
            Place { x: 7, y: 6 },
            &[ball.clone(), bowl.clone()],
            &[
                Happening::Put(bowl),
                Happening::Put(ball),
                done(EventKind::Call, "ann"),
                done(EventKind::Pet, "ann"),
                talk(),
            ],
        );
        assert_eq!(
            body["input"][0]["content"][0]["data"],
            json!({
                "pet": {"doing": "sit", "mood": 3, "where": [7, 6]},
                "floor": [
                    {"thing": "the ball", "at": [9, 8], "by": "bob"},
                    {"thing": "a bowl of food", "at": [9, 8], "by": "ann"},
                ],
                "happened": [
                    {"who": "ann", "did": "put a bowl of food down", "at": [9, 8]},
                    {"who": "bob", "did": "threw the ball", "to": [9, 8]},
                    {"who": "ann", "did": "called the pet"},
                    {"who": "ann", "did": "petted the pet"},
                    {"who": "alice", "did": "said", "said": "いい子だね"},
                ],
            })
        );
        assert_eq!(
            body["questions"][1]["levels"][4]["label"],
            json!("much better")
        );
        assert_eq!(
            body["questions"][0]["choices"][0],
            json!({"value": "approach", "description": Reaction::Approach.description()})
        );
    }

    fn answers(reaction: &str, score: f64, kind: Option<f64>, bubble: &str) -> Vec<u8> {
        let mut answers = vec![
            json!({"type": "choice", "name": "reaction", "choice": reaction, "confidence": 0.9, "probabilities": []}),
            json!({"type": "score", "name": "mood_change", "score": score, "confidence": 0.9, "probabilities": []}),
            json!({"type": "choice", "name": "bubble", "choice": bubble, "confidence": 0.9, "probabilities": []}),
        ];
        if let Some(probability) = kind {
            answers.push(
                json!({"type": "predicate", "name": "kind_words", "probability": probability}),
            );
        }
        serde_json::to_vec(&json!({"model": "m", "answers": answers, "usage": {}})).unwrap()
    }

    /// (case, the status, the body, the batch, what is read)
    type Reading<'a> = (
        &'a str,
        u16,
        Vec<u8>,
        &'a [Happening],
        Result<Decision, NotDecided>,
    );

    fn decided(reaction: Reaction, mood_change: i8, bubble: Option<&'static str>) -> Decision {
        Decision {
            reaction,
            mood_change,
            bubble,
        }
    }

    /// (case, the status, the body, the batch, the decision read)
    type Decided<'a> = (&'a str, u16, Vec<u8>, &'a [Happening], Decision);

    #[test]
    fn an_answer_is_read_as_a_decision() {
        let (pet, said) = ([done(EventKind::Pet, "alice")], [talk()]);
        let cases: [Decided; 8] = [
            (
                "calm",
                200,
                answers("sit", 2.0, None, "♪"),
                &pet,
                decided(Reaction::Sit, 0, Some("♪")),
            ),
            (
                "much better",
                200,
                answers("jump", 4.0, None, "もっと！"),
                &pet,
                decided(Reaction::Jump, 2, Some("もっと！")),
            ),
            (
                "much worse, no bubble",
                200,
                answers("flee", 0.0, None, "none"),
                &pet,
                decided(Reaction::Flee, -2, None),
            ),
            (
                "a score with a fraction",
                200,
                answers("sit", 2.900_000_000_000_000_4, None, "♪"),
                &pet,
                decided(Reaction::Sit, 1, Some("♪")),
            ),
            (
                "a half takes the lower level",
                200,
                answers("sit", 1.5, None, "♪"),
                &pet,
                decided(Reaction::Sit, -1, Some("♪")),
            ),
            (
                "kind words",
                200,
                answers("approach", 3.0, Some(0.5), "♥"),
                &said,
                decided(Reaction::Approach, 2, Some("♥")),
            ),
            (
                "unkind words",
                200,
                answers("flee", 1.0, Some(0.2), "こわい…"),
                &said,
                decided(Reaction::Flee, -2, Some("こわい…")),
            ),
            (
                "words neither",
                200,
                answers("sit", 2.0, Some(0.3), "?"),
                &said,
                decided(Reaction::Sit, 0, Some("?")),
            ),
        ];
        for (case, status, body, batch, expected) in cases {
            assert_eq!(read(status, &body, &[], batch), Ok(expected), "{case}");
        }
    }

    #[test]
    fn an_answer_that_gives_no_decision_says_why() {
        let refused = serde_json::to_vec(&json!({"answers": [
            {"type": "refusal", "name": "reaction"},
            {"type": "score", "name": "mood_change", "score": 2.0},
            {"type": "choice", "name": "bubble", "choice": "none"},
        ]}))
        .unwrap();
        let (pet, said) = ([done(EventKind::Pet, "alice")], [talk()]);
        let cases: [Reading; 6] = [
            (
                "words without their answer",
                200,
                answers("sit", 2.0, None, "?"),
                &said,
                Err(NotDecided::Answer),
            ),
            (
                "a reaction that makes no sense",
                200,
                answers("chase", 2.0, None, "♪"),
                &pet,
                Err(NotDecided::Answer),
            ),
            (
                "a bubble not of the list",
                200,
                answers("sit", 2.0, None, "わん"),
                &pet,
                Err(NotDecided::Answer),
            ),
            ("a refusal", 200, refused, &pet, Err(NotDecided::Refusal)),
            (
                "an error status",
                500,
                answers("sit", 2.0, None, "♪"),
                &pet,
                Err(NotDecided::Status),
            ),
            (
                "no JSON",
                200,
                b"<html>".to_vec(),
                &pet,
                Err(NotDecided::Body),
            ),
        ];
        for (case, status, body, batch, expected) in cases {
            assert_eq!(read(status, &body, &[], batch), expected, "{case}");
        }
    }

    #[test]
    fn eat_and_chase_are_read_where_the_floor_has_their_thing() {
        let ball = thing(1, ThingKind::Ball, "bob");
        let bowl = thing(2, ThingKind::Bowl, "ann");
        let cases: [(&str, Vec<Thing>, &str, Option<Reaction>); 4] = [
            (
                "chase with a ball",
                vec![ball.clone()],
                "chase",
                Some(Reaction::Chase),
            ),
            ("chase with a bowl alone", vec![bowl.clone()], "chase", None),
            (
                "eat with a bowl",
                vec![ball, bowl],
                "eat",
                Some(Reaction::Eat),
            ),
            ("eat with nothing", vec![], "eat", None),
        ];
        let batch = [done(EventKind::Call, "ann")];
        for (case, floor, reaction, expected) in cases {
            let got = read(200, &answers(reaction, 3.0, None, "!"), &floor, &batch)
                .map(|decision| decision.reaction);
            assert_eq!(got, expected.ok_or(NotDecided::Answer), "{case}");
        }
    }

    #[test]
    fn a_score_off_the_levels_is_no_answer() {
        let cases: [(f64, Option<i8>); 7] = [
            (-0.6, None),
            (4.6, None),
            (f64::NAN, None),
            (2.900_000_000_000_000_4, Some(1)),
            (4.1, Some(2)),
            (0.4, Some(-2)),
            (3.5, Some(1)),
        ];
        for (score, expected) in cases {
            assert_eq!(mood_change(score), expected, "{score}");
        }
    }
}
