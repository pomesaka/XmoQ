//! What an actor does, as it sends it on `acted`; the garden as it is now, as
//! the garden sends it on `garden`; and the pet's body, as the pet sends it
//! on `state`. The words of the three are in `fixtures/pet-tracks/README.md`,
//! whose tables `@xmoq/pet` reads too.

use serde_json::{Map, Value};

/// The largest coordinate of a [`Place`]: the floor is 0 to 1000 across and
/// into the room.
pub const FLOOR: u16 = 1000;

/// The most bytes of UTF-8 the `text` of a `talk` may have.
pub const TALK_BYTES: usize = 120;

/// The most bytes of UTF-8 a `bubble` may have.
pub const BUBBLE_BYTES: usize = 64;

/// The largest `mood`: 0 is cross and 4 glad.
pub const MOOD_MOST: u8 = 4;

/// The most things on the floor of a garden.
pub const MOST_THINGS: usize = 8;

/// The most events a garden holds.
pub const MOST_EVENTS: usize = 16;

/// The largest time, and the largest id, taken: 2^53 - 1, the largest
/// integer a JavaScript number holds exactly. WHY that bound: `@xmoq/pet`
/// reads the payloads with `JSON.parse`, and a larger value would reach it
/// as another number.
pub const TIME_MOST: u64 = (1 << 53) - 1;

/// A place on the floor, `[x,y]` in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Place {
    /// Across the room, 0 to [`FLOOR`].
    pub x: u16,
    /// Into the room, 0 to [`FLOOR`].
    pub y: u16,
}

/// What an actor does: `kind`, with what that kind has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    /// `pet`.
    Pet,
    /// `feed`, with where the bowl is put.
    Feed {
        /// `to`.
        to: Place,
    },
    /// `call`.
    Call,
    /// `throw`, with where the ball goes.
    Throw {
        /// `to`.
        to: Place,
    },
    /// `talk`, with what is said.
    Talk {
        /// `text`, 1 to [`TALK_BYTES`] bytes of UTF-8.
        text: String,
    },
    /// `eat`, the pet's: the bowl it eats from.
    Eat {
        /// `thing`, the id of the bowl.
        thing: u64,
    },
    /// `take`, the pet's: the ball it takes in its mouth.
    Take {
        /// `thing`, the id of the ball.
        thing: u64,
    },
}

impl ActionKind {
    /// The word of `kind`.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Self::Pet => "pet",
            Self::Feed { .. } => "feed",
            Self::Call => "call",
            Self::Throw { .. } => "throw",
            Self::Talk { .. } => "talk",
            Self::Eat { .. } => "eat",
            Self::Take { .. } => "take",
        }
    }
}

/// A payload of `acted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// `kind`, and `to`, `text` or `thing` where it has one.
    pub kind: ActionKind,
    /// `sentAt`, the sender's clock, which only its form is checked of.
    pub sent_at: u64,
}

/// Why a payload of `acted` is no action. The variants are the words of
/// `fixtures/pet-tracks/acted.tsv` ([`ActionError::word`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// Not UTF-8, not JSON, or no JSON object.
    NotAnObject,
    /// `kind` is missing or none of the kinds.
    Kind,
    /// `to` is missing on `feed` or `throw`, no place, or there on another
    /// kind.
    To,
    /// `text` is missing on `talk`, no string of 1 to [`TALK_BYTES`] bytes,
    /// or there on another kind.
    Text,
    /// `thing` is missing on `eat` or `take`, no id, or there on another kind.
    Thing,
    /// `sentAt` is missing, no integer, below 0 or above [`TIME_MOST`].
    SentAt,
}

impl ActionError {
    /// One word, as the table says it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotAnObject => "not-an-object",
            Self::Kind => "kind",
            Self::To => "to",
            Self::Text => "text",
            Self::Thing => "thing",
            Self::SentAt => "sent-at",
        }
    }
}

/// Reads a payload of `acted`. Members other than the ones of its kind and
/// `sentAt` are passed over, so that a sender may add some.
///
/// WHY `to`, `text` or `thing` on a kind that does not have it is refused and
/// not passed over: the sender meant something by it that this reader would
/// drop, such as a `pet` at a place.
///
/// # Errors
///
/// `kind` where it is wrong; then the member the kind has, where it is wrong;
/// then the first of `to`, `text` and `thing` that is there on a kind that
/// does not have it; then `sentAt`. `@xmoq/pet` checks in the same order.
pub fn read_action(payload: &[u8]) -> Result<Action, ActionError> {
    let members = members(payload).ok_or(ActionError::NotAnObject)?;
    let word = match members.get("kind") {
        Some(Value::String(word)) => word.as_str(),
        _ => return Err(ActionError::Kind),
    };
    let to = members.get("to");
    let text = members.get("text");
    let thing = members.get("thing");
    let kind = match word {
        "pet" => ActionKind::Pet,
        "feed" => ActionKind::Feed {
            to: to.and_then(place).ok_or(ActionError::To)?,
        },
        "call" => ActionKind::Call,
        "throw" => ActionKind::Throw {
            to: to.and_then(place).ok_or(ActionError::To)?,
        },
        "talk" => ActionKind::Talk {
            text: text
                .and_then(|text| bounded(text, TALK_BYTES))
                .ok_or(ActionError::Text)?,
        },
        "eat" => ActionKind::Eat {
            thing: thing.and_then(whole).ok_or(ActionError::Thing)?,
        },
        "take" => ActionKind::Take {
            thing: thing.and_then(whole).ok_or(ActionError::Thing)?,
        },
        _ => return Err(ActionError::Kind),
    };
    if !matches!(kind, ActionKind::Feed { .. } | ActionKind::Throw { .. }) && to.is_some() {
        return Err(ActionError::To);
    }
    if !matches!(kind, ActionKind::Talk { .. }) && text.is_some() {
        return Err(ActionError::Text);
    }
    if !matches!(kind, ActionKind::Eat { .. } | ActionKind::Take { .. }) && thing.is_some() {
        return Err(ActionError::Thing);
    }
    Ok(Action {
        kind,
        sent_at: time(&members, "sentAt").ok_or(ActionError::SentAt)?,
    })
}

/// The payload of `action` on `acted`: `kind`, then `to`, `text` or `thing`
/// where it has one, then `sentAt`.
///
/// WHY written by hand and not with a map of `serde_json`: the members are
/// in the order of the table, whatever the features of `serde_json` sort a
/// map by.
///
/// The caller keeps what [`read_action`] holds it to: a place within
/// [`FLOOR`], a text of 1 to [`TALK_BYTES`] bytes, and a `thing` and a
/// `sent_at` up to [`TIME_MOST`]. Written otherwise, the payload is no action
/// to a reader.
#[must_use]
pub fn action_payload(action: &Action) -> Vec<u8> {
    let kind = json_string(action.kind.word());
    let what = match &action.kind {
        ActionKind::Pet | ActionKind::Call => String::new(),
        ActionKind::Feed { to } | ActionKind::Throw { to } => {
            format!(r#","to":{}"#, place_json(*to))
        }
        ActionKind::Talk { text } => format!(r#","text":{}"#, json_string(text)),
        ActionKind::Eat { thing } | ActionKind::Take { thing } => {
            format!(r#","thing":{thing}"#)
        }
    };
    format!(r#"{{"kind":{kind}{what},"sentAt":{}}}"#, action.sent_at).into_bytes()
}

/// What lies on the floor of a garden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThingKind {
    /// `bowl`, of food, which `feed` puts down.
    Bowl,
    /// `ball`, which `throw` puts down.
    Ball,
}

impl ThingKind {
    const ALL: [Self; 2] = [Self::Bowl, Self::Ball];

    /// Its word in `kind`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Bowl => "bowl",
            Self::Ball => "ball",
        }
    }

    /// The kind whose word is `word`; `None` for any other.
    #[must_use]
    pub fn of(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.word() == word)
    }
}

/// A thing on the floor of a garden, `{"id","kind","at","by"}` in JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thing {
    /// `id`, up to [`TIME_MOST`], used once in a room.
    pub id: u64,
    /// `kind`.
    pub kind: ThingKind,
    /// `at`, where it lies.
    pub at: Place,
    /// `by`, the name of the actor that put it there, not empty.
    pub by: String,
}

/// What happened in a garden that leaves nothing on the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// `call`.
    Call,
    /// `pet`.
    Pet,
    /// `talk`, the one kind with a `text`.
    Talk,
}

impl EventKind {
    const ALL: [Self; 3] = [Self::Call, Self::Pet, Self::Talk];

    /// Its word in `kind`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Pet => "pet",
            Self::Talk => "talk",
        }
    }

    /// The kind whose word is `word`; `None` for any other.
    #[must_use]
    pub fn of(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.word() == word)
    }
}

/// An event of a garden, `{"id","kind","by","text","at"}` in JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// `id`, up to [`TIME_MOST`], used once in a room.
    pub id: u64,
    /// `kind`.
    pub kind: EventKind,
    /// `by`, the name of the actor that did it, not empty.
    pub by: String,
    /// `text`, the words of a [`EventKind::Talk`], 1 to [`TALK_BYTES`]
    /// bytes; `None` (`null`) for any other kind.
    pub text: Option<String>,
    /// `at`, the garden's clock when it took it.
    pub at: u64,
}

/// A payload of `garden`: the garden as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Garden {
    /// `things`, at most [`MOST_THINGS`].
    pub things: Vec<Thing>,
    /// `events`, the newest last, at most [`MOST_EVENTS`].
    pub events: Vec<Event>,
    /// `at`, the garden's clock when it wrote this.
    pub at: u64,
}

/// Why a payload of `garden` is no garden. The variants are the words of
/// `fixtures/pet-tracks/garden.tsv` ([`GardenError::word`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GardenError {
    /// Not UTF-8, not JSON, or no JSON object.
    NotAnObject,
    /// `things` is missing, no array, longer than [`MOST_THINGS`], or holds
    /// what is no thing.
    Things,
    /// `events` is missing, no array, longer than [`MOST_EVENTS`], or holds
    /// what is no event.
    Events,
    /// `at` is missing or no time.
    At,
}

impl GardenError {
    /// One word, as the table says it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotAnObject => "not-an-object",
            Self::Things => "things",
            Self::Events => "events",
            Self::At => "at",
        }
    }
}

/// Reads a payload of `garden`. Members other than the ones named, of the
/// garden, a thing or an event, are passed over, as in [`read_action`].
///
/// WHY one thing or event that is wrong makes the whole no garden, and is
/// not dropped alone: the garden writes all of it at once, so a wrong one is
/// a writer that does not follow the table. WHY the ids are not checked to
/// go up: the garden is the one that counts them, and a reader that knows an
/// event again by its id is not misled by an order it does not rely on.
///
/// # Errors
///
/// The first of `things`, `events` and `at` that is wrong.
pub fn read_garden(payload: &[u8]) -> Result<Garden, GardenError> {
    let members = members(payload).ok_or(GardenError::NotAnObject)?;
    let things = listed(&members, "things", MOST_THINGS, thing).ok_or(GardenError::Things)?;
    let events = listed(&members, "events", MOST_EVENTS, event).ok_or(GardenError::Events)?;
    let at = time(&members, "at").ok_or(GardenError::At)?;
    Ok(Garden { things, events, at })
}

/// The array `key` of at most `most` objects, each one `read`.
fn listed<T>(
    members: &Map<String, Value>,
    key: &str,
    most: usize,
    read: fn(&Map<String, Value>) -> Option<T>,
) -> Option<Vec<T>> {
    let items = members.get(key)?.as_array()?;
    if items.len() > most {
        return None;
    }
    items
        .iter()
        .map(|item| item.as_object().and_then(read))
        .collect()
}

fn thing(members: &Map<String, Value>) -> Option<Thing> {
    Some(Thing {
        id: members.get("id").and_then(whole)?,
        kind: word(members, "kind").and_then(ThingKind::of)?,
        at: members.get("at").and_then(place)?,
        by: members.get("by").and_then(|by| bounded(by, usize::MAX))?,
    })
}

/// WHY `text` must be there, as `null` on a kind other than `talk`: as
/// `bubble` of [`read_state`], the garden writes every member.
fn event(members: &Map<String, Value>) -> Option<Event> {
    let id = members.get("id").and_then(whole)?;
    let kind = word(members, "kind").and_then(EventKind::of)?;
    let by = members.get("by").and_then(|by| bounded(by, usize::MAX))?;
    let text = match (kind, members.get("text")?) {
        (EventKind::Talk, text) => Some(bounded(text, TALK_BYTES)?),
        (_, Value::Null) => None,
        _ => return None,
    };
    let at = time(members, "at")?;
    Some(Event {
        id,
        kind,
        by,
        text,
        at,
    })
}

/// The payload of `garden` on `garden`, its members in the order of the
/// table. WHY by hand: as [`action_payload`].
///
/// The caller keeps what [`read_garden`] holds it to: at most
/// [`MOST_THINGS`] things and [`MOST_EVENTS`] events, places within
/// [`FLOOR`], ids and times up to [`TIME_MOST`], names that are not empty
/// and texts of 1 to [`TALK_BYTES`] bytes on a talk alone. Written
/// otherwise, the payload is no garden to a reader.
#[must_use]
pub fn garden_payload(garden: &Garden) -> Vec<u8> {
    format!(
        r#"{{"things":{},"events":{},"at":{}}}"#,
        things_json(&garden.things),
        events_json(&garden.events),
        garden.at,
    )
    .into_bytes()
}

fn things_json(things: &[Thing]) -> String {
    let things: Vec<String> = things
        .iter()
        .map(|thing| {
            format!(
                r#"{{"id":{},"kind":{},"at":{},"by":{}}}"#,
                thing.id,
                json_string(thing.kind.word()),
                place_json(thing.at),
                json_string(&thing.by),
            )
        })
        .collect();
    format!("[{}]", things.join(","))
}

fn events_json(events: &[Event]) -> String {
    let events: Vec<String> = events
        .iter()
        .map(|event| {
            format!(
                r#"{{"id":{},"kind":{},"by":{},"text":{},"at":{}}}"#,
                event.id,
                json_string(event.kind.word()),
                json_string(&event.by),
                event
                    .text
                    .as_deref()
                    .map_or_else(|| "null".to_string(), json_string),
                event.at,
            )
        })
        .collect();
    format!("[{}]", events.join(","))
}

/// Which way the pet looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facing {
    /// `left`.
    Left,
    /// `right`.
    Right,
}

impl Facing {
    const ALL: [Self; 2] = [Self::Left, Self::Right];

    /// Its word in `facing`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// The facing whose word is `word`; `None` for any other.
    #[must_use]
    pub fn of(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|facing| facing.word() == word)
    }
}

/// What the pet does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Doing {
    /// `idle`.
    Idle,
    /// `walk`.
    Walk,
    /// `sit`.
    Sit,
    /// `sleep`.
    Sleep,
    /// `jump`.
    Jump,
    /// `eat`.
    Eat,
    /// `chase`.
    Chase,
    /// `flee`.
    Flee,
    /// `approach`.
    Approach,
}

impl Doing {
    const ALL: [Self; 9] = [
        Self::Idle,
        Self::Walk,
        Self::Sit,
        Self::Sleep,
        Self::Jump,
        Self::Eat,
        Self::Chase,
        Self::Flee,
        Self::Approach,
    ];

    /// Its word in `doing`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Sit => "sit",
            Self::Sleep => "sleep",
            Self::Jump => "jump",
            Self::Eat => "eat",
            Self::Chase => "chase",
            Self::Flee => "flee",
            Self::Approach => "approach",
        }
    }

    /// The doing whose word is `word`; `None` for any other.
    #[must_use]
    pub fn of(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|doing| doing.word() == word)
    }
}

/// A payload of `state`: the pet goes from `from` at `began` to `to` at
/// `arrives`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetState {
    /// `from`.
    pub from: Place,
    /// `to`; the same as `from` where the pet stays.
    pub to: Place,
    /// `began`, the pet's clock.
    pub began: u64,
    /// `arrives`, the pet's clock, not before `began`.
    pub arrives: u64,
    /// `facing`.
    pub facing: Facing,
    /// `doing`.
    pub doing: Doing,
    /// `mood`, 0 to [`MOOD_MOST`].
    pub mood: u8,
    /// `bubble`, what the pet says, 1 to [`BUBBLE_BYTES`] bytes; `None` for
    /// `null`.
    pub bubble: Option<String>,
    /// `by`, the name of whom the pet reacts to, not empty; `None` for
    /// `null`.
    pub by: Option<String>,
    /// `at`, the pet's clock when it wrote this.
    pub at: u64,
}

/// Why a payload of `state` is no state. The variants are the words of
/// `fixtures/pet-tracks/state.tsv` ([`StateError::word`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    /// Not UTF-8, not JSON, or no JSON object.
    NotAnObject,
    /// `from` is missing or no place.
    From,
    /// `to` is missing or no place.
    To,
    /// `began` is missing or no time.
    Began,
    /// `arrives` is missing, no time, or before `began`.
    Arrives,
    /// `facing` is missing or none of the facings.
    Facing,
    /// `doing` is missing or none of the doings.
    Doing,
    /// `mood` is missing, no integer, or above [`MOOD_MOST`].
    Mood,
    /// `bubble` is missing, or neither `null` nor a string of 1 to
    /// [`BUBBLE_BYTES`] bytes.
    Bubble,
    /// `by` is missing, or neither `null` nor a string that is not empty.
    By,
    /// `at` is missing or no time.
    At,
}

impl StateError {
    /// One word, as the table says it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotAnObject => "not-an-object",
            Self::From => "from",
            Self::To => "to",
            Self::Began => "began",
            Self::Arrives => "arrives",
            Self::Facing => "facing",
            Self::Doing => "doing",
            Self::Mood => "mood",
            Self::Bubble => "bubble",
            Self::By => "by",
            Self::At => "at",
        }
    }
}

/// Reads a payload of `state`. Members other than the ten are passed over,
/// as in [`read_action`].
///
/// WHY `bubble` and `by` must be there, as `null` where there is none: the
/// pet writes every member of each state, so a missing one is a writer that
/// does not follow the table, not a pet that says nothing.
///
/// # Errors
///
/// The first member, in the order of the table, that is wrong.
pub fn read_state(payload: &[u8]) -> Result<PetState, StateError> {
    let members = members(payload).ok_or(StateError::NotAnObject)?;
    let from = members
        .get("from")
        .and_then(place)
        .ok_or(StateError::From)?;
    let to = members.get("to").and_then(place).ok_or(StateError::To)?;
    let began = time(&members, "began").ok_or(StateError::Began)?;
    let arrives = time(&members, "arrives")
        .filter(|arrives| *arrives >= began)
        .ok_or(StateError::Arrives)?;
    let facing = word(&members, "facing")
        .and_then(Facing::of)
        .ok_or(StateError::Facing)?;
    let doing = word(&members, "doing")
        .and_then(Doing::of)
        .ok_or(StateError::Doing)?;
    let mood = members
        .get("mood")
        .and_then(Value::as_u64)
        .and_then(|mood| u8::try_from(mood).ok())
        .filter(|mood| *mood <= MOOD_MOST)
        .ok_or(StateError::Mood)?;
    let bubble = nullable(&members, "bubble", BUBBLE_BYTES, StateError::Bubble)?;
    let by = nullable(&members, "by", usize::MAX, StateError::By)?;
    let at = time(&members, "at").ok_or(StateError::At)?;
    Ok(PetState {
        from,
        to,
        began,
        arrives,
        facing,
        doing,
        mood,
        bubble,
        by,
        at,
    })
}

/// The payload of `state` on `state`, its members in the order of the table.
/// WHY by hand: as [`action_payload`].
///
/// The caller keeps what [`read_state`] holds it to: places within
/// [`FLOOR`], times up to [`TIME_MOST`] with `arrives` not before `began`,
/// `mood` up to [`MOOD_MOST`], a `bubble` of 1 to [`BUBBLE_BYTES`] bytes and
/// a `by` that is not empty. Written otherwise, the payload is no state to a
/// reader.
#[must_use]
pub fn state_payload(state: &PetState) -> Vec<u8> {
    let or_null = |text: &Option<String>| {
        text.as_deref()
            .map_or_else(|| "null".to_string(), json_string)
    };
    format!(
        r#"{{"from":{},"to":{},"began":{},"arrives":{},"facing":{},"doing":{},"mood":{},"bubble":{},"by":{},"at":{}}}"#,
        place_json(state.from),
        place_json(state.to),
        state.began,
        state.arrives,
        json_string(state.facing.word()),
        json_string(state.doing.word()),
        state.mood,
        or_null(&state.bubble),
        or_null(&state.by),
        state.at,
    )
    .into_bytes()
}

/// The members of a payload that is a JSON object; `None` for any other.
fn members(payload: &[u8]) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(payload) {
        Ok(Value::Object(members)) => Some(members),
        _ => None,
    }
}

/// A whole number from 0 to [`TIME_MOST`]: an id, or a time. `as_u64` is
/// `None` for a number written with a fraction or an exponent (`2.0` among
/// them), and for one below 0: neither is an id or a time in milliseconds.
fn whole(value: &Value) -> Option<u64> {
    value.as_u64().filter(|number| *number <= TIME_MOST)
}

/// The member `key` that is a time, [`whole`].
fn time(members: &Map<String, Value>, key: &str) -> Option<u64> {
    members.get(key).and_then(whole)
}

/// A member that is a string.
fn word<'a>(members: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    members.get(key).and_then(Value::as_str)
}

/// A string of 1 to `most` bytes.
fn bounded(value: &Value, most: usize) -> Option<String> {
    value
        .as_str()
        .filter(|text| (1..=most).contains(&text.len()))
        .map(str::to_string)
}

/// A member that is there, and is `null` (`Ok(None)`) or a string of 1 to
/// `most` bytes; `wrong` for any other.
fn nullable(
    members: &Map<String, Value>,
    key: &str,
    most: usize,
    wrong: StateError,
) -> Result<Option<String>, StateError> {
    match members.get(key) {
        Some(Value::Null) => Ok(None),
        Some(value) => bounded(value, most).map(Some).ok_or(wrong),
        None => Err(wrong),
    }
}

/// `[x,y]`, two whole numbers from 0 to [`FLOOR`].
fn place(value: &Value) -> Option<Place> {
    let [x, y] = value.as_array()?.as_slice() else {
        return None;
    };
    let coordinate = |value: &Value| {
        value
            .as_u64()
            .and_then(|number| u16::try_from(number).ok())
            .filter(|number| *number <= FLOOR)
    };
    Some(Place {
        x: coordinate(x)?,
        y: coordinate(y)?,
    })
}

fn place_json(place: Place) -> String {
    format!("[{},{}]", place.x, place.y)
}

/// A string as JSON writes it, with its quotes.
fn json_string(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column of the tables: `-` is none.
    fn column(text: &str) -> Option<&str> {
        (text != "-").then_some(text)
    }

    /// A text column: a JSON string literal.
    fn text_of(literal: &str) -> String {
        serde_json::from_str(literal).unwrap()
    }

    /// A place column: `[x,y]`.
    fn place_of(literal: &str) -> Place {
        let [x, y]: [u16; 2] = serde_json::from_str(literal).unwrap();
        Place { x, y }
    }

    /// The rows of a table, split at its tabs, each of `columns` columns.
    fn rows(table: &str, columns: usize) -> Vec<Vec<&str>> {
        assert!(table.ends_with('\n') && !table.ends_with("\n\n"));
        table
            .lines()
            .map(|row| {
                let row: Vec<&str> = row.split('\t').collect();
                assert_eq!(row.len(), columns, "{row:?}");
                row
            })
            .collect()
    }

    /// `fixtures/pet-tracks/acted.tsv`, which `@xmoq/pet` reads too.
    #[test]
    fn a_payload_of_acted_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/pet-tracks/acted.tsv");
        let mut read = 0;
        for row in rows(table, 9) {
            let [case, payload, kind, to, text, thing, sent_at, why, written] = row[..] else {
                unreachable!();
            };
            let got = read_action(payload.as_bytes());
            let expected = if let Some(word) = column(why) {
                assert_eq!(
                    [kind, to, text, thing, sent_at, written],
                    ["-"; 6],
                    "{case}"
                );
                Err(word)
            } else {
                let to = column(to).map(place_of);
                let text = column(text).map(text_of);
                let thing = column(thing).map(|thing| thing.parse::<u64>().unwrap());
                let kind = match (kind, to, text, thing) {
                    ("pet", None, None, None) => ActionKind::Pet,
                    ("feed", Some(to), None, None) => ActionKind::Feed { to },
                    ("call", None, None, None) => ActionKind::Call,
                    ("throw", Some(to), None, None) => ActionKind::Throw { to },
                    ("talk", None, Some(text), None) => ActionKind::Talk { text },
                    ("eat", None, None, Some(thing)) => ActionKind::Eat { thing },
                    ("take", None, None, Some(thing)) => ActionKind::Take { thing },
                    _ => panic!("no action: {case}"),
                };
                Ok(Action {
                    kind,
                    sent_at: sent_at.parse().unwrap(),
                })
            };
            assert_eq!(got.clone().map_err(ActionError::word), expected, "{case}");
            if let Ok(action) = got {
                assert_eq!(
                    action_payload(&action) == payload.as_bytes(),
                    written == "yes",
                    "{case}"
                );
            }
            read += 1;
        }
        // A table that lost its rows would pass.
        assert!(read > 0);
    }

    /// `fixtures/pet-tracks/garden.tsv`, which `@xmoq/pet` reads too. WHY the
    /// things and events read are compared as the writer writes them: the
    /// table holds them as the writer's compact JSON, and the JSON of each
    /// member is pinned by the payloads written back (column 7).
    #[test]
    fn a_payload_of_garden_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/pet-tracks/garden.tsv");
        let mut read = 0;
        for row in rows(table, 7) {
            let [case, payload, things, events, at, why, written] = row[..] else {
                unreachable!();
            };
            let got = read_garden(payload.as_bytes());
            match (column(why), &got) {
                (Some(word), _) => {
                    assert_eq!([things, events, at, written], ["-"; 4], "{case}");
                    assert_eq!(got.clone().map_err(GardenError::word), Err(word), "{case}");
                }
                (None, Ok(garden)) => {
                    assert_eq!(
                        [
                            things_json(&garden.things),
                            events_json(&garden.events),
                            garden.at.to_string()
                        ],
                        [things, events, at],
                        "{case}"
                    );
                    assert_eq!(
                        garden_payload(garden) == payload.as_bytes(),
                        written == "yes",
                        "{case}"
                    );
                }
                (None, Err(error)) => panic!("{case}: {error:?}"),
            }
            read += 1;
        }
        // A table that lost its rows would pass.
        assert!(read > 0);
    }

    /// `fixtures/pet-tracks/state.tsv`, which `@xmoq/pet` reads too.
    #[test]
    fn a_payload_of_state_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/pet-tracks/state.tsv");
        let mut read = 0;
        for row in rows(table, 14) {
            let [
                case,
                payload,
                from,
                to,
                began,
                arrives,
                facing,
                doing,
                mood,
                bubble,
                by,
                at,
                why,
                written,
            ] = row[..]
            else {
                unreachable!();
            };
            let got = read_state(payload.as_bytes());
            let expected = match column(why) {
                Some(word) => {
                    assert_eq!(
                        [
                            from, to, began, arrives, facing, doing, mood, bubble, by, at, written
                        ],
                        ["-"; 11],
                        "{case}"
                    );
                    Err(word)
                }
                None => Ok(PetState {
                    from: place_of(from),
                    to: place_of(to),
                    began: began.parse().unwrap(),
                    arrives: arrives.parse().unwrap(),
                    facing: Facing::of(facing).unwrap(),
                    doing: Doing::of(doing).unwrap(),
                    mood: mood.parse().unwrap(),
                    bubble: column(bubble).map(text_of),
                    by: column(by).map(text_of),
                    at: at.parse().unwrap(),
                }),
            };
            assert_eq!(got.clone().map_err(StateError::word), expected, "{case}");
            if let Ok(state) = got {
                assert_eq!(
                    state_payload(&state) == payload.as_bytes(),
                    written == "yes",
                    "{case}"
                );
            }
            read += 1;
        }
        // A table that lost its rows would pass.
        assert!(read > 0);
    }

    /// Bytes that are not UTF-8 and a control character as it is in a
    /// string, which the tables, being text, cannot hold.
    #[test]
    fn a_payload_the_tables_cannot_hold_is_nothing() {
        let cases: [(&str, &[u8]); 3] = [
            (
                "a byte order mark",
                b"\xef\xbb\xbf{\"kind\":\"pet\",\"sentAt\":5}",
            ),
            (
                "a tab in a string",
                b"{\"kind\":\"talk\",\"text\":\"a\tb\",\"sentAt\":5}",
            ),
            (
                "not UTF-8",
                b"{\"kind\":\"talk\",\"text\":\"\xff\",\"sentAt\":5}",
            ),
        ];
        for (case, payload) in cases {
            assert_eq!(
                read_action(payload).err(),
                Some(ActionError::NotAnObject),
                "{case}"
            );
            assert_eq!(
                read_state(payload).err(),
                Some(StateError::NotAnObject),
                "{case}"
            );
            assert_eq!(
                read_garden(payload).err(),
                Some(GardenError::NotAnObject),
                "{case}"
            );
        }
    }

    #[test]
    fn a_word_is_read_back_as_what_it_is_of() {
        for facing in Facing::ALL {
            assert_eq!(Facing::of(facing.word()), Some(facing));
        }
        for doing in Doing::ALL {
            assert_eq!(Doing::of(doing.word()), Some(doing));
        }
        for kind in ThingKind::ALL {
            assert_eq!(ThingKind::of(kind.word()), Some(kind));
        }
        for kind in EventKind::ALL {
            assert_eq!(EventKind::of(kind.word()), Some(kind));
        }
    }
}
