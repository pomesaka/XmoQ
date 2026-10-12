#!/usr/bin/env python3
"""Runs a command on a pseudo-terminal and presses keys, for try-keys.sh.

    keys-on-a-pty.py <step>... -- <command>...

A step is `wait:<seconds>`, `keys:<text>` (\\r for Enter, \\x03 for Ctrl-C),
`signal:<name>:<pid>`, or `kill:<name>` for the command itself. Prints what the
terminal's line mode was before and after (ICANON and ECHO), how many bytes the
command wrote to the terminal, and how it ended.
"""
import os
import signal
import subprocess
import sys
import termios
import threading
import time

at = sys.argv.index("--")
steps, command = sys.argv[1:at], sys.argv[at + 1 :]
master, slave = os.openpty()


def line_mode():
    flags = termios.tcgetattr(slave)[3]
    return f"icanon={int(bool(flags & termios.ICANON))} echo={int(bool(flags & termios.ECHO))} isig={int(bool(flags & termios.ISIG))}"


before = line_mode()
child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
written = [0, b""]


def drain():
    while True:
        try:
            data = os.read(master, 1 << 20)
        except OSError:
            return
        if not data:
            return
        written[0] += len(data)
        written[1] = (written[1] + data)[-4000:]


threading.Thread(target=drain, daemon=True).start()
during = None
for step in steps:
    kind, _, rest = step.partition(":")
    if kind == "wait":
        time.sleep(float(rest))
        during = during or line_mode()
    elif kind == "keys":
        os.write(master, rest.encode().decode("unicode_escape").encode("latin-1"))
    elif kind == "signal":
        name, _, pid = rest.partition(":")
        os.kill(int(pid), getattr(signal, name))
    elif kind == "kill":
        child.send_signal(getattr(signal, rest))
try:
    code = child.wait(timeout=20)
except subprocess.TimeoutExpired:
    child.kill()
    code = "killed after 20 s"
time.sleep(0.2)
print(f"TERMINAL before: {before} | while watching: {during} | after: {line_mode()} | exit={code} bytes_to_terminal={written[0]}")
tail = written[1].decode("utf-8", "replace")
for line in tail.replace("\r", "\n").split("\n"):
    if "AV_PLAYED" in line or "panicked" in line or "Error" in line:
        print("ON_TERMINAL " + line.strip()[:160])
