//! One pet: where it is, what it does, its mood, how it wanders when
//! nobody does anything, and what it does to the garden when it arrives
//! where it went for a bowl or the ball, and when it looks at the floor
//! again. Each method is given the time, in
//! milliseconds of the pet's clock since the epoch (the times of `state`);
//! none reads a clock.

use std::ops::RangeInclusive;

use xmoq_pet_tracks::payloads::{
    ActionKind, Doing, FLOOR, Facing, MOOD_MOST, PetState, Place, Thing, ThingKind,
};

use crate::chance::Chance;
use crate::decide::{Decision, Reaction, Upon};

/// Where the pet walks to when it comes to the people: the front middle of
/// the floor. WHY there for a call too: a person has no place on the floor
/// that the pet could walk to.
pub const FRONT_MIDDLE: Place = Place { x: 500, y: 150 };

/// The rug, across and into the room: where the pet sleeps.
pub const RUG: (RangeInclusive<u16>, RangeInclusive<u16>) = (120..=420, 520..=820);

/// Where a pet begins: the middle of the floor.
pub const START: Place = Place { x: 500, y: 500 };

/// How far the pet walks in a second, of the 1000 across the floor.
/// **Provisional.**
pub const WALK_PER_SECOND: u64 = 200;

/// How far it runs after a ball in a second. **Provisional.**
pub const RUN_PER_SECOND: u64 = 450;

/// How long after it arrived the pet picks a new place by itself, the
/// least and the most, at random between. **Provisional.**
pub const WANDER_MS: (u64, u64) = (3_000, 8_000);

/// Once in this many of its whims the pet sits instead of walking.
/// **Provisional.**
pub const SIT_ONE_IN: u64 = 4;

/// A pet nobody did anything to for this long goes to sleep on the rug.
/// **Provisional.**
pub const SLEEPY_AFTER_MS: u64 = 60_000;

/// A mood at or below this sends the pet to sleep at its next whim.
pub const SLEEPY_MOOD: u8 = 1;

/// The mood the pet drifts back to, one step every [`MOOD_DRIFT_MS`].
pub const CALM_MOOD: u8 = 2;

/// How often the mood takes a step back towards [`CALM_MOOD`].
/// **Provisional.**
pub const MOOD_DRIFT_MS: u64 = 30_000;

/// How long a bubble stays. **Provisional.**
pub const BUBBLE_MS: u64 = 4_000;

/// How long the pet eats from a bowl it arrived at, before it goes on.
/// **Provisional.**
pub const EAT_MS: u64 = 5_000;

/// How far the pet walks back from the front when it flees.
pub const FLEE_BY: u16 = 300;

/// What the pet says while it thinks over a batch.
pub const THINKING: &str = "?";

/// A pet and what it is about to do.
#[derive(Debug, Clone)]
pub struct Pet {
    state: PetState,
    chance: Chance,
    bubble_until: Option<u64>,
    next_whim_at: u64,
    next_drift_at: u64,
    last_acted_at: u64,
    /// Between [`Self::think`] and [`Self::react`]: no whim moves it.
    thinking: bool,
    /// The thing it goes to, to eat from or take once it arrives.
    errand: Option<Errand>,
    /// When it goes back to itself after something it did, and looks at
    /// the floor ([`Self::look`]); `None` where it does not.
    look_at: Option<u64>,
    /// The thing it ate from or took last. WHY kept: the garden takes it
    /// away once it is told on `acted`, which the floor the pet has seen
    /// does not show yet where the pet looks at once (after it took the
    /// ball); a pet that saw it there would go for it again. One is enough:
    /// it is the one the pet is at.
    acted_on: Option<u64>,
}

/// A thing of the garden the pet goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Errand {
    thing: u64,
    kind: ThingKind,
}

/// What the pet did on arriving where it went for a thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    /// What it does to the garden: `eat` for a bowl, `take` for a ball.
    pub action: ActionKind,
    /// Whether the thing is on the floor still: where it is not, the
    /// action is not sent, and the pet goes on.
    pub there: bool,
}

/// The thing of `kind` nearest to `here`; of two as near, the one put
/// first. WHY the nearest and not the one put last: where several bowls lie,
/// the pet eats them one after another ([`Pet::look`]), and the nearest
/// first is the shortest way round them, as a dog would go; the one put
/// last was what the batch's newest happening was about, which is no
/// reason once there are several. **Provisional.**
fn nearest_of(floor: &[Thing], kind: ThingKind, here: Place) -> Option<&Thing> {
    floor
        .iter()
        .filter(|thing| thing.kind == kind)
        .min_by_key(|thing| (walk_ms(here, thing.at, 1_000), thing.id))
}

/// The place a coordinate takes along a line, `done` of `whole`.
fn along(from: u16, to: u16, done: u64, whole: u64) -> u16 {
    let (from, to) = (i64::from(from), i64::from(to));
    let (done, whole) = (
        i64::try_from(done).unwrap_or(i64::MAX),
        i64::try_from(whole).unwrap_or(i64::MAX).max(1),
    );
    let at = from + (to - from) * done / whole;
    u16::try_from(at.clamp(0, i64::from(FLOOR))).unwrap_or(FLOOR)
}

/// How long a way of `from` to `to` takes at `per_second`.
fn walk_ms(from: Place, to: Place, per_second: u64) -> u64 {
    let dx = u64::from(from.x.abs_diff(to.x));
    let dy = u64::from(from.y.abs_diff(to.y));
    (dx * dx + dy * dy).isqrt() * 1_000 / per_second.max(1)
}

impl Pet {
    /// A pet that begins at `now` in the middle of the floor, calm, with
    /// the whims of `seed`.
    #[must_use]
    pub fn new(seed: u64, now: u64) -> Self {
        let mut chance = Chance::new(seed);
        let next_whim_at = now.saturating_add(chance.within(WANDER_MS.0, WANDER_MS.1));
        Self {
            state: PetState {
                from: START,
                to: START,
                began: now,
                arrives: now,
                facing: Facing::Right,
                doing: Doing::Idle,
                mood: CALM_MOOD,
                bubble: None,
                by: None,
                at: now,
            },
            chance,
            bubble_until: None,
            next_whim_at,
            next_drift_at: now.saturating_add(MOOD_DRIFT_MS),
            last_acted_at: now,
            thinking: false,
            errand: None,
            look_at: None,
            acted_on: None,
        }
    }

    /// What the pet does, as it was written last.
    #[must_use]
    pub fn state(&self) -> &PetState {
        &self.state
    }

    /// When [`Self::arrive`] or [`Self::go_on`] has something to do next.
    #[must_use]
    pub fn next_at(&self) -> u64 {
        [
            Some(self.next_whim_at),
            Some(self.next_drift_at),
            self.bubble_until,
            self.errand.map(|_| self.state.arrives),
            self.look_at,
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(self.next_whim_at)
    }

    /// Where the pet is at `now`, on its way from `from` to `to`.
    #[must_use]
    pub fn place_at(&self, now: u64) -> Place {
        let state = &self.state;
        if now >= state.arrives {
            return state.to;
        }
        let done = now.saturating_sub(state.began);
        let whole = state.arrives - state.began;
        Place {
            x: along(state.from.x, state.to.x, done, whole),
            y: along(state.from.y, state.to.y, done, whole),
        }
    }

    /// Goes from where it is at `now` to `to`, doing `doing`.
    fn go(&mut self, to: Place, doing: Doing, per_second: u64, now: u64) {
        let from = self.place_at(now);
        let state = &mut self.state;
        state.facing = match to.x.cmp(&from.x) {
            std::cmp::Ordering::Greater => Facing::Right,
            std::cmp::Ordering::Less => Facing::Left,
            std::cmp::Ordering::Equal => state.facing,
        };
        state.from = from;
        state.to = to;
        state.began = now;
        state.arrives = now.saturating_add(walk_ms(from, to, per_second));
        state.doing = doing;
    }

    /// Stays where it is at `now`, doing `doing`.
    fn stay(&mut self, doing: Doing, now: u64) {
        let here = self.place_at(now);
        self.go(here, doing, WALK_PER_SECOND, now);
    }

    /// A place on the rug.
    fn on_the_rug(&mut self) -> Place {
        let (across, into) = RUG;
        Place {
            x: self.pick(across),
            y: self.pick(into),
        }
    }

    fn pick(&mut self, range: RangeInclusive<u16>) -> u16 {
        let number = self
            .chance
            .within(u64::from(*range.start()), u64::from(*range.end()));
        u16::try_from(number).unwrap_or(*range.start())
    }

    /// The time of the next whim: a while after it arrives where it goes,
    /// or after `now` where it is there already. WHY not from `arrives`
    /// alone: a pet that stays (asleep, thinking) arrived long ago, and its
    /// whims would all be due at once, again and again.
    fn whim_after_arriving(&mut self, now: u64) {
        let after = self.chance.within(WANDER_MS.0, WANDER_MS.1);
        self.next_whim_at = self.state.arrives.max(now).saturating_add(after);
    }

    /// What the pet does by itself up to `now`: its bubble goes, its mood
    /// drifts, and at its whim it walks, sits or sleeps. Whether its state
    /// changed; where it did, the state was written at `now`.
    pub fn go_on(&mut self, now: u64) -> bool {
        let mut changed = false;
        if self.bubble_until.is_some_and(|until| until <= now) {
            self.bubble_until = None;
            self.state.bubble = None;
            changed = true;
        }
        if self.next_drift_at <= now {
            let mood = self.state.mood;
            self.state.mood = match mood.cmp(&CALM_MOOD) {
                std::cmp::Ordering::Less => mood + 1,
                std::cmp::Ordering::Greater => mood - 1,
                std::cmp::Ordering::Equal => mood,
            };
            changed |= self.state.mood != mood;
            // WHY from now and not from the last step: a pet whose room was
            // away for an hour takes one step, not a hundred at once.
            self.next_drift_at = now.saturating_add(MOOD_DRIFT_MS);
        }
        if self.next_whim_at <= now {
            changed |= self.whim(now);
        }
        if changed {
            self.state.at = now;
        }
        changed
    }

    /// The pet does what it likes. Whether its state changed.
    fn whim(&mut self, now: u64) -> bool {
        if self.thinking {
            self.whim_after_arriving(now);
            return false;
        }
        let sleepy = self.state.mood <= SLEEPY_MOOD
            || now.saturating_sub(self.last_acted_at) >= SLEEPY_AFTER_MS;
        let changed = if sleepy && self.state.doing == Doing::Sleep {
            false
        } else if sleepy {
            let rug = self.on_the_rug();
            self.go(rug, Doing::Sleep, WALK_PER_SECOND, now);
            true
        } else if self.chance.one_in(SIT_ONE_IN) {
            self.stay(Doing::Sit, now);
            true
        } else {
            let to = Place {
                x: self.pick(0..=FLOOR),
                y: self.pick(0..=FLOOR),
            };
            self.go(to, Doing::Walk, WALK_PER_SECOND, now);
            true
        };
        if changed {
            // What it reacted to is over.
            self.state.by = None;
        }
        self.whim_after_arriving(now);
        changed
    }

    /// The things of `floor` the pet has not acted on: what it can go to.
    #[must_use]
    pub fn untouched(&self, floor: &[Thing]) -> Vec<Thing> {
        floor
            .iter()
            .filter(|thing| Some(thing.id) != self.acted_on)
            .cloned()
            .collect()
    }

    /// Whether the pet goes back to itself at `now` and looks at `floor`,
    /// the things on the floor of the garden, for something to do: where it
    /// finished eating, took the ball, or would wander again after a
    /// reaction to a batch, and something it has not acted on lies there.
    /// Its decision is then made upon the floor ([`Upon::Floor`]). The look
    /// is used up, whether or not there is something.
    ///
    /// WHY it looks: what stays on the floor is no happening
    /// ([`crate::perceive`]), so a pet that decided upon happenings alone
    /// ate one of three bowls put down at once and left two. WHY not a
    /// loop: it looks once for each such end, and a decision upon the floor
    /// makes it look again only through eating a bowl or taking the ball,
    /// each of which takes a thing off the floor ([`Self::react`]).
    pub fn look(&mut self, now: u64, floor: &[Thing]) -> bool {
        if self.look_at.is_none_or(|at| at > now) {
            return false;
        }
        self.look_at = None;
        !self.thinking && !self.untouched(floor).is_empty()
    }

    /// The pet thinks over a batch at `now`, while its decision is asked
    /// for: it stops where it is and sits, with [`THINKING`], until
    /// [`Self::react`]; `by` is the actor of the newest happening of the
    /// batch. WHY: a decision of the API takes seconds (3 to 4 on the whole,
    /// up to 7, the PM's runs of 2026-10-28), and a pet that walked on
    /// meanwhile would look as if it did not hear. Its facing stays: `state`
    /// has left and right alone, and no word for the front. What it went to
    /// it goes to no more.
    pub fn think(&mut self, by: Option<&str>, now: u64) {
        self.stay(Doing::Sit, now);
        self.state.bubble = Some(THINKING.to_string());
        self.state.by = by.map(str::to_string);
        self.bubble_until = None;
        self.thinking = true;
        self.errand = None;
        self.look_at = None;
        self.state.at = now;
    }

    /// The pet reacts as `decision` says, made `upon` a batch or the floor,
    /// at `now`, with `floor` the things on the floor of the garden. `by` is
    /// the actor of the newest happening of the batch: the one the decision
    /// is of, as [`crate::decide::fixed`] takes it; `None` upon the floor.
    ///
    /// For `eat` it walks to the nearest bowl, and for `chase` runs to the
    /// ball, and does to it what [`Self::arrive`] says once there. WHY it
    /// sits where it is where there is no such thing: the API is offered
    /// `eat` and `chase` only with one on the floor
    /// ([`crate::decide::sensible`]), and the table only for a thing put
    /// down or lying there; the thing can have gone between, and a pet that
    /// walked nowhere would look stuck.
    ///
    /// After `approach`, `flee`, `jump` and `sit` upon a batch, it looks at
    /// the floor at its next whim ([`Self::look`]); after `eat` and `chase`,
    /// once it arrived. After `ignore` and `sleep` it does not, nor after
    /// anything but `eat` and `chase` upon the floor: WHY, a decision that
    /// took nothing off the floor would come again at every whim, a call of
    /// the API each. **Provisional**, the user's choice of 2026-10-28.
    pub fn react(
        &mut self,
        decision: &Decision,
        by: Option<&str>,
        floor: &[Thing],
        now: u64,
        upon: Upon,
    ) {
        let mood = i16::from(self.state.mood) + i16::from(decision.mood_change);
        self.state.mood = u8::try_from(mood.clamp(0, i16::from(MOOD_MOST))).unwrap_or(CALM_MOOD);
        self.state.bubble = decision.bubble.map(str::to_string);
        self.bubble_until = decision.bubble.map(|_| now.saturating_add(BUBBLE_MS));
        self.state.by = by.map(str::to_string);
        self.last_acted_at = now;
        self.thinking = false;
        self.errand = None;
        let here = self.place_at(now);
        match decision.reaction {
            Reaction::Approach => self.go(FRONT_MIDDLE, Doing::Approach, WALK_PER_SECOND, now),
            Reaction::Chase | Reaction::Eat => {
                let (kind, doing, speed) = if decision.reaction == Reaction::Eat {
                    (ThingKind::Bowl, Doing::Eat, WALK_PER_SECOND)
                } else {
                    (ThingKind::Ball, Doing::Chase, RUN_PER_SECOND)
                };
                match nearest_of(&self.untouched(floor), kind, here).cloned() {
                    Some(thing) => {
                        self.errand = Some(Errand {
                            thing: thing.id,
                            kind,
                        });
                        self.go(thing.at, doing, speed, now);
                    }
                    None => self.stay(Doing::Sit, now),
                }
            }
            Reaction::Flee => {
                let away = Place {
                    x: here.x,
                    y: here.y.saturating_add(FLEE_BY).min(FLOOR),
                };
                self.go(away, Doing::Flee, WALK_PER_SECOND, now);
            }
            Reaction::Sleep => {
                let rug = self.on_the_rug();
                self.go(rug, Doing::Sleep, WALK_PER_SECOND, now);
            }
            Reaction::Jump => self.stay(Doing::Jump, now),
            Reaction::Sit => self.stay(Doing::Sit, now),
            Reaction::Ignore => {}
        }
        self.whim_after_arriving(now);
        self.look_at = match (decision.reaction, upon) {
            // WHY sit as well: a talk is answered by sitting a while, after
            // which a bowl that waits on the floor is to be eaten all the same.
            // WHY NOT ignore and sleep: the pet chose to let what is there be.
            (
                Reaction::Approach | Reaction::Flee | Reaction::Jump | Reaction::Sit,
                Upon::Happened,
            ) => Some(self.next_whim_at),
            _ => None,
        };
        self.state.at = now;
    }

    /// Where the pet arrives at `now` where it went for a thing: what it
    /// does to the garden, with `floor` the things on the floor of the
    /// garden. At a bowl it eats for [`EAT_MS`] and then looks at the
    /// floor; at the ball it takes it, looks at the floor at once, and
    /// goes on as by itself. Where the thing is gone, it stands there, and
    /// looks at its next whim. `None` where it went for nothing, or is not
    /// there yet; otherwise its state was written at `now`.
    pub fn arrive(&mut self, now: u64, floor: &[Thing]) -> Option<Arrival> {
        let errand = self.errand.filter(|_| now >= self.state.arrives)?;
        self.errand = None;
        let there = floor.iter().any(|thing| thing.id == errand.thing);
        let action = match errand.kind {
            ThingKind::Bowl => ActionKind::Eat {
                thing: errand.thing,
            },
            ThingKind::Ball => ActionKind::Take {
                thing: errand.thing,
            },
        };
        if there {
            self.acted_on = Some(errand.thing);
        }
        if there && errand.kind == ThingKind::Bowl {
            self.stay(Doing::Eat, now);
            self.next_whim_at = now.saturating_add(EAT_MS);
            self.look_at = Some(self.next_whim_at);
        } else {
            self.stay(Doing::Idle, now);
            self.whim_after_arriving(now);
            self.look_at = Some(if there { now } else { self.next_whim_at });
        }
        self.state.at = now;
        Some(Arrival { action, there })
    }
}

#[cfg(test)]
mod tests {
    use xmoq_pet_tracks::payloads::{read_state, state_payload};

    use super::*;
    use crate::decide::table;

    const NOW: u64 = 1_000_000;

    fn decision(reaction: Reaction, mood_change: i8, bubble: Option<&'static str>) -> Decision {
        Decision {
            reaction,
            mood_change,
            bubble,
        }
    }

    fn thing(id: u64, kind: ThingKind, at: Place) -> Thing {
        Thing {
            id,
            kind,
            at,
            by: "ann".to_string(),
        }
    }

    fn in_the_rug(place: Place) -> bool {
        RUG.0.contains(&place.x) && RUG.1.contains(&place.y)
    }

    /// (case, the reaction, the floor, what it does, where it goes: `None`
    /// for the rug)
    type Reacting = (&'static str, Reaction, Vec<Thing>, Doing, Option<Place>);

    fn reacting() -> [Reacting; 10] {
        let ball = Place { x: 900, y: 50 };
        let bowl = Place { x: 100, y: 500 };
        let floor = vec![
            thing(1, ThingKind::Ball, Place { x: 1, y: 1 }),
            thing(4, ThingKind::Ball, ball),
            thing(2, ThingKind::Bowl, Place { x: 2, y: 2 }),
            thing(3, ThingKind::Bowl, bowl),
        ];
        let cases: [Reacting; 10] = [
            (
                "approach",
                Reaction::Approach,
                vec![],
                Doing::Approach,
                Some(FRONT_MIDDLE),
            ),
            (
                "chase the nearest ball",
                Reaction::Chase,
                floor.clone(),
                Doing::Chase,
                Some(ball),
            ),
            (
                "chase without a ball",
                Reaction::Chase,
                vec![thing(3, ThingKind::Bowl, bowl)],
                Doing::Sit,
                Some(START),
            ),
            (
                "eat from the nearest bowl",
                Reaction::Eat,
                floor.clone(),
                Doing::Eat,
                Some(bowl),
            ),
            (
                "eat without a bowl",
                Reaction::Eat,
                vec![],
                Doing::Sit,
                Some(START),
            ),
            (
                "flee from the front",
                Reaction::Flee,
                vec![],
                Doing::Flee,
                Some(Place { x: 500, y: 800 }),
            ),
            (
                "jump in place",
                Reaction::Jump,
                vec![],
                Doing::Jump,
                Some(START),
            ),
            (
                "sit in place",
                Reaction::Sit,
                vec![],
                Doing::Sit,
                Some(START),
            ),
            (
                "sleep on the rug",
                Reaction::Sleep,
                vec![],
                Doing::Sleep,
                None,
            ),
            (
                "ignore keeps on",
                Reaction::Ignore,
                vec![],
                Doing::Idle,
                Some(START),
            ),
        ];
        cases
    }

    #[test]
    fn a_reaction_becomes_a_state() {
        for (case, reaction, floor, doing, to) in reacting() {
            let mut pet = Pet::new(3, NOW);
            pet.react(
                &decision(reaction, 0, Some("!")),
                Some("bob"),
                &floor,
                NOW + 10,
                Upon::Happened,
            );
            let state = pet.state().clone();
            assert_eq!(state.doing, doing, "{case}");
            assert_eq!(state.from, START, "{case}");
            match to {
                Some(to) => assert_eq!(state.to, to, "{case}"),
                None => assert!(in_the_rug(state.to), "{case}: {:?}", state.to),
            }
            assert_eq!(
                (state.by.as_deref(), state.bubble.as_deref(), state.at),
                (Some("bob"), Some("!"), NOW + 10),
                "{case}"
            );
            // Whatever it does is a state a reader takes.
            assert_eq!(read_state(&state_payload(&state)), Ok(state), "{case}");
        }
    }

    /// (case, the reaction, the floor when it arrives, what it does to the
    /// garden, whether the thing is there, what it does after, until when)
    type Arriving = (&'static str, Reaction, Vec<Thing>, ActionKind, bool, Doing);

    #[test]
    fn a_pet_that_arrives_at_a_thing_eats_it_or_takes_it() {
        let at = Place { x: 500, y: 300 };
        let bowl = thing(7, ThingKind::Bowl, at);
        let ball = thing(8, ThingKind::Ball, at);
        let cases: [Arriving; 4] = [
            (
                "a bowl is eaten from",
                Reaction::Eat,
                vec![bowl.clone()],
                ActionKind::Eat { thing: 7 },
                true,
                Doing::Eat,
            ),
            (
                "the ball is taken",
                Reaction::Chase,
                vec![ball.clone()],
                ActionKind::Take { thing: 8 },
                true,
                Doing::Idle,
            ),
            (
                "a bowl that is gone",
                Reaction::Eat,
                vec![ball.clone()],
                ActionKind::Eat { thing: 7 },
                false,
                Doing::Idle,
            ),
            (
                "a ball that is gone",
                Reaction::Chase,
                vec![],
                ActionKind::Take { thing: 8 },
                false,
                Doing::Idle,
            ),
        ];
        for (case, reaction, floor, action, there, doing) in cases {
            let mut pet = Pet::new(3, NOW);
            pet.react(
                &decision(reaction, 0, None),
                Some("ann"),
                &[bowl.clone(), ball.clone()],
                NOW,
                Upon::Happened,
            );
            let arrives = pet.state().arrives;
            assert!(arrives > NOW, "{case}");
            assert_eq!(pet.next_at(), arrives, "{case}: due when it arrives");
            assert_eq!(pet.arrive(arrives - 1, &floor), None, "{case}: on its way");
            assert_eq!(
                pet.arrive(arrives, &floor),
                Some(Arrival { action, there }),
                "{case}"
            );
            let state = pet.state();
            assert_eq!(
                (state.doing, state.from, state.to, state.at),
                (doing, at, at, arrives),
                "{case}"
            );
            assert_eq!(pet.arrive(arrives + 1, &floor), None, "{case}: once");
            // Eating, no whim comes before it has eaten.
            if doing == Doing::Eat {
                assert_eq!(pet.next_whim_at, arrives + EAT_MS, "{case}");
            }
            // Where it took the ball it looks at once, and finds nothing
            // but the ball it took, which the garden has not taken away.
            assert!(!pet.look(arrives, &floor), "{case}");
            assert!(pet.next_at() > arrives, "{case}");
        }
    }

    #[test]
    fn what_the_pet_goes_to_it_goes_to_no_more_once_it_thinks_or_reacts_again() {
        let bowl = thing(7, ThingKind::Bowl, Place { x: 500, y: 300 });
        let floor = [bowl];
        let mut pet = Pet::new(3, NOW);
        pet.react(
            &decision(Reaction::Eat, 0, None),
            None,
            &floor,
            NOW,
            Upon::Happened,
        );
        pet.think(Some("bob"), NOW + 10);
        assert_eq!(pet.arrive(NOW + 60_000, &floor), None, "thinking");
        pet.react(
            &decision(Reaction::Eat, 0, None),
            None,
            &floor,
            NOW + 20,
            Upon::Happened,
        );
        pet.react(
            &decision(Reaction::Jump, 0, None),
            None,
            &floor,
            NOW + 30,
            Upon::Happened,
        );
        assert_eq!(pet.arrive(NOW + 60_000, &floor), None, "jumping");
    }

    #[test]
    fn the_way_takes_its_distance_at_the_speed() {
        let mut pet = Pet::new(3, NOW);
        // From (500, 500) to (500, 150): 350 at 200 a second.
        pet.react(
            &decision(Reaction::Approach, 0, None),
            Some("a"),
            &[],
            NOW,
            Upon::Happened,
        );
        assert_eq!(pet.state().arrives, NOW + 1_750);
        assert_eq!(
            pet.state().facing,
            Facing::Right,
            "no way across keeps the facing"
        );
        assert_eq!(pet.place_at(NOW + 875), Place { x: 500, y: 325 });
        assert_eq!(pet.place_at(NOW + 5_000), FRONT_MIDDLE);
        // A reaction under way begins where the pet is.
        pet.react(
            &decision(Reaction::Jump, 0, None),
            Some("a"),
            &[],
            NOW + 875,
            Upon::Happened,
        );
        assert_eq!(
            (pet.state().from, pet.state().to),
            (Place { x: 500, y: 325 }, Place { x: 500, y: 325 })
        );
        assert_eq!(pet.state().began, pet.state().arrives);
    }

    #[test]
    fn a_pet_that_thinks_stops_and_sits_until_it_reacts() {
        let mut pet = Pet::new(3, NOW);
        pet.react(
            &decision(Reaction::Approach, 0, None),
            Some("a"),
            &[],
            NOW,
            Upon::Happened,
        );
        pet.think(Some("b"), NOW + 875);
        let state = pet.state().clone();
        assert_eq!(
            (state.from, state.to),
            (Place { x: 500, y: 325 }, Place { x: 500, y: 325 })
        );
        assert_eq!(
            (state.doing, state.bubble.as_deref(), state.by.as_deref()),
            (Doing::Sit, Some(THINKING), Some("b"))
        );
        assert_eq!(state.at, NOW + 875);
        // No whim moves it, however long the answer takes; the mood drifts.
        let mut now = NOW + 875;
        while now < NOW + 3 * WANDER_MS.1 {
            now = pet.next_at();
            pet.go_on(now);
            assert!(pet.next_at() > now, "nothing stays due at {now}");
            assert_eq!(
                (pet.state().doing, pet.state().to),
                (Doing::Sit, state.to),
                "at {now}"
            );
            assert_eq!(pet.state().bubble.as_deref(), Some(THINKING));
        }
        pet.react(
            &decision(Reaction::Ignore, 0, Some("♥")),
            Some("b"),
            &[],
            now,
            Upon::Happened,
        );
        assert_eq!(pet.state().bubble.as_deref(), Some("♥"));
        assert!(pet.next_at() <= now + WANDER_MS.1, "its whims come again");
        let whim = pet.next_whim_at;
        assert!(pet.go_on(whim));
    }

    #[test]
    fn the_mood_changes_within_its_bounds() {
        let cases: [(&str, i8, u8); 5] = [
            ("none", 0, 2),
            ("better", 1, 3),
            ("much better and kind", 3, 4),
            ("worse", -1, 1),
            ("much worse and unkind", -3, 0),
        ];
        for (case, change, mood) in cases {
            let mut pet = Pet::new(3, NOW);
            pet.react(
                &decision(Reaction::Ignore, change, None),
                Some("a"),
                &[],
                NOW,
                Upon::Happened,
            );
            assert_eq!(pet.state().mood, mood, "{case}");
        }
    }

    #[test]
    fn by_itself_the_pet_lets_its_bubble_go_drifts_and_sleeps() {
        let mut pet = Pet::new(3, NOW);
        pet.react(
            &decision(Reaction::Sit, -2, Some("こわい…")),
            Some("a"),
            &[],
            NOW,
            Upon::Happened,
        );
        assert_eq!(pet.state().mood, 0);
        assert!(!pet.go_on(NOW + 1), "nothing is due yet");
        // Cross, it sleeps on the rug at its next whim, and its bubble goes.
        while pet.state().doing != Doing::Sleep || pet.state().bubble.is_some() {
            let now = pet.next_at();
            assert!(now <= NOW + WANDER_MS.1, "{now}");
            assert!(pet.go_on(now));
            assert_eq!(pet.state().at, now);
        }
        assert!(in_the_rug(pet.state().to));
        assert_eq!(pet.state().by, None, "what it reacted to is over");
        // Asleep and still cross: its whims change nothing.
        while pet.next_at() < NOW + MOOD_DRIFT_MS {
            let now = pet.next_at();
            // As the session does after each step: with nothing on the floor
            // a look there is spent and decides nothing.
            assert!(!pet.look(now, &[]));
            assert!(!pet.go_on(now));
            assert!(pet.next_at() > now, "nothing stays due at {now}");
        }
        // The mood drifts back a step at a time.
        assert!(pet.go_on(NOW + MOOD_DRIFT_MS));
        assert_eq!(pet.state().mood, 1);
    }

    #[test]
    fn a_pet_nobody_minds_falls_asleep_and_one_that_is_minded_wanders() {
        let mut pet = Pet::new(11, NOW);
        let mut now = NOW;
        let mut doings = Vec::new();
        while now < NOW + SLEEPY_AFTER_MS + WANDER_MS.1 * 3 {
            now = pet.next_at();
            let changed = pet.go_on(now);
            assert!(pet.next_at() > now, "nothing stays due at {now}");
            if changed {
                doings.push(pet.state().doing);
                let state = pet.state();
                assert!(state.to.x <= FLOOR && state.to.y <= FLOOR);
                assert!(state.arrives >= state.began);
            }
        }
        assert_eq!(doings.last(), Some(&Doing::Sleep), "{doings:?}");
        assert!(doings.contains(&Doing::Walk), "{doings:?}");
        // Asleep, it stays so.
        let before = pet.state().clone();
        for _ in 0..5 {
            let at = pet.next_whim_at;
            pet.go_on(at);
        }
        assert_eq!(pet.state().doing, Doing::Sleep);
        assert_eq!(pet.state().to, before.to);
    }

    /// What the pet did to the garden, and what it decided upon the floor.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Did {
        Looked(Reaction),
        Ate(u64),
        Took(u64),
    }

    /// What decides upon the floor: the table, or an API that says this.
    #[derive(Clone, Copy)]
    enum Decides {
        Table,
        Says(Reaction),
    }

    enum Step {
        /// A decision upon a batch of happenings by "ann".
        Reacts(Reaction),
        /// The pet lives for this long: what it does to the garden and
        /// decides upon the floor.
        Lives(u64, &'static [Did]),
    }
    use Did::{Ate, Looked, Took};
    use Step::{Lives, Reacts};

    /// The pet lives from `now` to `until` in a garden of `floor`, as the
    /// session drives it ([`crate::decide::table`] where the table
    /// decides): what it arrives at, then whether it looks, then what it
    /// does by itself. The garden takes a thing away when the pet eats it
    /// or takes it where `takes_away`, and never otherwise (a garden that
    /// is slow to). What it did, and the time it got to.
    fn live(
        pet: &mut Pet,
        floor: &mut Vec<Thing>,
        (decides, takes_away): (Decides, bool),
        mut now: u64,
        until: u64,
    ) -> (Vec<Did>, u64) {
        let mut did = Vec::new();
        for _ in 0..10_000 {
            let next = pet.next_at().max(now);
            if next > until {
                return (did, until);
            }
            now = next;
            if let Some(Arrival { action, there }) = pet.arrive(now, floor)
                && there
            {
                {
                    let (thing, done) = match action {
                        ActionKind::Eat { thing } => (thing, Ate(thing)),
                        ActionKind::Take { thing } => (thing, Took(thing)),
                        other => panic!("{other:?}"),
                    };
                    did.push(done);
                    if takes_away {
                        floor.retain(|lying| lying.id != thing);
                    }
                }
            }
            if pet.look(now, floor) {
                let decision = match decides {
                    Decides::Table => table(&[], &pet.untouched(floor)).expect("something lies"),
                    Decides::Says(reaction) => decision(reaction, 0, None),
                };
                did.push(Looked(decision.reaction));
                pet.react(&decision, None, floor, now, Upon::Floor);
                continue;
            }
            pet.go_on(now);
        }
        panic!("the pet stays due at {now}: {did:?}");
    }

    /// (case, the floor, what decides upon it, whether the garden takes a
    /// thing away, the steps)
    type Living = (&'static str, Vec<Thing>, Decides, bool, Vec<Step>);

    const LONG: u64 = 60_000;

    fn living() -> [Living; 6] {
        let bowl = |id, x, y| thing(id, ThingKind::Bowl, Place { x, y });
        let ball = |id, x, y| thing(id, ThingKind::Ball, Place { x, y });
        // From START (500, 500): bowl 2 is the nearest, and from it bowl 3.
        let three = vec![bowl(1, 100, 100), bowl(2, 450, 520), bowl(3, 600, 700)];
        let cases: [Living; 6] = [
            (
                "three bowls are eaten to the end, the nearest first",
                three.clone(),
                Decides::Table,
                true,
                vec![
                    Reacts(Reaction::Eat),
                    Lives(
                        LONG,
                        &[
                            Ate(2),
                            Looked(Reaction::Eat),
                            Ate(3),
                            Looked(Reaction::Eat),
                            Ate(1),
                        ],
                    ),
                    Lives(LONG, &[]),
                ],
            ),
            (
                "the ball is left after a bowl",
                vec![ball(1, 900, 100), bowl(2, 500, 500)],
                Decides::Table,
                true,
                vec![
                    Reacts(Reaction::Eat),
                    Lives(LONG, &[Ate(2), Looked(Reaction::Chase), Took(1)]),
                    Lives(LONG, &[]),
                ],
            ),
            (
                "a floor empty after eating decides nothing",
                vec![bowl(1, 600, 600)],
                Decides::Table,
                true,
                vec![Reacts(Reaction::Eat), Lives(LONG * 2, &[Ate(1)])],
            ),
            (
                "a thing the garden has not taken away is not gone to again",
                vec![ball(1, 900, 100)],
                Decides::Table,
                false,
                vec![Reacts(Reaction::Chase), Lives(LONG * 2, &[Took(1)])],
            ),
            (
                "ignore asks nothing again until something new happens",
                three.clone(),
                Decides::Table,
                true,
                vec![
                    Reacts(Reaction::Ignore),
                    Lives(LONG * 2, &[]),
                    Reacts(Reaction::Approach),
                    Lives(
                        LONG,
                        &[
                            Looked(Reaction::Eat),
                            Ate(2),
                            Looked(Reaction::Eat),
                            Ate(3),
                            Looked(Reaction::Eat),
                            Ate(1),
                        ],
                    ),
                ],
            ),
            (
                "a decision upon the floor that takes nothing off it is not asked again",
                three,
                Decides::Says(Reaction::Jump),
                true,
                vec![
                    Reacts(Reaction::Jump),
                    Lives(LONG * 2, &[Looked(Reaction::Jump)]),
                ],
            ),
        ];
        cases
    }

    /// The rows of what the pet looks at after a reaction to a happening.
    fn after_reactions() -> [Living; 2] {
        let bowl = |id, x, y| thing(id, ThingKind::Bowl, Place { x, y });
        // From START (500, 500): bowl 2 is the nearest, and from it bowl 3.
        let three = vec![bowl(1, 100, 100), bowl(2, 450, 520), bowl(3, 600, 700)];
        [
            (
                "after sitting it looks at the floor, and eats the bowls nearest first",
                three.clone(),
                Decides::Table,
                true,
                vec![
                    Reacts(Reaction::Sit),
                    Lives(
                        LONG,
                        &[
                            Looked(Reaction::Eat),
                            Ate(2),
                            Looked(Reaction::Eat),
                            Ate(3),
                            Looked(Reaction::Eat),
                            Ate(1),
                        ],
                    ),
                ],
            ),
            (
                "asleep it looks at nothing",
                three.clone(),
                Decides::Table,
                true,
                vec![Reacts(Reaction::Sleep), Lives(LONG, &[])],
            ),
        ]
    }

    #[test]
    fn a_pet_that_goes_back_to_itself_looks_at_the_floor_once() {
        for (case, mut floor, decides, takes_away, steps) in
            living().into_iter().chain(after_reactions())
        {
            let mut pet = Pet::new(3, NOW);
            let mut now = NOW;
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Reacts(reaction) => {
                        pet.react(
                            &decision(reaction, 0, None),
                            Some("ann"),
                            &floor,
                            now,
                            Upon::Happened,
                        );
                    }
                    Lives(long, expected) => {
                        let (did, until) =
                            live(&mut pet, &mut floor, (decides, takes_away), now, now + long);
                        assert_eq!(did, expected, "{case}, {index}");
                        now = until;
                    }
                }
            }
        }
    }
}
