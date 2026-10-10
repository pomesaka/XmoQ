"""Spike: what the OS does when a dual-stack UDP socket gets the port of an IPv4 one.

Binds 0.0.0.0:P, then [::]:P with IPV6_V6ONLY off, sends to 127.0.0.1:P and prints
which socket the datagram reached. Run: python3 bind_check.py
"""

import select
import socket

held = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
held.bind(("0.0.0.0", 0))
port = held.getsockname()[1]

dual = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
dual.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
try:
    dual.bind(("::", port))
except OSError as error:
    print(f"[::]:{port} refused while 0.0.0.0:{port} is held: {error}")
    raise SystemExit(0)
print(f"[::]:{port} bound while 0.0.0.0:{port} is held")

sender = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
for _ in range(5):
    sender.sendto(b"x", ("127.0.0.1", port))
readable, _, _ = select.select([held, dual], [], [], 0.5)
names = ["the IPv4 socket" if s is held else "the dual-stack socket" for s in readable]
print("a datagram to 127.0.0.1 reached:", ", ".join(names) or "neither")
