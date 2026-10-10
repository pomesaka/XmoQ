// Spike, outside lint and review: whether the code of a reset and of a session close
// by @xmoq/moqt in Chromium reaches the own relay and, through it, an own-rust subscriber.
// Needs `cargo build --locked -p xmoq-moqt-client -p xmoq-relay` and the certificate
// that an interop run creates under interop/.cert.
// Run: bun spikes/wt-error-form/codes.ts
import { join } from "node:path";
import { spawn } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";
const W = join(import.meta.dir, "..", ".."); const port = "45200";
const cert = W + "/interop/.cert/cert.pem", key = W + "/interop/.cert/key.pem";
const sha = createHash("sha256").update(new X509Certificate(readFileSync(cert)).raw).digest("hex");
const lines: string[] = [];
function run(name: string, cmd: string, args: string[]) { const c = spawn(cmd, args); for (const s of [c.stdout, c.stderr]) { s.setEncoding("utf8"); s.on("data", (d: string) => d.split("\n").filter(Boolean).forEach((l) => { lines.push("[" + name + "] " + l); })); } return c; }
const waitFor = async (re: RegExp) => { for (let i = 0; i < 100; i++) { const l = lines.find((x) => re.test(x)); if (l) return l; await new Promise((r) => setTimeout(r, 100)); } throw new Error("no line " + re); };
const relay = run("relay", W + "/target/debug/xmoq-relay", ["--port", port, "--cert-file", cert, "--key-file", key]);
await waitFor(/LISTENING/);
const build = await Bun.build({ entrypoints: [join(import.meta.dir, "page.ts")], target: "browser" });
const js = await build.outputs[0].text();
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: (q) => new URL(q.url).pathname === "/page.js" ? new Response(js, { headers: { "content-type": "text/javascript" } }) : new Response("<script type=module src=/page.js></script>", { headers: { "content-type": "text/html" } }) });
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const page = await browser.newPage();
page.on("pageerror", (e) => lines.push("[page] error " + e.message));
await page.goto("http://localhost:" + server.port + "/");
await page.waitForFunction(() => typeof (globalThis as any).probe === "function");
console.log(await page.evaluate(([u, h]) => (globalThis as any).probe(u, h), ["https://127.0.0.1:" + port, sha]));
const sub = run("sub", W + "/target/debug/xmoq-moqt-client", ["--mode", "subscribe", "--url", "https://127.0.0.1:" + port, "--cert-sha256", sha, "--namespace", "xmoq/probe", "--track", "object", "--objects", "5", "--timeout-secs", "30"]);
await waitFor(/\[sub\] SUBSCRIBED/);
console.log(await page.evaluate(() => (globalThis as any).resetWith(0x12)));
console.log(await waitFor(/\[sub\] STREAM_ABORTED/));
console.log("closed resolved with:", await page.evaluate(() => (globalThis as any).closeWith(5, "bye from the page")));
console.log(await waitFor(/\[relay\] CONNECTION_ENDED session=0/));
console.log(lines.filter((l) => /RESET_DATA|STOP_DATA|error/.test(l)).join("\n"));
sub.kill("SIGKILL"); await browser.close(); server.stop(true); relay.kill("SIGINT");
