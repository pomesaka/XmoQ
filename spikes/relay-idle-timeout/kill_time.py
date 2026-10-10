"""How long the own relay takes to notice a client that was killed.

Starts the relay, connects one own client in publish mode, waits `quiet`
seconds, kills the client with SIGKILL and measures the time until the relay
prints CONNECTION_ENDED. Usage: kill_time.py <port> <runs> <quiet> [relay args]
"""
import hashlib, re, ssl, subprocess, sys, time

port, runs, quiet = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
extra = sys.argv[4:]
cert, key = "spikes/moqtail/cert/cert.pem", "spikes/moqtail/cert/key.pem"
sha = hashlib.sha256(ssl.PEM_cert_to_DER_cert(open(cert).read())).hexdigest()
relay = subprocess.Popen(["target/debug/xmoq-relay", "--port", port, "--cert-file", cert, "--key-file", key, *extra],
                         stdout=subprocess.PIPE, text=True)
def wait_for(pattern):
    while True:
        line = relay.stdout.readline()
        if not line:
            raise SystemExit("the relay ended")
        if re.search(pattern, line):
            return line.strip()
wait_for("^LISTENING")
times = []
try:
    for _ in range(runs):
        client = subprocess.Popen(["target/debug/xmoq-moqt-client", "--mode", "publish", "--url", f"https://127.0.0.1:{port}",
                                   "--cert-sha256", sha, "--namespace", "xmoq/kill", "--track", "t", "--payload-hex", "00",
                                   "--timeout-secs", "120"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        while "PUBLISHED" not in client.stdout.readline():
            pass
        time.sleep(quiet)
        client.kill()
        killed = time.monotonic()
        line = wait_for("^CONNECTION_ENDED")
        times.append(time.monotonic() - killed)
        client.wait()
        print(f"{times[-1]:.2f}s  {line}")
finally:
    relay.kill()
    relay.wait()
print("quiet", quiet, "runs", [f"{t:.1f}" for t in times])
