#!/bin/sh
# xmoq-watch on Linux against moqlivemock's publisher (mlmpub v0.16.1 for linux/arm64), as
# interop/watch-rust.ts --watch from-mlmpub does: 8 s with no terminal and no sound output;
# then the same with a pseudo-terminal (script) as its terminal; then with the sound on.
# Started by watch-run.sh in a container.
set -u
bin=/target/release
out=/spike/work/logs/watch
mkdir -p "$out"
/spike/work/mlmpub-linux -addr 127.0.0.1:48950 -sideport 49950 -asset /assets/test10s -qlog "$out/mlmpub.qlog" > "$out/mlmpub.log" 2>&1 &
mlmpub=$!
trap 'case "$mlmpub" in ""|0|*[!0-9]*) ;; *) kill "$mlmpub" 2>/dev/null;; esac' EXIT
sha=
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  sha=$(python3 -c 'import urllib.request; print(urllib.request.urlopen("http://127.0.0.1:49950/fingerprint", timeout=1).read().decode().strip())' 2>/dev/null) && [ -n "$sha" ] && break
  sleep 0.5
done
common="--url https://127.0.0.1:48950/moq --cert-sha256 $sha --namespace mlm/msf/clear --seconds 8"
# shellcheck disable=SC2086
"$bin/xmoq-watch" $common --log "$out/headless.log" --sound off --out /dev/null --medium direct --fit none > "$out/headless.out" 2>&1
echo "HEADLESS exit=$?"; grep -h '^CATALOG\|^AV_PLAYED' "$out/headless.out" "$out/headless.log" 2>/dev/null
TERM=xterm-kitty script -qec "$bin/xmoq-watch $common --log $out/pty.log --sound off --fit 80x24" "$out/pty.typescript" > /dev/null 2>&1
echo "PTY exit=$? typescript_bytes=$(wc -c < "$out/pty.typescript")"; grep -h '^CATALOG\|^AV_PLAYED' "$out/pty.log" 2>/dev/null
# shellcheck disable=SC2086
"$bin/xmoq-watch" $common --log "$out/sound.log" --out /dev/null --medium direct --fit none > "$out/sound.out" 2>&1
echo "SOUND exit=$?"; tail -3 "$out/sound.out"; grep -h '^AV_PLAYED' "$out/sound.log" 2>/dev/null
