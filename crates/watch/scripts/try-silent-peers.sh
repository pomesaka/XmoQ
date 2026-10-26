#!/bin/sh
# Peers that do not answer: a room that takes the connection and says nothing, and a relay
# nobody is behind. The viewer has to end when it is told to (--seconds, SIGTERM) while it
# waits for either, and of itself after its own limit.
#
#   crates/watch/scripts/try-silent-peers.sh            # from the repository root
#
# Needs nothing but Python. PORT (48940) and one more are used. Prints one line a case: how the
# viewer ended and after how long.
set -eu
root=$(cd "$(dirname "$0")/../../.." && pwd)
(cd "$root" && cargo build --release --quiet --locked -p xmoq-watch)
watch="$root/target/release/xmoq-watch"
port=${PORT:-48940}
python3 -c "
import socket, time
listener = socket.socket(); listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(('127.0.0.1', $port)); listener.listen(8)
held = []
while True:
    held.append(listener.accept()[0])
" &
silent=$!
trap 'kill "$silent" 2>/dev/null || true' EXIT
sleep 0.5
room="http://127.0.0.1:$port/broadcasts/x"
nobody="https://127.0.0.1:$((port + 1))/"
zeros=0000000000000000000000000000000000000000000000000000000000000000

timed() { # <what> <seconds before SIGTERM, or -> <flags of xmoq-watch...>
  what=$1; term=$2; shift 2
  began=$(python3 -c "import time; print(time.time())")
  "$watch" "$@" --sound off --out /dev/null > /dev/null 2> "$root/target/.try-silent.err" &
  viewer=$!
  if [ "$term" != - ]; then sleep "$term"; kill -TERM "$viewer"; fi
  code=0; wait "$viewer" || code=$?
  python3 -c "import time; print('%-58s exit=%s after %.1f s  %s' % ('$what', $code, time.time() - $began, open('$root/target/.try-silent.err').read().strip()[:80]))"
  rm -f "$root/target/.try-silent.err"
}

timed "a silent room, --seconds 2" - --room "$room" --seconds 2
timed "a silent room, SIGTERM after 1 s" 1 --room "$room"
timed "a silent room, nothing to end it (its limit is --room-wait-ms, 5 s)" - --room "$room"
timed "a relay nobody is behind, --seconds 2" - --url "$nobody" --cert-sha256 "$zeros" --seconds 2
timed "a relay nobody is behind, SIGTERM after 1 s" 1 --url "$nobody" --cert-sha256 "$zeros"
timed "a relay nobody is behind, nothing to end it (10 s)" - --url "$nobody" --cert-sha256 "$zeros"
