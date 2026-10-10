// Spike, outside lint and review: which forms of `new WebTransportError(...)` the
// Chromium of the pinned playwright-core accepts
// (docs/experiments/2026-10-10-webtransport-error-in-chromium.md).
// Run: bun spikes/wt-error-form/constructor.ts
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response("<html></html>", { headers: { "content-type": "text/html" } }) });
const browser = await chromium.launch({ headless: true, channel: "chromium" });
const page = await browser.newPage();
await page.goto(`http://localhost:${server.port}/`);
const result = await page.evaluate(() => {
  const attempt = (label: string, make: () => any) => {
    try { const e = make(); return `${label}: ok message=${JSON.stringify(e.message)} source=${e.source} streamErrorCode=${e.streamErrorCode}`; }
    catch (error) { return `${label}: threw ${String(error)}`; }
  };
  const WTE: any = (globalThis as any).WebTransportError;
  return [
    attempt('new WebTransportError("m", { streamErrorCode: 1 })', () => new WTE("m", { streamErrorCode: 1 })),
    attempt('new WebTransportError({ message: "m", streamErrorCode: 1 })', () => new WTE({ message: "m", streamErrorCode: 1 })),
    attempt('new WebTransportError("m")', () => new WTE("m")),
    attempt('new WebTransportError()', () => new WTE()),
    ...[0, 255, 256, 65535, 65536, 4294967295, 4294967296, -1, 1.5].map((code) =>
      attempt(`new WebTransportError({ streamErrorCode: ${code} })`, () => new WTE({ streamErrorCode: code })),
    ),
    attempt('new WebTransportError({ source: "session" })', () => new WTE({ source: "session" })),
    `length=${WTE.length}`,
  ];
});
console.log(browser.version());
console.log(result.join("\n"));
await browser.close();
server.stop(true);
