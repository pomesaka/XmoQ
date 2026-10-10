// Bundles page.ts, serves it on 127.0.0.1 and drives the experiments in a real Chromium.
//
//   CHROMIUM_PATH=<binary> bun run.ts --label chromium-headless [--headed] [--chrome-arg=--flag ...]
//
// Writes out/<label>.json. `bun report.ts out/<label>.json` turns it into Markdown tables.
// Playwright's own browser download is not used: CHROMIUM_PATH points at an existing binary.
import { mkdir } from "node:fs/promises";
import { parseArgs } from "node:util";
import { chromium, type Page } from "playwright-core";
import {
	type AudioRunParams,
	type AudioRunResult,
	type BitstreamFormat,
	type CodecCandidate,
	type CodecFamily,
	type HardwarePreference,
	type Resolution,
	type RunFile,
	type SupportRow,
	type VideoRunEntry,
	type VideoRunParams,
	provisionalBitrate,
} from "./types";

const { values } = parseArgs({
	options: {
		label: { type: "string" },
		headed: { type: "boolean", default: false },
		// Extra Chromium command-line switches, repeatable.
		"chrome-arg": { type: "string", multiple: true, default: [] },
		// Frames per loopback run.
		frames: { type: "string", default: "90" },
		// Seconds one experiment may take before it is recorded as a failure.
		timeout: { type: "string", default: "300" },
		"skip-audio": { type: "boolean", default: false },
	},
});

const label = values.label;
if (!label) throw new Error("--label is required");
const executablePath = process.env.CHROMIUM_PATH;
if (!executablePath) throw new Error("CHROMIUM_PATH is required");

// Everything below is a provisional value chosen to make this check run. None is a project setting.
const FRAMERATE = 30;
const KEY_FRAME_INTERVAL = 30;
const MAIN_RESOLUTION: Resolution = { width: 1280, height: 720 };
const EXTRA_RESOLUTIONS: Resolution[] = [
	{ width: 640, height: 360 },
	{ width: 1920, height: 1080 },
];
const PROBE_RESOLUTIONS: Resolution[] = [
	{ width: 640, height: 360 },
	{ width: 1280, height: 720 },
	{ width: 1920, height: 1080 },
	{ width: 3840, height: 2160 },
];
const CANDIDATES: CodecCandidate[] = [
	{ family: "h264", codec: "avc1.42001f", note: "Baseline, level 3.1" },
	{ family: "h264", codec: "avc1.42E01F", note: "Constrained Baseline, level 3.1" },
	{ family: "h264", codec: "avc1.4D401F", note: "Main, level 3.1" },
	{ family: "h264", codec: "avc1.64001F", note: "High, level 3.1" },
	{ family: "h264", codec: "avc1.640033", note: "High, level 5.1" },
	{ family: "h264", codec: "avc3.42E01F", note: "Constrained Baseline, level 3.1, avc3" },
	{ family: "hevc", codec: "hvc1.1.6.L93.B0", note: "Main, level 3.1" },
	{ family: "hevc", codec: "hev1.1.6.L93.B0", note: "Main, level 3.1, hev1" },
	{ family: "hevc", codec: "hvc1.1.6.L153.B0", note: "Main, level 5.1" },
	{ family: "hevc", codec: "hvc1.2.4.L93.B0", note: "Main 10, level 3.1" },
	{ family: "vp8", codec: "vp8", note: "" },
	{ family: "vp9", codec: "vp09.00.31.08", note: "Profile 0, level 3.1, 8 bit" },
	{ family: "vp9", codec: "vp09.00.51.08", note: "Profile 0, level 5.1, 8 bit" },
	{ family: "vp9", codec: "vp09.02.31.10", note: "Profile 2, level 3.1, 10 bit" },
	{ family: "av1", codec: "av01.0.05M.08", note: "Main, level 3.1, 8 bit" },
	{ family: "av1", codec: "av01.0.13M.08", note: "Main, level 5.1, 8 bit" },
	{ family: "av1", codec: "av01.0.05M.10", note: "Main, level 3.1, 10 bit" },
];
// The codec string each family uses for the loopback runs.
const LOOPBACK_CODEC: Record<CodecFamily, string> = {
	h264: "avc1.42E01F",
	hevc: "hvc1.1.6.L93.B0",
	vp8: "vp8",
	vp9: "vp09.00.31.08",
	av1: "av01.0.05M.08",
};
const LOOPBACK_HARDWARE: HardwarePreference[] = ["prefer-software", "prefer-hardware"];
const AUDIO_RUNS: AudioRunParams[] = [
	// 960 frames is 20 ms at 48 kHz; 1024 is not a multiple of an Opus frame length.
	{ codec: "opus", sampleRate: 48000, numberOfChannels: 2, bitrate: 96000, framesPerInput: 960, inputCount: 100 },
	{ codec: "opus", sampleRate: 48000, numberOfChannels: 2, bitrate: 96000, framesPerInput: 1024, inputCount: 100 },
	{ codec: "opus", sampleRate: 48000, numberOfChannels: 1, bitrate: 32000, framesPerInput: 960, inputCount: 100 },
];

function isSupported(support: SupportRow[], role: "encoder" | "decoder", codec: string, size: Resolution, hardware: HardwarePreference): boolean {
	return support.some(
		(row) =>
			row.role === role &&
			row.codec === codec &&
			row.width === size.width &&
			row.height === size.height &&
			row.hardwareAcceleration === hardware &&
			row.supported,
	);
}

function planVideoRuns(support: SupportRow[], frameCount: number): VideoRunParams[] {
	const runs: VideoRunParams[] = [];
	const families: CodecFamily[] = ["h264", "hevc", "vp8", "vp9", "av1"];
	for (const family of families) {
		const codec = LOOPBACK_CODEC[family];
		const formats: BitstreamFormat[] = family === "h264" || family === "hevc" ? ["native", "annexb"] : ["native"];
		for (const hardwareAcceleration of LOOPBACK_HARDWARE) {
			const base = (size: Resolution) => ({
				family,
				codec,
				...size,
				framerate: FRAMERATE,
				bitrate: provisionalBitrate(size, FRAMERATE),
				hardwareAcceleration,
				reconfigure: "none",
				// The decoder gets the same preference when it reports support for it, otherwise no preference.
				decoderHardwareAcceleration: isSupported(support, "decoder", codec, size, hardwareAcceleration)
					? hardwareAcceleration
					: "no-preference",
			}) satisfies Partial<VideoRunParams>;

			if (isSupported(support, "encoder", codec, MAIN_RESOLUTION, hardwareAcceleration)) {
				for (const format of formats) {
					runs.push({ ...base(MAIN_RESOLUTION), frameCount, keyFrameInterval: KEY_FRAME_INTERVAL, latencyMode: "realtime", format, scenarios: "all" });
				}
				runs.push({ ...base(MAIN_RESOLUTION), frameCount, keyFrameInterval: KEY_FRAME_INTERVAL, latencyMode: "quality", format: "native", scenarios: "main" });
				for (const latencyMode of ["realtime", "quality"] satisfies VideoRunParams["latencyMode"][]) {
					runs.push({ ...base(MAIN_RESOLUTION), frameCount: 300, keyFrameInterval: null, latencyMode, format: "native", scenarios: "encode-only" });
				}
				for (const reconfigure of ["bitrate", "size"] satisfies VideoRunParams["reconfigure"][]) {
					for (const format of formats) {
						runs.push({ ...base(MAIN_RESOLUTION), frameCount: 60, keyFrameInterval: null, latencyMode: "realtime", format, scenarios: "reconfigure", reconfigure });
					}
				}
			}
			for (const size of EXTRA_RESOLUTIONS) {
				if (!isSupported(support, "encoder", codec, size, hardwareAcceleration)) continue;
				runs.push({ ...base(size), frameCount, keyFrameInterval: KEY_FRAME_INTERVAL, latencyMode: "realtime", format: "native", scenarios: "main" });
			}
		}
	}
	return runs;
}

function withTimeout<T>(promise: Promise<T>, seconds: number): Promise<T> {
	return Promise.race([
		promise,
		new Promise<never>((_, reject) => setTimeout(() => reject(new Error(`no result after ${seconds}s`)), seconds * 1000)),
	]);
}

async function readGpuFeatureStatus(page: Page): Promise<string[] | null> {
	try {
		await page.goto("chrome://gpu");
		await page.waitForTimeout(1500);
		// chrome://gpu renders into the shadow root of <info-view>.
		const text = await page.evaluate(() => document.querySelector("info-view")?.shadowRoot?.textContent ?? null);
		if (!text) return null;
		// The section is a bulleted list; textContent joins the items with "*".
		const start = text.indexOf("Graphics Feature Status");
		const end = text.indexOf("Version Information");
		if (start < 0 || end < 0) return null;
		return text
			.slice(start, end)
			.split("*")
			.slice(1)
			.map((line) => line.trim())
			.filter((line) => line.length > 0);
	} catch (error) {
		console.log(`[driver] chrome://gpu could not be read: ${String(error)}`);
		return null;
	}
}

const build = await Bun.build({ entrypoints: [`${import.meta.dir}/page.ts`], target: "browser" });
if (!build.success) throw new AggregateError(build.logs, "failed to bundle page.ts");
const script = await build.outputs[0].text();

const html = `<!doctype html><meta charset="utf-8"><title>xmoq webcodecs loopback spike</title><script type="module" src="/page.js"></script>`;
// WHY these headers: a cross-origin isolated page gets the finer performance.now() resolution,
// which the per-frame timings rely on. The page reports crossOriginIsolated in its environment.
const isolation = { "cross-origin-opener-policy": "same-origin", "cross-origin-embedder-policy": "require-corp" };
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch(request) {
		const { pathname } = new URL(request.url);
		if (pathname === "/page.js") return new Response(script, { headers: { ...isolation, "content-type": "text/javascript" } });
		return new Response(html, { headers: { ...isolation, "content-type": "text/html" } });
	},
});

const chromeArgs = values["chrome-arg"];
const timeout = Number(values.timeout);
const browser = await chromium.launch({ executablePath, headless: !values.headed, args: chromeArgs });
try {
	console.log(`[driver] browser=${browser.version()} headless=${!values.headed} args=${JSON.stringify(chromeArgs)}`);
	const context = await browser.newContext();
	const gpuFeatureStatus = await readGpuFeatureStatus(await context.newPage());

	const page = await context.newPage();
	page.on("console", (message) => console.log(`[console.${message.type()}] ${message.text()}`));
	page.on("pageerror", (error) => console.log(`[pageerror] ${error.message}`));

	// The DevTools Media domain reports which encoder and decoder implementation a WebCodecs object uses.
	const implementationProperties = ["kVideoEncoderName", "kIsPlatformVideoEncoder", "kVideoDecoderName", "kIsPlatformVideoDecoder"];
	const mediaProperties = new Set<string>();
	const cdp = await context.newCDPSession(page);
	cdp.on("Media.playerPropertiesChanged", (event) => {
		for (const property of event.properties) {
			if (implementationProperties.includes(property.name)) mediaProperties.add(`${property.name}=${property.value}`);
		}
	});
	await cdp.send("Media.enable");

	// WHY 127.0.0.1: http://127.0.0.1 is a secure context, which WebCodecs requires.
	await page.goto(`http://127.0.0.1:${server.port}/`);
	await page.waitForFunction(() => window.__spike !== undefined);

	const environment = await page.evaluate(() => window.__spike?.environment());
	if (!environment) throw new Error("page did not expose the spike API");
	console.log(`[driver] environment ${JSON.stringify(environment)}`);

	const support = await page.evaluate(
		({ candidates, resolutions, framerate }) => window.__spike?.probeSupport(candidates, resolutions, framerate),
		{ candidates: CANDIDATES, resolutions: PROBE_RESOLUTIONS, framerate: FRAMERATE },
	);
	if (!support) throw new Error("support probe returned nothing");
	console.log(`[driver] support rows=${support.length} supported=${support.filter((row) => row.supported).length}`);

	const video: VideoRunEntry[] = [];
	for (const params of planVideoRuns(support, Number(values.frames))) {
		const name = `${params.codec} ${params.width}x${params.height} ${params.hardwareAcceleration} ${params.format} ${params.latencyMode} ${params.scenarios} reconfigure=${params.reconfigure} frames=${params.frameCount}`;
		// WHY the wait: Media events of the previous run arrive after its result. Without it they
		// were recorded under the next run.
		await page.waitForTimeout(300);
		mediaProperties.clear();
		try {
			const result = await withTimeout(page.evaluate((p) => window.__spike?.runVideo(p), params), timeout);
			await page.waitForTimeout(300);
			video.push({ params, result: result ?? null, failure: result ? null : "no result", mediaProperties: [...mediaProperties] });
			const full = result?.decodes.find((decode) => decode.scenario === "full");
			console.log(`[driver] ${name}: chunks=${result?.encode.chunks.length} encodeErrors=${result?.encode.errors.length} full=${JSON.stringify(full?.check ?? null)}`);
		} catch (error) {
			video.push({ params, result: null, failure: String(error), mediaProperties: [...mediaProperties] });
			console.log(`[driver] ${name}: FAILED ${String(error)}`);
			// A run that hangs leaves the page in an unknown state; start from a fresh one.
			await page.reload();
			await page.waitForFunction(() => window.__spike !== undefined);
		}
	}

	const audio: AudioRunResult[] = [];
	if (!values["skip-audio"]) {
		for (const params of AUDIO_RUNS) {
			const result = await withTimeout(page.evaluate((p) => window.__spike?.runAudio(p), params), timeout);
			if (result) audio.push(result);
			console.log(`[driver] audio ${JSON.stringify(params)}: chunks=${result?.chunks.length} errors=${result?.encodeErrors.length}`);
		}
	}

	const file: RunFile = {
		label,
		finishedAt: new Date().toISOString(),
		browserVersion: browser.version(),
		executablePath,
		headless: !values.headed,
		chromeArgs,
		environment,
		gpuFeatureStatus,
		candidates: CANDIDATES,
		support,
		video,
		audio,
	};
	await mkdir(`${import.meta.dir}/out`, { recursive: true });
	const path = `${import.meta.dir}/out/${label}.json`;
	await Bun.write(path, JSON.stringify(file, null, "\t"));
	console.log(`[driver] wrote ${path}`);
} finally {
	await browser.close();
	server.stop(true);
}
