#!/bin/sh
# A demo of its own with authorization and rooms on other ports, its publishing page in
# headless Chromium in a room, and xmoq-watch joining that room: the room says where the
# broadcast is and gives the token. The viewer's --say goes to the chat's tracks (the
# room carries no chat), which show nothing here: no gatherer (xmoq-chat) is started.
# Then once more without a token, which the relay is to refuse.
#
#   crates/watch/scripts/try-room.sh [seconds, 15]            # from the repository root
#
# Needs `mise run interop` once (Playwright's Chromium); the rooms' dependencies are the
# workspace's, installed here as `mise run demo` does.
# PORT (48910) and two more are used.
set -eu
seconds=${1:-15}
root=$(cd "$(dirname "$0")/../../.." && pwd)
here="$root/crates/watch/scripts"
port=${PORT:-48910}
name="try-$$"
work=$(mktemp -d)
(cd "$root" && cargo build --release --quiet --locked -p xmoq-watch && cargo build --quiet --locked -p xmoq-relay)
# The rooms' dependencies, Miniflare among them, are the workspace's.
(cd "$root" && mise exec -- bun install --frozen-lockfile > /dev/null)
# WHY the binary and not `mise exec`: the signal at the end has to reach bun itself.
bun=$(cd "$root/interop" && mise which bun)
(cd "$root" && exec "$bun" demo/serve.ts --auth --relay-port "$port" --port "$((port + 1))" --rooms-port "$((port + 2))") > "$work/demo.log" 2>&1 &
demo=$!
trap 'kill -INT "${page:-}" 2>/dev/null || true; kill -INT "$demo" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT
for _ in $(seq 1 60); do
  grep -q "^auth:" "$work/demo.log" && break
  sleep 0.5
done
grep -E "^(rooms|auth):" "$work/demo.log" >&2 || { cat "$work/demo.log" >&2; exit 1; }
(cd "$root" && exec "$bun" "$here/publish-headless.ts" "http://localhost:$((port + 1))" "$name" "hello from the page") > "$work/page.log" 2>&1 &
page=$!
for _ in $(seq 1 40); do
  grep -q "^PUBLISHING" "$work/page.log" && break
  sleep 0.5
done
room="http://127.0.0.1:$((port + 2))/rooms/$name"
echo "--- with a token of the room"
"$root/target/release/xmoq-watch" --room "$room" --say "hello from the terminal" \
  --seconds "$seconds" --volume 0 --medium direct --out "${OUT:-/dev/null}" 2>&1 | grep -E "ROOM_|CATALOG|CHAT|AV_PLAYED|SESSION_ENDED|Error" || true
echo "--- without a token"
"$root/target/release/xmoq-watch" --room "$room" --token no \
  --seconds 5 --volume 0 --medium direct --out /dev/null 2>&1 | grep -E "ROOM_|AV_PLAYED|SESSION_ENDED|Error" || true
echo "--- the page"
kill -INT "$page"
wait "$page" 2>/dev/null || true
page=
cat "$work/page.log"
