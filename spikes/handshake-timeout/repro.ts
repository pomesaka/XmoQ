// Spike, outside lint and review: looks for the WebTransport handshake that never
// ends (docs/experiments/2026-10-10-client-handshake-timeout.md).
//
// Needs `cargo build --locked -p xmoq-moqt-client -p xmoq-relay` and the certificate
// that an interop run creates under interop/.cert.
//
// It connects own-rust clients to one own relay over and over and reports the ones that
// do not get through. Usage: bun repro.ts --port N --count N [--parallel K] [--killed K] [--relays R]
//   --killed K: before every 20th batch, K publishers on other tracks are connected and
//     SIGKILLed, so that the relay holds sessions that are about to time out.
//   --limit MS: how long one client gets before it is counted as stuck and killed.
// Writes <out>.relay.log (the relay's lines with seconds since the start) and
// <out>.failures.json.
//   --relays R: R more idle relays run beside, as load.
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
const W = join(import.meta.dir, "..", "..");
const { values: o } = parseArgs({ options: {
  port: { type: "string", default: "45000" }, count: { type: "string", default: "500" },
  parallel: { type: "string", default: "1" }, killed: { type: "string", default: "0" },
  relays: { type: "string", default: "0" }, limit: { type: "string", default: "4000" },
  env: { type: "string", default: "" }, out: { type: "string", default: join(import.meta.dir, "out") },
} });
const cert = W + "/interop/.cert/cert.pem", key = W + "/interop/.cert/key.pem";
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const relayBin = W + "/target/debug/xmoq-relay", clientBin = W + "/target/debug/xmoq-moqt-client";
const env = { ...process.env, ...Object.fromEntries(o.env.split(",").filter(Boolean).map((p) => p.split("="))) };
const t0 = performance.now();
const now = () => ((performance.now() - t0) / 1000).toFixed(3);
function startRelay(port: number, logTo: string[] | null) {
  const child = spawn(relayBin, ["--port", String(port), "--cert-file", cert, "--key-file", key], { env });
  let ready: () => void; const up = new Promise<void>((r) => (ready = r));
  for (const s of [child.stdout, child.stderr]) { s.setEncoding("utf8"); let b = ""; s.on("data", (c: string) => { b += c; const p = b.split("\n"); b = p.pop() ?? ""; for (const l of p) { if (l.startsWith("LISTENING")) ready(); logTo?.push(now() + " " + l); } }); }
  return { child, up };
}
const relayLog: string[] = [];
const relay = startRelay(Number(o.port), relayLog);
const extra = Array.from({ length: Number(o.relays) }, (_, i) => startRelay(Number(o.port) + 1 + i, null));
await Promise.all([relay.up, ...extra.map((e) => e.up)]);
const url = "https://127.0.0.1:" + o.port;
function client(args: string[]) {
  return spawn(clientBin, ["--url", url, "--cert-sha256", sha, "--namespace", "xmoq/repro", "--timeout-secs", "60", ...args], { env });
}
/** One subscriber for a track nobody published: connects, is refused, exits. Resolves with ms, or with what it printed when it took too long. */
function attempt(id: number): Promise<{ id: number; ms: number; ok: boolean; out: string; started: string }> {
  return new Promise((resolve) => {
    const started = now(); const began = performance.now();
    const c = client(["--mode", "subscribe", "--track", "missing"]);
    let out = "";
    for (const s of [c.stdout, c.stderr]) { s.setEncoding("utf8"); s.on("data", (d: string) => (out += d)); }
    const timer = setTimeout(() => { c.kill("SIGKILL"); }, Number(o.limit));
    c.once("exit", () => { clearTimeout(timer); const ms = Math.round(performance.now() - began); resolve({ id, ms, ok: /SUBSCRIBE rejected: error_code=0x10/.test(out), out, started }); });
  });
}
async function killedPublisher(i: number) {
  const c = client(["--mode", "publish", "--track", "killed" + i + "-" + Math.random(), "--payload-hex", "00"]);
  await new Promise<void>((resolve) => { let out = ""; c.stdout.setEncoding("utf8"); c.stdout.on("data", (d: string) => { out += d; if (out.includes("PUBLISHED")) resolve(); }); c.once("exit", () => resolve()); });
  c.kill("SIGKILL");
}
const failures: unknown[] = []; let done = 0; let slowest = 0;
const count = Number(o.count), parallel = Number(o.parallel);
for (let i = 0; i < count; i += parallel) {
  if ((i / parallel) % 20 === 0) for (let k = 0; k < Number(o.killed); k++) await killedPublisher(k);
  const batch = await Promise.all(Array.from({ length: Math.min(parallel, count - i) }, (_, j) => attempt(i + j)));
  for (const r of batch) { done++; slowest = Math.max(slowest, r.ms); if (!r.ok) { failures.push({ ...r, finished: now() }); console.log("FAILURE", JSON.stringify(r)); } }
}
console.log("attempts=" + done + " failures=" + failures.length + " slowest_ok_or_not_ms=" + slowest + " seconds=" + now());
writeFileSync(o.out + ".relay.log", relayLog.join("\n"));
writeFileSync(o.out + ".failures.json", JSON.stringify(failures, null, 1));
for (const r of [relay, ...extra]) r.child.kill("SIGINT");
