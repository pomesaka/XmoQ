#!/bin/sh
# The keys of xmoq-watch on a pseudo-terminal, against a demo of its own with rooms: the volume
# and mute, a line typed for the chat, the page loaded anew (its broadcast ends and begins
# again) and the viewer following it, `q`; then the ends that are not `q`: Ctrl-C and SIGTERM.
# After each the terminal's line mode is read back.
#
#   demo/broadcast/scripts/try-keys.sh            # from the repository root
#
# Needs what try-room.sh needs. PORT (48920) and two more are used. Nobody looks at the
# pseudo-terminal: what is checked is the log of the viewer, the page's chat, and the mode.
set -eu
root=$(cd "$(dirname "$0")/../../.." && pwd)
here="$root/demo/broadcast/scripts"
port=${PORT:-48920}
name="keys-$$"
work=$(mktemp -d)
(cd "$root" && cargo build --release --quiet --locked -p xmoq-watch && cargo build --quiet --locked -p xmoq-relay)
# The rooms' dependencies, Miniflare among them, are the workspace's.
(cd "$root" && mise exec -- bun install --frozen-lockfile > /dev/null)
bun=$(cd "$root/interop" && mise which bun)
(cd "$root" && exec "$bun" demo/serve.ts --relay-port "$port" --port "$((port + 1))" --rooms-port "$((port + 2))") > "$work/demo.log" 2>&1 &
demo=$!
trap 'kill -INT "${page:-}" 2>/dev/null || true; kill -INT "$demo" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT
for _ in $(seq 1 60); do
  grep -q "^auth:" "$work/demo.log" && break
  sleep 0.5
done
(cd "$root" && exec "$bun" "$here/publish-headless.ts" "http://localhost:$((port + 1))" "$name") > "$work/page.log" 2>&1 &
page=$!
for _ in $(seq 1 40); do
  grep -q "^PUBLISHING" "$work/page.log" && break
  sleep 0.5
done
watch="$root/target/release/xmoq-watch"
room="http://127.0.0.1:$((port + 2))/broadcasts/$name"
echo "--- keys: + + m m - , a line for the chat, the page loaded anew, q"
python3 "$here/keys-on-a-pty.py" wait:4 keys:++ wait:0.5 keys:m wait:0.5 keys:m wait:0.5 keys:- wait:0.5 \
  'keys:ttyped in the termx\x7fnal\r' wait:2 "signal:SIGUSR1:$page" wait:9 keys:q -- \
  "$watch" --room "$room" --volume 0 --medium direct --log "$work/watch.log"
grep -vE "^AV_PLAYED" "$work/watch.log" || true
grep -oE "AV_PLAYED why=\"[^\"]*\" seconds=[0-9.]+|video_drawn=[0-9]+|underruns=[0-9]+" "$work/watch.log" | tr '\n' ' ' || true
echo
echo "--- Ctrl-C"
python3 "$here/keys-on-a-pty.py" wait:3 'keys:\x03' -- "$watch" --room "$room" --volume 0 --medium direct --log "$work/c.log"
echo "--- SIGTERM"
python3 "$here/keys-on-a-pty.py" wait:3 kill:SIGTERM -- "$watch" --room "$room" --volume 0 --medium direct --log "$work/t.log"
echo "--- the page"
kill -INT "$page"
wait "$page" 2>/dev/null || true
page=
cat "$work/page.log"
