"""Reads `perf script` of the relay, demangled (stdin), and says of all samples how many have a
frame of each group on their stack (a sample can be in several). A frame is matched by its
path with the generic arguments taken out (`Hub<C>::run` is `Hub::run`), so that a closure
of the hub that a writer runs is the writer's.

  perf script -i perf.data -F ip,sym --no-inline | rustfilt | python3 perf-groups.py
"""
import collections, re, sys

GROUPS = [
    # The hub: one task for all sessions.
    ("the hub (one task for all sessions)", r"^xmoq_relay::hub::Hub::run"),
    ("  the core's decisions (xmoq_relay_core)", r"^xmoq_relay_core::"),
    ("  log lines: describe and format", r"^xmoq_relay::hub::describe|^xmoq_relay::hub::Hub::log\b|^alloc::fmt::format"),
    ("  log lines: queueing (Log::print)", r"^xmoq_relay::hub::Log::print"),
    ("  starting a stream's writer (open_data_stream)", r"^xmoq_relay::hub::Hub::open_data_stream$"),
    ("    OutgoingSubgroup::spawn", r"^xmoq_moqt_io::streams::OutgoingSubgroup::spawn$"),
    ("  sending an Object to a writer (send_object)", r"^xmoq_relay::hub::Hub::send_object"),
    ("  hash maps (SipHash)", r"^std::hash::random::|^core::hash::sip::|^hashbrown::"),
    ("the log's own task (println)", r"^std::io::stdio::_print"),
    ("the writer of a subgroup stream (one task each)", r"^xmoq_moqt_io::streams::Writer::run"),
    ("  its reports to the hub (open_data_stream's closure)", r"^xmoq_relay::hub::Hub::open_data_stream::\{closure"),
    ("  opening a stream (quinn OpenUni)", r"^<quinn::connection::OpenUni as|^quinn::connection::OpenUni"),
    ("quinn's connection driver (packets, crypto, acks)", r"ConnectionDriver as core::future::future::Future>::poll|^quinn_proto::connection::Connection::"),
    ("quinn's endpoint (receiving, routing)", r"^<?quinn::endpoint::|^quinn_udp::"),
    ("malloc and free", r"^(malloc|cfree|free|realloc|_int_malloc|_int_free|__libc_malloc|__libc_free|malloc_consolidate)$"),
    ("tokio's scheduler and wakers", r"tokio::runtime::scheduler::|tokio::runtime::task::"),
    ("tokio's sync (Notify, mpsc, watch)", r"tokio::sync::"),
]
# Which task a sample ran in: the future of the tokio task on its stack.
# The first that is in the task's name: a writer's has the hub's closures among its generics.
TASKS = [
    ("writers of subgroup streams", "xmoq_moqt_io::streams::OutgoingSubgroup>::spawn"),
    ("the hub", "xmoq_relay::hub::Hub<"),
    ("quinn's connection drivers", "ConnectionDriver"),
    ("quinn's endpoint", "quinn::endpoint"),
    ("the log's task", "xmoq_relay::hub::Log>::stdout"),
]
tasks = collections.Counter()
doing = collections.defaultdict(collections.Counter)
compiled = [(name, re.compile(rx)) for name, rx in GROUPS]
counts = {name: 0 for name, _ in GROUPS}
samples = 0
stack = []


def strip_generics(text):
    out, depth = [], 0
    for ch in text:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def own_path(symbol):
    """The path of a symbol without generic arguments: `<a::Hub<C>>::run::{closure#0}` is
    `a::Hub::run::{closure#0}`, `<a::D as b::Future>::poll` is `a::D as b::Future::poll`."""
    if not symbol.startswith("<"):
        return strip_generics(symbol)
    depth = 0
    for at, ch in enumerate(symbol):
        depth += ch == "<"
        depth -= ch == ">"
        if depth == 0:
            return strip_generics(symbol[1:at]) + strip_generics(symbol[at + 1:])
    return strip_generics(symbol)


def flush():
    global samples
    if not stack:
        return
    samples += 1
    paths = [own_path(s) for s in stack] + stack
    for name, rx in compiled:
        if any(rx.search(p) for p in paths):
            counts[name] += 1
    core = next((s for s in stack if s.startswith("<tokio::runtime::task::core::Core<")), None)
    if core is None:
        # Named by the outermost frame that perf unwound to.
        tasks["no task on the stack, outermost: " + own_path(stack[-1])[:70]] += 1
    else:
        # A boxed future (quinn spawns its drivers so) is named by what runs in it.
        named = core if "core::pin::Pin<alloc::boxed::Box<dyn" not in core else " ".join(stack)
        task = next((name for name, key in TASKS if key in named), "other tasks: " + own_path(core[34:])[:80])
        tasks[task] += 1
        # The first frame of this program's crates from the leaf: what it was doing.
        mine = next((own_path(s) for s in stack if re.match(r"<?xmoq_", s)), "(none of the relay's)")
        doing[task][mine[:110]] += 1
    stack.clear()


for line in sys.stdin:
    line = line.rstrip("\n")
    if not line.strip():
        flush()
        continue
    if line[:1] in "\t ":
        parts = line.strip().split(" ", 1)
        stack.append(parts[1] if len(parts) == 2 else parts[0])
    else:
        flush()
flush()
print(f"PERF_GROUPS samples={samples}")
for name, _ in GROUPS:
    print(f"{100 * counts[name] / max(samples, 1):5.1f}%  {name}")
print("PERF_TASKS")
for name, n in tasks.most_common(12):
    print(f"{100 * n / max(samples, 1):5.1f}%  {name}")
for task in ("writers of subgroup streams", "the hub"):
    print(f"PERF_IN {task}: the innermost function of the relay's crates")
    for name, n in doing[task].most_common(10):
        print(f"{100 * n / max(samples, 1):5.1f}%    {name}")
