// Spike, outside lint and review: the forms of `WebTransportError`, the width of its
// code, and what `isConfigSupported` answers, in a Playwright build of a browser.
// Run from the repository root: bun spikes/browser-support/error-and-codecs.ts <chromium|firefox>
import { chromium, firefox } from "../../interop/node_modules/playwright-core/index.mjs";

const name = process.argv[2] ?? "";
if (name !== "chromium" && name !== "firefox") throw new Error("usage: error-and-codecs.ts <chromium|firefox>");
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response("<!DOCTYPE html><title>probe</title>", { headers: { "content-type": "text/html" } }) });
const browser = await (name === "chromium" ? chromium.launch({ channel: "chromium" }) : firefox.launch());
try {
	const page = await browser.newPage();
	await page.goto(`http://localhost:${server.port}/`);
	const result = await page.evaluate(async () => {
		const E = (globalThis as any).WebTransportError;
		const attempt = (label: string, make: () => any) => {
			try {
				const e = make();
				return `${label}: message=${JSON.stringify(e.message)} source=${e.source} streamErrorCode=${e.streamErrorCode}`;
			} catch (error) {
				return `${label}: threw ${(error as Error).name}: ${(error as Error).message}`;
			}
		};
		const errors = [
			attempt('new E("m", { streamErrorCode: 1 })', () => new E("m", { streamErrorCode: 1 })),
			attempt('new E({ message: "m", streamErrorCode: 1 })', () => new E({ message: "m", streamErrorCode: 1 })),
			attempt("new E()", () => new E()),
			...[255, 256, 65536, 4294967295, -1].map((c) => attempt(`new E({ streamErrorCode: ${c} })`, () => new E({ streamErrorCode: c }))),
		];
		const video = ["avc1.42E01F", "avc1.640033", "vp8", "vp09.00.31.08", "av01.0.05M.08", "hvc1.1.6.L93.B0"];
		const accel = ["no-preference", "prefer-hardware", "prefer-software"] as const;
		const yes = async (check: () => Promise<{ supported?: boolean }>) => {
			try { return (await check()).supported ? "yes" : "no"; } catch (error) { return `throws ${(error as Error).name}`; }
		};
		const codecs: string[] = [];
		for (const codec of video) {
			const formats = codec.startsWith("avc1") ? [{ avc: { format: "avc" } }, { avc: { format: "annexb" } }] : [{}];
			for (const format of formats) {
				const enc: string[] = [], dec: string[] = [];
				for (const hardwareAcceleration of accel) {
					enc.push(await yes(() => (globalThis as any).VideoEncoder.isConfigSupported({ codec, width: 1280, height: 720, bitrate: 2_764_800, framerate: 30, latencyMode: "realtime", hardwareAcceleration, ...format })));
					dec.push(await yes(() => (globalThis as any).VideoDecoder.isConfigSupported({ codec, codedWidth: 1280, codedHeight: 720, hardwareAcceleration })));
				}
				codecs.push(`${codec} ${JSON.stringify(format)} 1280x720 encode ${enc.join(" / ")} | decode ${dec.join(" / ")}`);
			}
		}
		for (const codec of ["opus", "mp4a.40.2"]) {
			const enc = await yes(() => (globalThis as any).AudioEncoder.isConfigSupported({ codec, sampleRate: 48000, numberOfChannels: 2, bitrate: 96000 }));
			const dec = await yes(() => (globalThis as any).AudioDecoder.isConfigSupported({ codec, sampleRate: 48000, numberOfChannels: 2 }));
			codecs.push(`${codec} encode ${enc} | decode ${dec}`);
		}
		return [...errors, ...codecs];
	});
	console.log(`${name} ${browser.version()}`);
	for (const line of result) console.log(`  ${line}`);
} finally {
	await browser.close();
	server.stop(true);
}
