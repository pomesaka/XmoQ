#!/usr/bin/env python3
"""Runs two versions of the relay core side by side and counts where they differ.

    python3 spikes/relay-core-diff/compare.py <before> <after> [--spikes] [--mutate]
    python3 spikes/relay-core-diff/compare.py --same <revision>

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
  state     `Relay::counts()` after the event: what each version still holds
  sequence  the actions for each session, in order, leaving out
            `StopDataStream`. Not looked at for an event that drops a session
            (`SessionClosed`, or an action `CloseSession`): there both
            versions may walk a `HashMap` of tracks
  order     the whole list in order, which is all that is left

It exits with 1 when any event differed in content, state or sequence, when a
test failed, or when the random events did not run in the number asked for.
Nothing outside `spikes/relay-core-diff/.work` is written.

Both versions have to share `events.rs`: the `Event` and `Action` types are
those of <after>. A <before> whose events differ does not compile here.
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
    allow = "#![allow(dead_code, unused_imports, clippy::all, clippy::pedantic, missing_docs)]\n"
    (old / "mod.rs").write_text(allow + lib)
    routing = jj("file", "show", "-r", before, f"{CORE}/routing.rs")
    (old / "routing.rs").write_text(once(routing, "use crate::{", "use super::{", "use crate::{"))
    (old / "fuzz.rs").write_text("#![allow(clippy::all, clippy::pedantic)]\n" + (HERE / "fuzz.rs").read_text())


def patch_new_core():
    path = WORK / CORE / "lib.rs"
    lib = path.read_text()
    lib = once(lib, "mod events;\n", "mod events;\nmod old;\n", "mod events;")
    lib = once(lib, "    pub fn handle(&mut self, event: Event) -> Vec<Action> {",
               "    fn decide(&mut self, event: Event) -> Vec<Action> {", "pub fn handle")
    lib = once(lib, "    next_own_request: u64,\n}", "    next_own_request: u64,\n    shadow: old::Relay,\n}",
               "the last field of Relay")
    lib = once(lib, "            next_own_request: 0,\n        }",
               "            next_own_request: 0,\n            shadow: old::Relay::new(implementation),\n        }",
               "the last field of Relay::new")
    lib += '''
impl Relay {
    /// Feeds the event to both cores and reports where they differ.
    #[allow(clippy::all, clippy::pedantic, missing_docs)]
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let said = format!("{event:?}");
        let drops_a_session = matches!(event, Event::SessionClosed { .. });
        let before = self.shadow.handle(event.clone());
        let after = self.decide(event);
        let sorted = |actions: &[Action]| {
            let mut lines: Vec<String> = actions.iter().map(|a| format!("{a:?}")).collect();
            lines.sort();
            lines
        };
        // The actions for each session and each stream to a subscriber, in
        // the order they were pushed.
        let by_target = |actions: &[Action]| {
            let mut lines: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            for action in actions {
                if matches!(action, Action::StopDataStream { .. }) {
                    continue;
                }
                let target = match action.target() {
                    Target::Session(session) => format!("session {}", session.0),
                    Target::OutStream(out_stream) => format!("out {}", out_stream.0),
                    Target::Closing => "closing".to_string(),
                };
                lines.entry(target).or_default().push(format!("{action:?}"));
            }
            lines
        };
        let closes = before.iter().any(|a| matches!(a, Action::CloseSession { .. }));
        let held_before = format!("{:?}", self.shadow.counts());
        let held_after = format!("{:?}", self.counts());
        let kind = if sorted(&before) != sorted(&after) {
            Some("content")
        } else if held_before != held_after {
            Some("state")
        } else if !drops_a_session && !closes && by_target(&before) != by_target(&after) {
            Some("sequence")
        } else if before != after {
            Some("order")
        } else {
            None
        };
        if let Some(kind) = kind {
            let line = format!(
                "CORE_MISMATCH kind={kind} event={said} before={before:?} after={after:?} held_before={held_before} held_after={held_after}"
            );
            println!("{line}");
            assert!(kind == "order" || !cfg!(test), "{line}");
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
    path.write_text(once(path.read_text(), "mod events;\n", "mod events;\n#[cfg(test)]\nmod fuzz;\n", "mod events;"))
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
    wrong = [line for line in lines if "kind=order" not in line]
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
    if random_events:
        # WHY checked: a run in which the random events did not happen ends
        # with no difference found, and would read as a pass.
        runs = int(os.environ.get("XMOQ_FUZZ_RUNS", "20000"))
        length = int(os.environ.get("XMOQ_FUZZ_LEN", "80"))
        if not fuzz or (int(fuzz.group(1)), int(fuzz.group(2))) != (runs, runs * length):
            print(f"    THE RANDOM EVENTS DID NOT RUN: expected runs={runs} events={runs * length}")
            totals["failed"] += 1
    if not results and "error" in output and not lines and "RESULT" not in output:
        print(output[-3000:])
        totals["failed"] += 1


def main():
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
    _, output = cargo("test", "--locked", "-p", "xmoq-relay-core", "--", "--nocapture", "--test-threads", "1")
    report("tests of xmoq-relay-core and the random events", output, totals, random_events=True)
    _, output = cargo("test", "--locked", "-p", "xmoq-relay", "--", "--nocapture", "--test-threads", "1")
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
    wrong = totals["content"] or totals["state"] or totals["sequence"] or totals["failed"]
    sys.exit(1 if wrong else 0)


main()
