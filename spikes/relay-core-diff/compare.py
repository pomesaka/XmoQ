#!/usr/bin/env python3
"""Runs two versions of the relay core side by side and counts where they differ.

    python3 spikes/relay-core-diff/compare.py <before> <after> [--spikes] [--mutate]
    python3 spikes/relay-core-diff/compare.py --same <revision>

Add `--world v1|distinct|full|split|one-publisher|split-one|shared` to either
for what the random events are made of; `fuzz.rs` describes them.
`split-steady` and `shared-steady` are `split-one` and `shared` in which the
publisher of the second half never says PUBLISH_DONE, and `no-announce` is
`full` in which nobody announces a namespace. The default
is `v1`. With `--priorities` the subgroup streams have one of three Publisher
Priorities, where they all have the same without it.
`--touch subgroups|publishers|republish|waiting-or-draining|asks|priority` says which events `--touching` takes
for the ones a change is about; the default is `subgroups`.

`--touching` is for a change that is meant to alter what the relay does with
some inputs and nothing else. `fuzz.rs` knows which events those inputs are
(a stream whose header leaves the Subgroup ID to its first Object, a second
stream for a Subgroup). Once the two versions have parted in a sequence,
everything after it differs too, so the differences are not counted as
failures; what has to hold is that no sequence shows a difference before its
first such event. That says nothing about what follows the first such event.
For that there are the worlds `split` and `split-one`: what the change is
about happens in one half, and the sessions of the other half (3 and 4) have
to be treated alike by both versions from the first event to the last. A
difference there is counted as `bystander`, with or without `--touching`, and
fails the run. In `shared`, session 4 subscribes to the tracks of both halves,
and what has to be alike is what is done to session 3 and to session 4's
subscription to the track of the second half: what is said on its request
stream, the streams opened for it, and what closes the session. The Track
Alias of that subscription is not compared; the relay numbers the aliases of a
session as it accepts subscriptions, of whichever track.

Requests the relay opens are compared in the same way, under the session and
how many the version opened on it before.

Streams to subscribers are compared under a name, the session and how many
streams the version opened to it before, and not by their number: a version
numbers them from one counter, so a difference in one track shifts the
numbers in every other. An event that names such a stream is taken to mean
the stream of the new version and is passed to the old one under the number
it gave the stream of that name. `--raw-numbers` turns the names off; it is
the check that the comparison of `split` can fail (it has to).

<before> and <after> are jj revisions; <after> may also be the word `disk`,
for the files of the working directory as they are. The tool exports `crates/` of <after>
into a scratch workspace, puts `lib.rs` and `routing.rs` of `xmoq-relay-core`
from <before> next to it as the module `old`, and patches `Relay::handle` to
feed every event to both and to compare the actions they return. Then it runs

  * the tests of `xmoq-relay-core` and `xmoq-relay` of <after>, and
  * `fuzz.rs`: random event sequences (XMOQ_FUZZ_RUNS x XMOQ_FUZZ_LEN events),
  * with --spikes, the scenarios of spikes/moqtail/browser/run.mjs against a
    relay built that way (needs the binaries that script needs).

--mutate is the check of the tool itself: it changes one decision of <after>
(the Error Code after an upstream timeout) before comparing, and the run has
to end with content mismatches and exit code 1.

--same asks another question of one version: do the same events always lead
to the same actions, in the same order? It runs the random events through
that version alone, three times in processes of their own, and compares a
digest of every list of actions. A core that pushes actions while it walks a
`HashMap` prints three different digests.

For every event it compares, in this order, and counts the first that differs:

  content   the actions as a multiset: what is done, to whom, with what
  state     `Relay::counts()` after the event: what each version still holds.
            What only <after> counts is left out
  sequence  the actions for each session, in order, leaving out
            `StopDataStream`. Not looked at for an event that drops a session
            (`SessionClosed`, or an action `CloseSession`): there both
            versions may walk a `HashMap` of tracks
  order     the whole list in order, which is all that is left

It exits with 1 when any event differed in content, state or sequence, when a
test failed, or when the random events did not run in the number asked for.
With `--random-only` it exits with 1 when a random sequence differed in
content, state or sequence, when a bystander was treated differently, or
when the sequences were not counted; the tests of the crate are run and
reported, and do not decide. That is the strict comparison for a change that
adds or changes tests, which then differ between the versions by design.
With `--touching` it exits with 1 when a sequence differed before its first
such event, when a bystander was treated differently, or when the sequences
were not counted.
Nothing outside `spikes/relay-core-diff/.work` is written.

Exit code 2 and a line `NOT COMPARED` mean that nothing was compared: the two
cores did not build together, or the random sequences were not counted. No
counts are printed then. The old core is built against the `xmoq-moqt` of
<after>, so a change to a type of that crate that the core matches on (a new
variant of an enum of the session, say) is such a case.

Both versions have to share `events.rs`: the `Event` and `Action` types are
those of <after>. A <before> whose events differ does not compile here, with
one exception: a <before> from the time when the timer of a drain had no
number (`wait`) is patched to ignore the number of the event and to give
every timer the number 0. The numbers are left out of the comparison, and
the random events answer a timer with a number the new core gave.
"""
import collections, os, pathlib, re, shutil, subprocess, sys

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent.parent
WORK = HERE / ".work"
CORE = "crates/relay-core/src"

SPIKES = [
    "--rust-publisher own --publish-namespace --direction rust-to-rust --rust-subscriber own --await-publish-done",
    "--rust-publisher own --publish-namespace --direction rust-to-rust --rust-subscriber moqtail",
    "--rust-publisher own --publish-namespace --direction rust-to-browser",
    "--rust-publisher own --publish-namespace --direction rust-to-browser --subscribers 3",
    "--rust-publisher own --publish-namespace --direction rust-to-rust --rust-subscriber own --subscribers 2 --await-publish-done",
    "--rust-publisher own --subscribe-before-publish --direction rust-to-rust --rust-subscriber own",
    "--rust-publisher own --subscribe-before-publish --direction rust-to-rust --rust-subscriber moqtail",
    "--rust-publisher own --subscribe-before-publish --direction rust-to-browser",
    "--rust-publisher own --subscribe-before-publish --direction rust-to-browser --subscribers 3",
    "--publish-namespace --direction browser-to-rust --rust-subscriber own",
    "--publish-namespace --direction browser-to-rust --rust-subscriber moqtail",
    "--publish-namespace --direction browser-to-browser --subscribers 2",
]


TOUCHES = ("subgroups", "publishers", "republish", "waiting-or-draining", "asks", "priority")
WORLDS = ("v1", "distinct", "full", "split", "one-publisher", "split-one", "shared", "split-steady", "shared-steady",
          "no-announce")
# Worlds of `fuzz.rs` with something taken out on top: the world, and the
# variable that tells `fuzz.rs` what.
VARIANTS = {"split-steady": ("split-one", "XMOQ_FUZZ_STEADY"), "shared-steady": ("shared", "XMOQ_FUZZ_STEADY"),
            "no-announce": ("full", "XMOQ_FUZZ_NO_ANNOUNCE")}


def jj(*args):
    return subprocess.run(["jj", "--ignore-working-copy", *args], cwd=REPO, check=True,
                          capture_output=True, text=True).stdout


def once(text, old, new, what):
    if text.count(old) != 1:
        sys.exit(f"cannot patch: expected one `{what}`, found {text.count(old)}")
    return text.replace(old, new)


def export(after):
    shutil.rmtree(WORK / "crates", ignore_errors=True)
    WORK.mkdir(exist_ok=True)
    if after == "disk":
        shutil.copytree(REPO / "crates", WORK / "crates")
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            shutil.copy(REPO / name, WORK / name)
        return
    files = jj("file", "list", "-r", after, "crates", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml").split()
    for path in files:
        target = WORK / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(jj("file", "show", "-r", after, path))


def add_old_core(before):
    old = WORK / CORE / "old"
    old.mkdir()
    lib = jj("file", "show", "-r", before, f"{CORE}/lib.rs")
    lib = once(lib, "mod events;\n", "", "mod events;")
    lib, replaced = re.subn(r"#\[cfg\(test\)\]\nmod tests;\n", "#[cfg(test)]\nmod fuzz;\n", lib, count=1)
    if replaced != 1:
        sys.exit("cannot patch: the old core has no `mod tests;` to put the random events in place of")
    lib = once(lib, "pub use events::{", "pub use crate::events::{", "pub use events::{")
    # `Action::target` exists once for the shared type, in the new core.
    lib = lib.replace("fn target(&self)", "fn old_target(&self)").replace("action.target()", "action.old_target()")
    if "next_drain_wait" not in lib:
        lib, event = re.subn(r"(Event::DrainTimeout \{\s+session,\s+track_alias,)(\s+\} =>)", r"\1 ..\2", lib)
        lib, action = re.subn(r"(Action::StartDrainTimer \{\s+session,\s+track_alias,)(\s+\}\))", r"\1 wait: 0,\2", lib)
        if (event, action) != (1, 1):
            sys.exit(f"cannot patch: the old core's drain timer (event {event}, action {action})")
    # An event or an action that <after> has and <before> has not: the old
    # core does nothing with such an event, and is counted doing so. WHY
    # counted: in the worlds of `fuzz.rs` no such event is made, so the count
    # has to be 0; anything else means the new kind of event reaches a place
    # the comparison takes for one the change does not touch.
    lib, handled = re.subn(
        r"(            Event::SessionClosed \{ session \} => self\.drop_session\(session, &mut actions\),\n)(        \}\n        actions\n)",
        r"\1            _ => {\n                crate::UNKNOWN_TO_THE_OLD_CORE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n            }\n\2",
        lib)
    lib, targeted = re.subn(r"(            Self::CloseSession \{ \.\. \} => Target::Closing,\n)(        \}\n)",
                            r"\1            _ => Target::Closing,\n\2", lib)
    if (handled, targeted) != (1, 1):
        sys.exit(f"cannot patch: the old core's match on events ({handled}) or on actions ({targeted})")
    allow = ("#![allow(dead_code, unused_imports, unreachable_patterns, clippy::all, clippy::pedantic, "
             "missing_docs)]\n")
    (old / "mod.rs").write_text(allow + lib)
    routing = jj("file", "show", "-r", before, f"{CORE}/routing.rs")
    (old / "routing.rs").write_text(once(routing, "use crate::{", "use super::{", "use crate::{"))
    # The modules a core may have besides: taken along when <before> has them.
    # The events are the one type both cores share, so `crate::events` stays.
    for module in ("fetch", "requests"):
        try:
            text = jj("file", "show", "-r", before, f"{CORE}/{module}.rs")
        except subprocess.CalledProcessError:
            continue
        for sibling in ("routing", "requests"):
            text = text.replace(f"use crate::{sibling}::", f"use super::{sibling}::")
        (old / f"{module}.rs").write_text(once(text, "use crate::{", "use super::{", "use crate::{"))
    (old / "fuzz.rs").write_text("#![allow(clippy::all, clippy::pedantic)]\n" + (HERE / "fuzz.rs").read_text())


def patch_new_core():
    path = WORK / CORE / "lib.rs"
    lib = path.read_text()
    lib = once(lib, "mod events;\n", "mod events;\nmod old;\n", "mod events;")
    lib = once(lib, "    pub fn handle(&mut self, event: Event) -> Vec<Action> {",
               "    fn decide(&mut self, event: Event) -> Vec<Action> {", "pub fn handle")
    lib = once(lib, "    next_own_request: u64,\n}", "    next_own_request: u64,\n    shadow: old::Relay,\n    names: (OutNames, OutNames),\n}",
               "the last field of Relay")
    lib = once(lib, "            next_own_request: 0,\n        }",
               "            next_own_request: 0,\n            shadow: old::Relay::new(implementation),\n"
               "            names: Default::default(),\n        }",
               "the last field of Relay::new")
    lib += '''
/// Events the old core did nothing with because it does not know their kind.
pub(crate) static UNKNOWN_TO_THE_OLD_CORE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Events for which the two cores differed in more than order, so far.
pub(crate) static DIFFERENCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Events for which the two cores did something else to a session named in
/// `XMOQ_DIFF_BYSTANDERS`, so far.
pub(crate) static BYSTANDER_DIFFERENCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Actions of the new core for those sessions, so far.
pub(crate) static BYSTANDER_ACTIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Request streams of a session that is a bystander in part: what is done on
/// them, and to the subscription that was accepted on them, has to be the
/// same in both cores. Set by `fuzz.rs`.
pub(crate) static BYSTANDER_STREAMS: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());

/// The streams to subscribers of one core, under a name that does not depend
/// on how the core numbers them: the session, the Track Alias of the header,
/// and how many streams the core opened to that session with that alias
/// before. WHY the alias: a session can subscribe to a track in which the
/// cores differ and to one in which they must not. WHY: the two cores number the streams from
/// one counter each, so once they differ in one track, every later stream of
/// every track has another number in the one than in the other.
#[derive(Debug, Default)]
struct OutNames {
    by_key: HashMap<u64, (u64, String, u64)>,
    opened: HashMap<(u64, String), u64>,
    /// A watched subscription under a name of its own, in place of its Track
    /// Alias: which of the session's watched subscriptions it is. WHY: the
    /// relay numbers the aliases of a session as it accepts subscriptions, so
    /// one more or less to a track in which the cores differ shifts the
    /// alias of a subscription in which they must not.
    alias_names: HashMap<(u64, u64), String>,
    watched: HashMap<u64, u64>,
    /// The session and Track Alias of a subscription that was accepted on one
    /// of `BYSTANDER_STREAMS`, and the streams opened for one.
    watched_aliases: HashSet<(u64, u64)>,
    watched_outs: HashSet<u64>,
    /// The requests the core opened, named in the same way and for the same
    /// reason: the session, and how many it opened on that session before.
    own_by_key: HashMap<u64, (u64, u64)>,
    own_opened: HashMap<u64, u64>,
}

#[allow(clippy::all, clippy::pedantic)]
impl OutNames {
    fn note(&mut self, actions: &[Action], streams: &[(u64, u64)]) {
        for action in actions {
            if let Action::SendRequest { session, stream, message: ControlMessage::SubscribeOk(ok) } = action
                && streams.contains(&(session.0, stream.0))
            {
                self.watched_aliases.insert((session.0, ok.track_alias));
                let nth = self.watched.entry(session.0).or_default();
                self.alias_names.insert((session.0, ok.track_alias), format!("watched {nth}"));
                *nth += 1;
            }
            if let Action::OpenRequest { session, request, .. } = action {
                let opened = self.own_opened.entry(session.0).or_default();
                self.own_by_key.insert(request.0, (session.0, *opened));
                *opened += 1;
            }
            if let Action::OpenDataStream { session, out_stream, header } = action {
                let alias = (session.0, header.track_alias);
                let name = self.alias_names.get(&alias).cloned().unwrap_or_else(|| alias.1.to_string());
                let opened = self.opened.entry((alias.0, name.clone())).or_default();
                self.by_key.insert(out_stream.0, (alias.0, name, *opened));
                *opened += 1;
                if self.watched_aliases.contains(&alias) {
                    self.watched_outs.insert(out_stream.0);
                }
            }
        }
    }

    /// A session key is used anew: its aliases start over, and with them the
    /// count of the streams of each. WHY the count as well: the alias that was
    /// a track in which the cores differ can be one in which they must not
    /// in the next session under the key.
    fn forget_session(&mut self, session: SessionKey) {
        self.watched_aliases.retain(|(watched, _)| *watched != session.0);
        self.alias_names.retain(|(named, _), _| *named != session.0);
        self.watched.remove(&session.0);
        self.opened.retain(|(opened, _), _| *opened != session.0);
        self.by_key.retain(|_, (named, _, _)| *named != session.0);
        self.own_opened.remove(&session.0);
        self.own_by_key.retain(|_, (named, _)| *named != session.0);
    }

    fn own_key_of(&self, name: (u64, u64)) -> Option<u64> {
        self.own_by_key.iter().find(|(_, known)| **known == name).map(|(key, _)| *key)
    }

    /// Whether the action is done to a bystander: a session of `whole`, or
    /// the part of a session that `streams` names. What closes such a session
    /// or is said on its control stream counts, whichever track caused it.
    fn is_about_a_bystander(&self, action: &Action, whole: &[u64], streams: &[(u64, u64)]) -> bool {
        let in_part = |session: &SessionKey| streams.iter().any(|(watched, _)| *watched == session.0);
        let watched = match action {
            Action::SendRequest { session, stream, .. }
            | Action::FinishRequest { session, stream, .. }
            | Action::ResetRequest { session, stream, .. }
            | Action::StopRequest { session, stream, .. }
            | Action::StartRequestTimer { session, stream, .. } => streams.contains(&(session.0, stream.0)),
            Action::OpenDataStream { session, header, .. } => {
                self.watched_aliases.contains(&(session.0, header.track_alias))
            }
            Action::SendObject { out_stream, .. }
            | Action::FinishDataStream { out_stream, .. }
            | Action::ResetDataStream { out_stream, .. } => self.watched_outs.contains(&out_stream.0),
            Action::CloseSession { session, .. } | Action::SendControl { session, .. } => in_part(session),
            _ => false,
        };
        watched || self.session_of(action).is_some_and(|session| whole.contains(&session))
    }

    fn key_of(&self, name: &(u64, String, u64)) -> Option<u64> {
        self.by_key.iter().find(|(_, known)| *known == name).map(|(key, _)| *key)
    }

    /// The text with every stream to a subscriber under its name.
    fn rename(&self, text: &str) -> String {
        // The number of a drain timer: an old core that has none says 0.
        let text = &without_the_number_after(text, "wait: ");
        // To show that the names are needed: with the numbers as they are,
        // the comparison of the world `split` has to find differences.
        if std::env::var_os("XMOQ_DIFF_RAW_NUMBERS").is_some() {
            return text.to_string();
        }
        let text = &self.rename_own(text);
        let (mut out, mut rest) = (String::new(), text.as_str());
        while let Some(at) = rest.find("OutStreamKey(") {
            let digits = at + "OutStreamKey(".len();
            let end = rest[digits..].find(')').map_or(rest.len(), |found| digits + found);
            out.push_str(&rest[..at]);
            match rest[digits..end].parse::<u64>().ok().and_then(|key| self.by_key.get(&key)) {
                Some((session, alias, nth)) => out.push_str(&format!("Out({session}.{alias}#{nth}")),
                None => out.push_str(&rest[at..end]),
            }
            rest = &rest[end..];
        }
        out + rest
    }

    fn rename_own(&self, text: &str) -> String {
        let (mut out, mut rest) = (String::new(), text);
        while let Some(at) = rest.find("OwnRequestKey(") {
            let digits = at + "OwnRequestKey(".len();
            let end = rest[digits..].find(')').map_or(rest.len(), |found| digits + found);
            out.push_str(&rest[..at]);
            match rest[digits..end].parse::<u64>().ok().and_then(|key| self.own_by_key.get(&key)) {
                Some((session, nth)) => out.push_str(&format!("Own({session}#{nth}")),
                None => out.push_str(&rest[at..end]),
            }
            rest = &rest[end..];
        }
        out + rest
    }

    fn session_of(&self, action: &Action) -> Option<u64> {
        match action.target() {
            Target::Session(session) => Some(session.0),
            Target::OutStream(out_stream) => self.by_key.get(&out_stream.0).map(|name| name.0),
            Target::Closing => match action {
                Action::CloseSession { session, .. } => Some(session.0),
                _ => None,
            },
        }
    }
}

/// The line with every Track Alias blanked. For a session that is watched
/// in part, the number of an alias is not held against a core (see
/// `OutNames::alias_names`); the streams are told apart by their names.
#[allow(clippy::all, clippy::pedantic)]
fn without_aliases(line: &str) -> String {
    without_the_number_after(line, "track_alias: ")
}

#[allow(clippy::all, clippy::pedantic)]
fn without_the_number_after(line: &str, field: &str) -> String {
    let (mut out, mut rest) = (String::new(), line);
    while let Some(at) = rest.find(field) {
        let digits = at + field.len();
        let end = rest[digits..].find(|c: char| !c.is_ascii_digit()).map_or(rest.len(), |found| digits + found);
        out.push_str(&rest[..digits]);
        out.push('_');
        rest = &rest[end..];
    }
    out + rest
}

#[allow(clippy::all, clippy::pedantic)]
impl Relay {
    /// The event as the old core has to be told it. Whoever calls `handle`
    /// got the numbers of the streams from the new core.
    fn for_old(&self, event: &Event) -> Event {
        if std::env::var_os("XMOQ_DIFF_RAW_NUMBERS").is_some() {
            return event.clone();
        }
        let old_key = |key: &OutStreamKey| {
            let name = self.names.1.by_key.get(&key.0);
            // A stream the old core does not have gets a number nobody has.
            OutStreamKey(name.and_then(|name| self.names.0.key_of(name)).unwrap_or(u64::MAX - key.0))
        };
        let old_own = |key: &OwnRequestKey| {
            let name = self.names.1.own_by_key.get(&key.0);
            OwnRequestKey(name.and_then(|name| self.names.0.own_key_of(*name)).unwrap_or(u64::MAX - key.0))
        };
        match event {
            Event::OwnRequestMessage { session, request, message } => {
                Event::OwnRequestMessage { session: *session, request: old_own(request), message: message.clone() }
            }
            Event::OwnRequestClosed { session, request, how } => {
                Event::OwnRequestClosed { session: *session, request: old_own(request), how: *how }
            }
            Event::UpstreamTimeout { session, request } => {
                Event::UpstreamTimeout { session: *session, request: old_own(request) }
            }
            Event::OutStreamOpened { out_stream } => Event::OutStreamOpened { out_stream: old_key(out_stream) },
            Event::OutStreamClosed { out_stream } => Event::OutStreamClosed { out_stream: old_key(out_stream) },
            Event::SendQueueFull { out_stream } => Event::SendQueueFull { out_stream: old_key(out_stream) },
            other => other.clone(),
        }
    }

    /// Feeds the event to both cores and reports where they differ.
    #[allow(missing_docs)]
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let said = format!("{event:?}");
        let drops_a_session = matches!(event, Event::SessionClosed { .. });
        if let Event::SessionOpened { session, .. } = &event {
            self.names.0.forget_session(*session);
            self.names.1.forget_session(*session);
        }
        let streams = BYSTANDER_STREAMS.lock().unwrap().clone();
        let before = self.shadow.handle(self.for_old(&event));
        let after = self.decide(event);
        self.names.0.note(&before, &streams);
        self.names.1.note(&after, &streams);
        let in_order = |actions: &[Action], names: &OutNames| -> Vec<String> {
            actions.iter().map(|a| names.rename(&format!("{a:?}"))).collect()
        };
        let sorted = |actions: &[Action], names: &OutNames| {
            let mut lines = in_order(actions, names);
            lines.sort();
            lines
        };
        // The actions for each session and each stream to a subscriber, in
        // the order they were pushed.
        let by_target = |actions: &[Action], names: &OutNames| {
            let mut lines: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            for action in actions {
                if matches!(action, Action::StopDataStream { .. }) {
                    continue;
                }
                let target = match action.target() {
                    Target::Session(session) => format!("session {}", session.0),
                    Target::OutStream(out_stream) => names.rename(&format!("{out_stream:?}")),
                    Target::Closing => "closing".to_string(),
                };
                lines.entry(target).or_default().push(names.rename(&format!("{action:?}")));
            }
            lines
        };
        let closes = before.iter().any(|a| matches!(a, Action::CloseSession { .. }));
        // What the new core counts and the old one does not is left out.
        let held_before = format!("{:?}", self.shadow.counts());
        let known = |field: &&str| {
            let name = field.split(':').next().and_then(|name| name.split_whitespace().last());
            name.is_some_and(|name| held_before.contains(&format!(" {name}:")))
        };
        let held_after = format!("{:?}", self.counts());
        let held_after = held_after.split(", ").filter(known).collect::<Vec<_>>().join(", ");
        let held_after = if held_after.ends_with('}') { held_after } else { held_after + " }" };
        let (old, new) = (&self.names.0, &self.names.1);
        let kind = if sorted(&before, old) != sorted(&after, new) {
            Some("content")
        } else if held_before != held_after {
            Some("state")
        } else if !drops_a_session && !closes && by_target(&before, old) != by_target(&after, new) {
            Some("sequence")
        } else if in_order(&before, old) != in_order(&after, new) {
            Some("order")
        } else {
            None
        };
        let line = format!(
            "event={said} before={before:?} after={after:?} held_before={held_before} held_after={held_after}"
        );
        // Sessions that have no part in what the change is about: what is
        // done to them, and in what order, has to be the same throughout.
        if let Ok(bystanders) = std::env::var("XMOQ_DIFF_BYSTANDERS") {
            let bystanders: Vec<u64> = bystanders.split(',').filter_map(|n| n.parse().ok()).collect();
            let to_them = |actions: &[Action], names: &OutNames| -> Vec<String> {
                actions
                    .iter()
                    .filter(|action| names.is_about_a_bystander(action, &bystanders, &streams))
                    .map(|action| names.rename(&format!("{action:?}")))
                    .map(|line| if streams.is_empty() { line } else { without_aliases(&line) })
                    .collect()
            };
            BYSTANDER_ACTIONS.fetch_add(to_them(&after, new).len() as u64, std::sync::atomic::Ordering::Relaxed);
            if to_them(&before, old) != to_them(&after, new) {
                println!("CORE_MISMATCH kind=bystander {line}");
                BYSTANDER_DIFFERENCES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some(kind) = kind {
            println!("CORE_MISMATCH kind={kind} {line}");
            if kind != "order" {
                DIFFERENCES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let keep_going = std::env::var_os("XMOQ_DIFF_KEEP_GOING").is_some();
            assert!(kind == "order" || !cfg!(test) || keep_going, "CORE_MISMATCH kind={kind} {line}");
        }
        after
    }
}
'''
    path.write_text(lib)


def same(revision):
    """Runs the random events through one version three times and compares the digests."""
    commit = "the working directory" if revision == "disk" else jj("log", "-r", revision, "--no-graph", "-T", "commit_id.short(12)").strip()
    print(f"revision: {revision} = {commit}")
    export(revision)
    (WORK / CORE / "fuzz.rs").write_text("#![allow(clippy::all, clippy::pedantic)]\n" + (HERE / "fuzz.rs").read_text())
    path = WORK / CORE / "lib.rs"
    stub = ("\n/// Nothing is compared in this build.\n#[cfg(test)]\n"
            "pub(crate) static DIFFERENCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n"
            "#[cfg(test)]\n"
            "pub(crate) static BYSTANDER_DIFFERENCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n"
            "#[cfg(test)]\n"
            "pub(crate) static BYSTANDER_ACTIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n"
            "#[cfg(test)]\n"
            "pub(crate) static BYSTANDER_STREAMS: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());\n")
    path.write_text(once(path.read_text(), "mod events;\n", "mod events;\n#[cfg(test)]\nmod fuzz;\n", "mod events;") + stub)
    digests = []
    for _ in range(3):
        code, output = cargo("test", "--locked", "-p", "xmoq-relay-core", "fuzz::", "--", "--nocapture")
        found = re.search(r"FUZZ runs=\d+ events=(\d+) digest=(\w+)", output)
        if code != 0 or not found:
            sys.exit(output[-3000:])
        asked = int(os.environ.get("XMOQ_FUZZ_RUNS", "20000")) * int(os.environ.get("XMOQ_FUZZ_LEN", "80"))
        if int(found.group(1)) != asked:
            sys.exit(f"the random events did not run in the number asked for: {found.group(1)}, not {asked}")
        digests.append(found.group(2))
        print(f"run {len(digests)}: events={found.group(1)} digest={found.group(2)}")
    alike = len(set(digests)) == 1
    print("SAME: the three runs returned the same actions in the same order" if alike
          else "DIFFERENT: the order of the actions depends on the run")
    sys.exit(0 if alike else 1)


def cargo(*args):
    env = dict(os.environ, CARGO_TARGET_DIR=str(WORK / "target"))
    run = subprocess.run(["cargo", *args], cwd=WORK, env=env, capture_output=True, text=True)
    return run.returncode, run.stdout + run.stderr


def compared(*args):
    """Runs the tests of a crate in which both cores are, or ends the run.

    WHY the run ends here when the crate does not build: then nothing was
    compared, and every count below would be a 0 that reads as "no
    difference". The old core is built against the `xmoq-moqt` of <after>, so
    this is what happens when a type of that crate the core matches on has
    changed between the two revisions.
    """
    code, output = cargo(*args)
    if code != 0 and "could not compile" in output:
        errors = [line for line in output.splitlines() if line.startswith("error")]
        print("NOT COMPARED: the two cores do not build together.")
        for line in errors[:12]:
            print("    " + line[:300])
        print("The old core is built against the xmoq-moqt of <after>: either it does not fit that, or it has a")
        print("module this tool does not take along (`add_old_core`).")
        sys.exit(2)
    return output


def order_cause(line):
    """Which of the places that iterate over a `HashMap` an order-only mismatch comes from."""
    event = re.search(r"event=(\w+)", line).group(1)
    before = line.split(" before=[", 1)[1].split("] after=[", 1)[0]
    if event == "SessionClosed" or "CloseSession" in before:
        return "a session is dropped that has to do with several tracks"
    if before.count("StopDataStream") >= 2:
        return "a track ends with several of its source's streams open"
    return "other"


def report(name, output, totals, random_events=False):
    lines = [line for line in output.splitlines() if "CORE_MISMATCH" in line]
    kinds = collections.Counter(re.search(r"kind=(\w+)", line).group(1) for line in lines)
    wrong = [line for line in lines if "kind=order" not in line and "kind=bystander" not in line]
    order = [line for line in lines if "kind=order" in line]
    results = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed", output)
    fuzz = re.search(r"FUZZ runs=(\d+) events=(\d+)", output)
    passed = sum(int(p) for _, p, _ in results)
    failed = sum(int(f) for _, _, f in results)
    print(f"{name}: content={kinds['content']} state={kinds['state']} sequence={kinds['sequence']} order={kinds['order']}"

          + (f" tests passed={passed} failed={failed}" if results else "")
          + (f" random events={fuzz.group(2)}" if fuzz else ""))
    for cause, count in collections.Counter(order_cause(line) for line in order).most_common():
        print(f"    order: {count} x {cause}")
    for line in wrong[:3]:
        print("    " + line[:1500])
    for kind in ("content", "state", "sequence", "order"):
        totals[kind] += kinds[kind]
    totals["failed"] += failed
    touch = re.search(r"FUZZ_TOUCH sequences=(\d+) touching=(\d+) differing=(\d+) differing_before_touching=(\d+)", output)
    if touch:
        print(f"    sequences: {touch.group(1)}; with an event the change is about: {touch.group(2)}; "
              f"in which the versions differ: {touch.group(3)}; differ before such an event: {touch.group(4)}")
        totals["before_touching"] += int(touch.group(4))
        totals["differing_sequences"] += int(touch.group(3))
    elif random_events and ("--touching" in sys.argv or "--random-only" in sys.argv):
        # WHY checked: without the line nothing was counted, and 0 would read as a pass.
        print("    THE SEQUENCES WERE NOT COUNTED: no FUZZ_TOUCH line")
        totals["not_counted"] += 1
    # Only the random events of the world `split` keep the bystanders out of
    # what the change is about; the tests of the crate use the same sessions.
    bystander = re.search(r"FUZZ_BYSTANDER differences=(\d+) actions=(\d+)", output)
    if bystander and "XMOQ_DIFF_BYSTANDERS" in os.environ:
        print(f"    events of the random sequences in which the bystanders were treated differently: {bystander.group(1)}"
              f" (actions for the bystanders: {bystander.group(2)})")
        totals["bystander"] += int(bystander.group(1))
        in_the_sequences = output.split("FUZZ_BYSTANDER", 1)[0].splitlines()
        for line in [line for line in in_the_sequences if "kind=bystander" in line][:3]:
            print("    " + line[:2500])
    unknown = re.search(r"FUZZ_UNKNOWN_TO_THE_OLD_CORE events=(\d+)", output)
    if unknown:
        print(f"    events of a kind the old core does not know, in the random sequences: {unknown.group(1)}")
        if int(unknown.group(1)):
            # The comparison says nothing about a sequence with such an event.
            totals["failed"] += 1
            totals["not_counted"] += 1
    elif random_events:
        print("    THE EVENTS UNKNOWN TO THE OLD CORE WERE NOT COUNTED: no FUZZ_UNKNOWN_TO_THE_OLD_CORE line")
        totals["not_counted"] += 1
    carried = re.search(r"FUZZ_CARRIED (.*)", output)
    if carried:
        print(f"    what the new version did in the random events: {carried.group(1)}")
    if random_events:
        # WHY checked: a run in which the random events did not happen ends
        # with no difference found, and would read as a pass.
        runs = int(os.environ.get("XMOQ_FUZZ_RUNS", "20000"))
        length = int(os.environ.get("XMOQ_FUZZ_LEN", "80"))
        if not fuzz or (int(fuzz.group(1)), int(fuzz.group(2))) != (runs, runs * length):
            print(f"    THE RANDOM EVENTS DID NOT RUN: expected runs={runs} events={runs * length}")
            totals["failed"] += 1
            totals["not_counted"] += 1
    if not results and "error" in output and not lines and "RESULT" not in output:
        print(output[-3000:])
        totals["failed"] += 1


def main():
    if "--world" in sys.argv:
        at = sys.argv.index("--world")
        if at + 1 >= len(sys.argv) or sys.argv[at + 1] not in WORLDS:
            sys.exit(f"--world takes one of {', '.join(WORLDS)}")
        os.environ["XMOQ_FUZZ_WORLD"] = sys.argv[at + 1]
        del sys.argv[at:at + 2]
    world = os.environ.get("XMOQ_FUZZ_WORLD", "v1")
    print(f"world: {world}")
    if world in VARIANTS:
        world, variable = VARIANTS[world]
        os.environ["XMOQ_FUZZ_WORLD"] = world
        os.environ[variable] = "1"
    # The sessions of the half of the world the change is kept out of. In
    # `shared`, session 4 is one of them for one of its subscriptions only,
    # which `fuzz.rs` names.
    bystanders = {"split": "3,4", "split-one": "3,4", "shared": "3"}
    if world in bystanders:
        os.environ["XMOQ_DIFF_BYSTANDERS"] = bystanders[world]
    if "--touch" in sys.argv:
        at = sys.argv.index("--touch")
        if at + 1 >= len(sys.argv) or sys.argv[at + 1] not in TOUCHES:
            sys.exit(f"--touch takes one of {', '.join(TOUCHES)}")
        os.environ["XMOQ_FUZZ_TOUCH"] = sys.argv[at + 1]
        del sys.argv[at:at + 2]
    print(f"touch: {os.environ.get('XMOQ_FUZZ_TOUCH', 'subgroups')}")
    if "--touching" in sys.argv:
        os.environ["XMOQ_DIFF_KEEP_GOING"] = "1"
    if "--priorities" in sys.argv:
        os.environ["XMOQ_FUZZ_PRIORITIES"] = "1"
        print("PRIORITIES: the subgroup streams have one of three Publisher Priorities")
    if "--raw-numbers" in sys.argv:
        os.environ["XMOQ_DIFF_RAW_NUMBERS"] = "1"
        print("RAW NUMBERS: streams to subscribers are compared by the number each version gave them")
    args = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
    if "--same" in sys.argv:
        if len(args) != 1:
            sys.exit("--same takes one revision")
        same(args[0])
    if len(args) != 2:
        sys.exit(__doc__)
    before, after = args
    for name, rev in (("before", before), ("after", after)):
        commit = "the working directory" if rev == "disk" else jj("log", "-r", rev, "--no-graph", "-T", "commit_id.short(12)").strip()
        print(f"{name}: {rev} = {commit}")
    export(after)
    add_old_core(before)
    patch_new_core()
    if "--mutate" in sys.argv:
        path = WORK / CORE / "routing.rs"
        path.write_text(once(path.read_text(), "&Refusal::code(request_error_code::TIMEOUT),",
                             "&Refusal::code(request_error_code::DOES_NOT_EXIST),", "the TIMEOUT refusal"))
        print("MUTATED: the new core answers DOES_NOT_EXIST where it answered TIMEOUT")
    totals = collections.Counter()
    output = compared("test", "--locked", "-p", "xmoq-relay-core", "--", "--nocapture", "--test-threads", "1")
    report("tests of xmoq-relay-core and the random events", output, totals, random_events=True)
    output = compared("test", "--locked", "-p", "xmoq-relay", "--", "--nocapture", "--test-threads", "1")
    report("tests of xmoq-relay", output, totals)
    if "--spikes" in sys.argv:
        code, output = cargo("build", "--locked", "-p", "xmoq-relay")
        if code != 0:
            sys.exit(output[-3000:])
        env = dict(os.environ, XMOQ_RELAY_BIN=str(WORK / "target" / "debug" / "xmoq-relay"))
        for index, scenario in enumerate(SPIKES, 1):
            run = subprocess.run(["node", "spikes/moqtail/browser/run.mjs", "--relay", "own", "--port", str(49400 + index),
                                  *scenario.split()], cwd=REPO, env=env, capture_output=True, text=True)
            output = run.stdout + run.stderr
            matched = '"match":true' in output
            report(f"spike {index:2} ({'payload matched' if matched else 'FAILED'})", output, totals)
            totals["failed"] += not matched
    print(f"TOTAL content={totals['content']} state={totals['state']} sequence={totals['sequence']} "
          f"order={totals['order']} failed={totals['failed']}")
    if totals["not_counted"] and ("--touching" in sys.argv or "--random-only" in sys.argv):
        # WHY no counts here: the sequences were not counted, and a line of
        # zeros would be read as a result.
        print("NOT COMPARED: the random sequences were not counted (see above).")
        sys.exit(2)
    if "--touching" in sys.argv:
        # The tests of the new version hold the new behaviour, so tests that
        # differ are expected; a difference before the change is touched is not.
        print(f"TOUCHING differing_before_touching={totals['before_touching']} bystander={totals['bystander']} "
              f"not_counted={totals['not_counted']}")
        sys.exit(1 if totals["before_touching"] or totals["bystander"] or totals["not_counted"] else 0)
    if "--random-only" in sys.argv:
        # For a change that comes with tests of its own: those differ between
        # the versions, as they are meant to, and say nothing about whether
        # anything else changed. The random sequences do.
        print(f"RANDOM ONLY differing_sequences={totals['differing_sequences']} bystander={totals['bystander']} "
              f"not_counted={totals['not_counted']}")
        sys.exit(1 if totals["differing_sequences"] or totals["bystander"] or totals["not_counted"] else 0)
    wrong = totals["content"] or totals["state"] or totals["sequence"] or totals["bystander"] or totals["failed"]
    sys.exit(1 if wrong else 0)


main()
