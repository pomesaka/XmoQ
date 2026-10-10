"""Does bind(port 0) pick a port that an IPv4 UDP socket already holds?

Holds N IPv4 sockets on 0.0.0.0 with kernel-chosen ports, then binds a fresh
socket to port 0 TRIES times (closing each) and counts how often the kernel
chose a held port, for a dual-stack [::] socket and for a 0.0.0.0 socket.
"""
import resource, socket, sys

HELD, TRIES = int(sys.argv[1]), int(sys.argv[2])
resource.setrlimit(resource.RLIMIT_NOFILE, (HELD + 256, resource.getrlimit(resource.RLIMIT_NOFILE)[1]))
held = []
for _ in range(HELD):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("0.0.0.0", 0))
    held.append(s)
ports = {s.getsockname()[1] for s in held}
print(f"held: {len(ports)} IPv4 sockets, ports {min(ports)}..{max(ports)}")

def count(family, address, dual):
    hits = errors = 0
    seen = set()
    for _ in range(TRIES):
        s = socket.socket(family, socket.SOCK_DGRAM)
        if dual:
            s.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        try:
            s.bind((address, 0))
            port = s.getsockname()[1]
            seen.add(port)
            hits += port in ports
        except OSError:
            errors += 1
        s.close()
    return hits, errors, len(seen)

for name, args in [("[::]:0 dual-stack", (socket.AF_INET6, "::", True)), ("0.0.0.0:0", (socket.AF_INET, "0.0.0.0", False))]:
    hits, errors, distinct = count(*args)
    print(f"{name}: {hits} of {TRIES} binds got a held port ({errors} bind errors, {distinct} distinct ports)")
