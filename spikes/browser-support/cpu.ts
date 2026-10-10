// Spike, outside lint and review: the CPU time a Chromium spends while it encodes, and
// while it decodes, a moving picture with WebCodecs in real time (30 frames a second).
// No network and no MoQ. CPU time stands in for power, which is not measured.
// Run from the repository root:
//   bun spikes/browser-support/cpu.ts <chromium|chrome> [--seconds 120] [--out file.json] [--only <regex>]
// --only: the conditions whose label matches, to measure some of them again. Every row has
// the time it started, so that a row can be set against what else the machine did then.
// chromium: Playwright's Chrome for Testing. chrome: the Google Chrome that is installed.
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { chromium } from "../../interop/node_modules/playwright-core/index.mjs";

const { values, positionals } = parseArgs({
	allowPositionals: true,
	options: { seconds: { type: "string", default: "120" }, out: { type: "string" }, only: { type: "string" } },
});
const channel = positionals[0] ?? "";
if (channel !== "chromium" && channel !== "chrome") throw new Error("usage: cpu.ts <chromium|chrome>");
const SECONDS = Number(values.seconds);
const FPS = 30;

interface Condition {
	label: string;
	kind: "idle" | "draw" | "encode" | "decode";
	width: number;
	height: number;
	codec?: string;
	hardwareAcceleration?: "prefer-hardware" | "prefer-software" | "no-preference";
}
const SIZES = [{ width: 1280, height: 720 }, { width: 1920, height: 1080 }];
// The level 3.1 string is refused at 1920x1080 (loopback record), so level 5.1 for both sizes.
const CODECS: { name: string; codec: string; hardwareAcceleration: Condition["hardwareAcceleration"] }[] = [
	{ name: "H.264 hardware", codec: "avc1.640033", hardwareAcceleration: "prefer-hardware" },
	{ name: "H.264 software", codec: "avc1.640033", hardwareAcceleration: "prefer-software" },
	{ name: "VP8", codec: "vp8", hardwareAcceleration: "no-preference" },
	{ name: "VP9", codec: "vp09.00.40.08", hardwareAcceleration: "no-preference" },
	{ name: "AV1", codec: "av01.0.08M.08", hardwareAcceleration: "no-preference" },
];
const CONDITIONS: Condition[] = [
	{ label: "idle page", kind: "idle", width: 0, height: 0 },
	...SIZES.map((size): Condition => ({ label: `draw only ${size.width}x${size.height}`, kind: "draw", ...size })),
	...SIZES.flatMap((size) =>
		CODECS.map((c): Condition => ({ label: `encode ${c.name} ${size.width}x${size.height}`, kind: "encode", ...size, codec: c.codec, hardwareAcceleration: c.hardwareAcceleration })),
	),
	...CODECS.map((c): Condition => ({ label: `decode ${c.name} 1280x720`, kind: "decode", width: 1280, height: 720, codec: c.codec, hardwareAcceleration: c.hardwareAcceleration })),
	// With no preference, VP9 and AV1 were decoded by the platform; a viewer without that decodes in software.
	{ label: "decode VP9 software 1280x720", kind: "decode", width: 1280, height: 720, codec: "vp09.00.40.08", hardwareAcceleration: "prefer-software" },
	{ label: "decode AV1 software 1280x720", kind: "decode", width: 1280, height: 720, codec: "av01.0.08M.08", hardwareAcceleration: "prefer-software" },
];

interface PageResult {
	error?: string;
	frames: number;
	outputs: number;
	maxQueue: number;
	lateFrames: number;
	bytes: number;
	keyChunks: number;
	wallMs: number;
	prepareMs?: number;
}

/** Runs in the page. */
async function runInPage([condition, seconds, fps]: [Condition, number, number]): Promise<PageResult> {
	const g = globalThis as any;
	const result: PageResult = { frames: 0, outputs: 0, maxQueue: 0, lateFrames: 0, bytes: 0, keyChunks: 0, wallMs: 0 };
	const total = seconds * fps;
	const sleepUntil = (at: number) => new Promise<void>((resolve) => setTimeout(resolve, Math.max(0, at - performance.now())));
	if (condition.kind === "idle") {
		const start = performance.now();
		await sleepUntil(start + seconds * 1000);
		result.wallMs = performance.now() - start;
		return result;
	}
	const canvas = new g.OffscreenCanvas(condition.width, condition.height);
	const context = canvas.getContext("2d");
	// A picture that changes all over from frame to frame: the hue of the whole gradient
	// moves, a bar crosses, and a number counts. A still picture would give an encoder
	// next to nothing to do.
	const draw = (index: number) => {
		const gradient = context.createLinearGradient(0, 0, condition.width, condition.height);
		gradient.addColorStop(0, `hsl(${index * 4}, 80%, 50%)`);
		gradient.addColorStop(1, `hsl(${index * 4 + 120}, 80%, 30%)`);
		context.fillStyle = gradient;
		context.fillRect(0, 0, condition.width, condition.height);
		context.fillStyle = "white";
		context.fillRect((index * 13) % (condition.width - 40), 0, 40, condition.height);
		context.font = `${Math.round(condition.height / 3.6)}px sans-serif`;
		context.fillText(String(index), condition.width * 0.375, condition.height * 0.58);
	};
	let failure: string | undefined;
	const bitrate = condition.width * condition.height * 3;
	const makeEncoder = (onChunk: (chunk: any) => void) => {
		const encoder = new g.VideoEncoder({
			output: (chunk: any) => { result.outputs += 1; result.bytes += chunk.byteLength; if (chunk.type === "key") result.keyChunks += 1; onChunk(chunk); },
			error: (error: Error) => { failure = `${error.name}: ${error.message}`; },
		});
		encoder.configure({ codec: condition.codec, width: condition.width, height: condition.height, bitrate, framerate: fps, latencyMode: "realtime", hardwareAcceleration: condition.hardwareAcceleration, ...(condition.codec?.startsWith("avc1") ? { avc: { format: "annexb" } } : {}) });
		return encoder;
	};
	try {
		if (condition.kind === "draw" || condition.kind === "encode") {
			const encoder = condition.kind === "encode" ? makeEncoder(() => {}) : undefined;
			const start = performance.now();
			for (let index = 0; index < total && !failure; index += 1) {
				const due = start + (index * 1000) / fps;
				if (performance.now() - due > 1000 / fps) result.lateFrames += 1;
				await sleepUntil(due);
				draw(index);
				const frame = new g.VideoFrame(canvas, { timestamp: Math.round((index * 1_000_000) / fps) });
				if (encoder) {
					encoder.encode(frame, { keyFrame: index % fps === 0 });
					result.maxQueue = Math.max(result.maxQueue, encoder.encodeQueueSize);
				}
				frame.close();
				result.frames += 1;
			}
			if (encoder && !failure) await encoder.flush();
			result.wallMs = performance.now() - start;
			if (encoder && encoder.state !== "closed") encoder.close();
		} else {
			// Prepared as fast as it goes and not measured: the chunks to decode.
			const chunks: { type: string; timestamp: number; data: Uint8Array }[] = [];
			const prepared = performance.now();
			const encoder = makeEncoder((chunk) => { const data = new Uint8Array(chunk.byteLength); chunk.copyTo(data); chunks.push({ type: chunk.type, timestamp: chunk.timestamp, data }); });
			for (let index = 0; index < total && !failure; index += 1) {
				draw(index);
				const frame = new g.VideoFrame(canvas, { timestamp: Math.round((index * 1_000_000) / fps) });
				encoder.encode(frame, { keyFrame: index % fps === 0 });
				frame.close();
				if (encoder.encodeQueueSize > 8) await new Promise((resolve) => encoder.addEventListener("dequeue", resolve, { once: true }));
			}
			await encoder.flush();
			encoder.close();
			result.prepareMs = performance.now() - prepared;
			result.outputs = 0;
			result.bytes = chunks.reduce((sum, chunk) => sum + chunk.data.byteLength, 0);
			g.__chunks = chunks;
			result.frames = chunks.length;
			return result;
		}
	} catch (error) {
		failure = `${(error as Error).name}: ${(error as Error).message}`;
	}
	if (failure) result.error = failure;
	return result;
}

/** Runs in the page, after `runInPage` prepared the chunks: decodes them at 30 a second. */
async function decodeInPage([condition, fps]: [Condition, number]): Promise<PageResult> {
	const g = globalThis as any;
	const chunks: { type: string; timestamp: number; data: Uint8Array }[] = g.__chunks;
	const result: PageResult = { frames: 0, outputs: 0, maxQueue: 0, lateFrames: 0, bytes: 0, keyChunks: 0, wallMs: 0 };
	let failure: string | undefined;
	const decoder = new g.VideoDecoder({
		output: (frame: any) => { result.outputs += 1; frame.close(); },
		error: (error: Error) => { failure = `${error.name}: ${error.message}`; },
	});
	try {
		decoder.configure({ codec: condition.codec, optimizeForLatency: true, hardwareAcceleration: condition.hardwareAcceleration });
		const start = performance.now();
		for (let index = 0; index < chunks.length && !failure; index += 1) {
			const due = start + (index * 1000) / fps;
			if (performance.now() - due > 1000 / fps) result.lateFrames += 1;
			await new Promise<void>((resolve) => setTimeout(resolve, Math.max(0, due - performance.now())));
			const chunk = chunks[index]!;
			decoder.decode(new g.EncodedVideoChunk({ type: chunk.type, timestamp: chunk.timestamp, data: chunk.data }));
			result.maxQueue = Math.max(result.maxQueue, decoder.decodeQueueSize);
			result.frames += 1;
		}
		if (!failure) await decoder.flush();
		result.wallMs = performance.now() - start;
	} catch (error) {
		failure = `${(error as Error).name}: ${(error as Error).message}`;
	}
	if (decoder.state !== "closed") decoder.close();
	g.__chunks = undefined;
	if (failure) result.error = failure;
	return result;
}

const profile = mkdtempSync(join(tmpdir(), "xmoq-cpu-"));
/** CPU seconds of every process of this browser, by `--type`; the browser process itself is "browser". */
function cpuByType(): Record<string, number> {
	const lines = execFileSync("ps", ["-axo", "pid=,ppid=,time=,command="], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).split("\n");
	const rows = lines.map((line) => line.trim().match(/^(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/)).filter((m): m is RegExpMatchArray => m !== null).map((m) => ({ pid: m[1]!, ppid: m[2]!, time: m[3]!, command: m[4]! }));
	const roots = new Set(rows.filter((row) => row.command.includes(profile) && !row.command.includes("--type=")).map((row) => row.pid));
	const mine = new Set(roots);
	for (let grew = true; grew; ) {
		grew = false;
		for (const row of rows) if (!mine.has(row.pid) && mine.has(row.ppid)) { mine.add(row.pid); grew = true; }
	}
	const seconds = (time: string) => { const parts = time.split(":").map(Number); return parts.reduce((sum, part) => sum * 60 + part, 0); };
	const totals: Record<string, number> = {};
	for (const row of rows) {
		if (!mine.has(row.pid)) continue;
		const type = row.command.match(/--type=([a-z-]+)/)?.[1] ?? "browser";
		const sub = row.command.match(/--utility-sub-type=([\w.]+)/)?.[1];
		const key = sub ? `${type}:${sub.split(".").pop()}` : type;
		totals[key] = (totals[key] ?? 0) + seconds(row.time);
	}
	return totals;
}

const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response("<!DOCTYPE html><title>cpu</title>", { headers: { "content-type": "text/html" } }) });
const context = await chromium.launchPersistentContext(profile, { channel, headless: true });
const results: unknown[] = [];
try {
	const page = context.pages()[0] ?? (await context.newPage());
	page.setDefaultTimeout((SECONDS + 600) * 1000);
	await page.goto(`http://localhost:${server.port}/`);
	const version = context.browser()?.version() ?? (await page.evaluate(() => navigator.userAgent));
	// The encoder and decoder a WebCodecs object really got, from the DevTools Media domain.
	const cdp = await context.newCDPSession(page);
	const media: Record<string, string> = {};
	cdp.on("Media.playerPropertiesChanged", (event: { properties: { name: string; value: string }[] }) => {
		for (const property of event.properties) if (/EncoderName|DecoderName|IsPlatform/.test(property.name)) media[property.name] = property.value;
	});
	await cdp.send("Media.enable");
	const gpu = await page.evaluate(() => {
		const gl = document.createElement("canvas").getContext("webgl");
		const info = gl?.getExtension("WEBGL_debug_renderer_info");
		return gl && info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : "no WebGL";
	});
	console.log(`${channel} ${version}; WebGL renderer: ${gpu}; ${SECONDS} s a condition`);
	const only = values.only === undefined ? undefined : new RegExp(values.only);
	for (const condition of CONDITIONS) {
		if (only !== undefined && !only.test(condition.label)) continue;
		for (const key of Object.keys(media)) delete media[key];
		let prepared: PageResult | undefined;
		if (condition.kind === "decode") prepared = await page.evaluate(runInPage, [condition, SECONDS, FPS] as [Condition, number, number]);
		await new Promise((resolve) => setTimeout(resolve, 2000));
		const before = cpuByType();
		const startedAt = new Date().toISOString();
		const started = performance.now();
		const out = condition.kind === "decode"
			? prepared?.error ? prepared : await page.evaluate(decodeInPage, [condition, FPS] as [Condition, number])
			: await page.evaluate(runInPage, [condition, SECONDS, FPS] as [Condition, number, number]);
		const wall = (performance.now() - started) / 1000;
		const after = cpuByType();
		const delta: Record<string, number> = {};
		for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) delta[key] = Math.round(((after[key] ?? 0) - (before[key] ?? 0)) * 100) / 100;
		const cpu = Object.values(delta).reduce((sum, value) => sum + value, 0);
		const row = { condition: condition.label, startedAt, wallSeconds: Math.round(wall * 10) / 10, cpuSeconds: Math.round(cpu * 100) / 100, percentOfOneCore: Math.round((cpu / wall) * 1000) / 10, byProcess: delta, media: { ...media }, page: out };
		results.push(row);
		console.log(JSON.stringify(row));
	}
	if (values.out) writeFileSync(values.out, JSON.stringify({ channel, version, gpu, seconds: SECONDS, results }, null, 1));
} finally {
	await context.close();
	server.stop(true);
	rmSync(profile, { recursive: true, force: true });
}
