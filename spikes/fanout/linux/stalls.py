"""Sleeps 5 ms again and again and writes, every second, how much later than asked it woke at
the most and in all: a machine whose cores are taken from it (a VM its host does not run) shows
here as long oversleeps, with nothing busy inside it.

  python3 stalls.py <seconds> > stalls.txt
"""
import sys, time

seconds = float(sys.argv[1])
end = time.monotonic() + seconds
second, worst, total = time.monotonic(), 0.0, 0.0
while time.monotonic() < end:
    asked = time.monotonic()
    time.sleep(0.005)
    late = time.monotonic() - asked - 0.005
    worst, total = max(worst, late), total + max(late, 0)
    if time.monotonic() - second >= 1:
        print(f"STALL at={time.time():.0f} worst_ms={worst * 1000:.1f} late_ms_in_all={total * 1000:.1f}", flush=True)
        second, worst, total = time.monotonic(), 0.0, 0.0
