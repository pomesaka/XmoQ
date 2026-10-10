#!/usr/bin/env python3
"""Runs two versions of the relay core side by side and counts where they differ.

    python3 spikes/relay-core-diff/compare.py <before> <after> [--spikes] [--mutate]

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

It prints how many events led to different actions ("content") and how many
to the same actions in another order ("order"), and exits with 1 when any
differed in content. Nothing outside `spikes/relay-core-diff/.work` is written.

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
    lib = re.sub(r"#\[cfg\(test\)\]\nmod tests;\n", "#[cfg(test)]\nmod fuzz;\n", lib, count=1)
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
    /// Feeds the event to both cores and reports where their actions differ.
    #[allow(clippy::all, clippy::pedantic, missing_docs)]
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let said = format!("{event:?}");
        let before = self.shadow.handle(event.clone());
        let after = self.decide(event);
        if before != after {
            let sorted = |actions: &[Action]| {
                let mut lines: Vec<String> = actions.iter().map(|a| format!("{a:?}")).collect();
                lines.sort();
                lines
            };
            let kind = if sorted(&before) == sorted(&after) { "order" } else { "content" };
            let line = format!("CORE_MISMATCH kind={kind} event={said} before={before:?} after={after:?}");
            println!("{line}");
            assert!(kind == "order" || !cfg!(test), "{line}");
        }
        after
    }
}
'''
    path.write_text(lib)


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


def report(name, output, totals):
    lines = [line for line in output.splitlines() if "CORE_MISMATCH" in line]
    content = [line for line in lines if "kind=content" in line]
    order = [line for line in lines if "kind=order" in line]
    results = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed", output)
    fuzz = re.search(r"FUZZ runs=(\d+) events=(\d+)", output)
    passed = sum(int(p) for _, p, _ in results)
    failed = sum(int(f) for _, _, f in results)
    print(f"{name}: content={len(content)} order={len(order)}"
          + (f" tests passed={passed} failed={failed}" if results else "")
          + (f" random events={fuzz.group(2)}" if fuzz else ""))
    for cause, count in collections.Counter(order_cause(line) for line in order).most_common():
        print(f"    order: {count} x {cause}")
    for line in content[:3]:
        print("    " + line[:1500])
    totals["content"] += len(content)
    totals["order"] += len(order)
    totals["failed"] += failed
    if not results and "error" in output and not lines and "RESULT" not in output:
        print(output[-3000:])
        totals["failed"] += 1


def main():
    args = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
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
    report("tests of xmoq-relay-core and the random events", output, totals)
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
    print(f"TOTAL content={totals['content']} order={totals['order']} failed={totals['failed']}")
    sys.exit(1 if totals["content"] or totals["failed"] else 0)


main()
