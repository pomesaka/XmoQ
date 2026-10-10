// Spike, outside lint and review: encode a generated picture with WebCodecs in one
// browser, decode it in the same one and in the other. No network and no MoQ.
// Every codec string, size, bitrate and frame count is a provisional value for this check,
// the ones of spikes/webcodecs-loopback (1280x720, 30 fps, 90 frames, key frames at 0, 30, 60).
// Run from the repository root: bun spikes/browser-support/codecs.ts [--frames 90]
import { parseArgs } from "node:util";
import { chromium, firefox, type Page } from "../../interop/node_modules/playwright-core/index.mjs";

const { values } = parseArgs({ options: { frames: { type: "string", default: "90" } } });
const FRAMES = Number(values.frames);

interface Config { label: string; codec: string; avcFormat?: "avc" | "annexb" }
const CONFIGS: Config[] = [
	{ label: "H.264 avc", codec: "avc1.42E01F", avcFormat: "avc" },
	{ label: "H.264 annexb", codec: "avc1.42E01F", avcFormat: "annexb" },
	{ label: "VP8", codec: "vp8" },
	{ label: "VP9", codec: "vp09.00.31.08" },
	{ label: "AV1", codec: "av01.0.05M.08" },
];

interface Encoded {
	error?: string;
	chunks: { type: string; timestamp: number; base64: string }[];
	/** Index of the output each decoderConfig came with, and what it held. */
	decoderConfigs: { at: number; codec: string; descriptionHex: string | null; codedWidth?: number; codedHeight?: number }[];
	/** Milliseconds from encode() to the output, one frame at a time; null where no output came within 2 s. */
	encodeMs: (number | null)[];
	keyAt: number[];
}
interface Decoded { error?: string; frames: number; sizes: string[]; decodeMs: (number | null)[] }

/** Runs in the page. */
async function encodeInPage([config, frames]: [Config, number]): Promise<Encoded> {
	const g = globalThis as any;
	const result: Encoded = { chunks: [], decoderConfigs: [], encodeMs: [], keyAt: [] };
	const toBase64 = (bytes: Uint8Array) => { let text = ""; for (const b of bytes) text += String.fromCharCode(b); return btoa(text); };
	const toHex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
	let waiting: (() => void) | undefined;
	let failure: string | undefined;
	const encoder = new g.VideoEncoder({
		output(chunk: any, metadata: any) {
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);
			if (metadata?.decoderConfig) {
				const d = metadata.decoderConfig;
				const description = d.description === undefined ? null : toHex(new Uint8Array(d.description instanceof ArrayBuffer ? d.description : d.description.buffer.slice(d.description.byteOffset, d.description.byteOffset + d.description.byteLength)));
				result.decoderConfigs.push({ at: result.chunks.length, codec: d.codec, descriptionHex: description, codedWidth: d.codedWidth, codedHeight: d.codedHeight });
			}
			if (chunk.type === "key") result.keyAt.push(result.chunks.length);
			result.chunks.push({ type: chunk.type, timestamp: chunk.timestamp, base64: toBase64(data) });
			waiting?.();
		},
		error(error: Error) { failure = `${error.name}: ${error.message}`; waiting?.(); },
	});
	try {
		encoder.configure({
			codec: config.codec, width: 1280, height: 720, bitrate: 2_764_800, framerate: 30, latencyMode: "realtime",
			...(config.avcFormat ? { avc: { format: config.avcFormat } } : {}),
		});
		const canvas = new g.OffscreenCanvas(1280, 720);
		const context = canvas.getContext("2d");
		for (let index = 0; index < frames; index += 1) {
			const gradient = context.createLinearGradient(0, 0, 1280, 720);
			gradient.addColorStop(0, `hsl(${index * 4}, 80%, 50%)`);
			gradient.addColorStop(1, `hsl(${index * 4 + 120}, 80%, 30%)`);
			context.fillStyle = gradient;
			context.fillRect(0, 0, 1280, 720);
			context.fillStyle = "white";
			context.fillRect((index * 13) % 1200, 0, 40, 720);
			context.font = "200px sans-serif";
			context.fillText(String(index), 480, 420);
			const frame = new g.VideoFrame(canvas, { timestamp: Math.round((index * 1_000_000) / 30) });
			const before = result.chunks.length;
			const output = new Promise<void>((resolve) => { waiting = resolve; });
			const start = performance.now();
			encoder.encode(frame, { keyFrame: index % 30 === 0 });
			frame.close();
			await Promise.race([output, new Promise((resolve) => setTimeout(resolve, 2000))]);
			result.encodeMs.push(result.chunks.length > before ? performance.now() - start : null);
			if (failure) break;
		}
		if (!failure) await encoder.flush();
	} catch (error) {
		failure = `${(error as Error).name}: ${(error as Error).message}`;
	}
	if (encoder.state !== "closed") encoder.close();
	if (failure) result.error = failure;
	return result;
}

/** Runs in the page. `description` null means: configure with the codec string alone. */
async function decodeInPage([codec, descriptionHex, chunks]: [string, string | null, Encoded["chunks"]]): Promise<Decoded> {
	const g = globalThis as any;
	const result: Decoded = { frames: 0, sizes: [], decodeMs: [] };
	const fromBase64 = (text: string) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
	const fromHex = (text: string) => new Uint8Array((text.match(/../g) ?? []).map((h) => Number.parseInt(h, 16)));
	let waiting: (() => void) | undefined;
	let failure: string | undefined;
	const decoder = new g.VideoDecoder({
		output(frame: any) {
			result.frames += 1;
			const size = `${frame.displayWidth}x${frame.displayHeight}`;
			if (!result.sizes.includes(size)) result.sizes.push(size);
			frame.close();
			waiting?.();
		},
		error(error: Error) { failure = `${error.name}: ${error.message}`; waiting?.(); },
	});
	try {
		decoder.configure({ codec, optimizeForLatency: true, ...(descriptionHex === null ? {} : { description: fromHex(descriptionHex) }) });
		for (const chunk of chunks) {
			const before = result.frames;
			const output = new Promise<void>((resolve) => { waiting = resolve; });
			const start = performance.now();
			decoder.decode(new g.EncodedVideoChunk({ type: chunk.type, timestamp: chunk.timestamp, data: fromBase64(chunk.base64) }));
			await Promise.race([output, new Promise((resolve) => setTimeout(resolve, 1000))]);
			result.decodeMs.push(result.frames > before ? performance.now() - start : null);
			if (failure) break;
		}
		if (!failure) await decoder.flush();
	} catch (error) {
		failure = `${(error as Error).name}: ${(error as Error).message}`;
	}
	if (decoder.state !== "closed") decoder.close();
	if (failure) result.error = failure;
	return result;
}

const stats = (values: (number | null)[]): string => {
	const numbers = values.filter((v): v is number => v !== null).sort((a, b) => a - b);
	if (numbers.length === 0) return "no output in time";
	const at = (q: number) => numbers[Math.min(numbers.length - 1, Math.floor(q * numbers.length))]!.toFixed(2);
	const held = values.length - numbers.length;
	return `${at(0)} / ${at(0.5)} / ${at(0.95)} / ${numbers.at(-1)!.toFixed(2)}${held > 0 ? ` (${held} without output in time)` : ""}`;
};
const bytesOf = (chunks: Encoded["chunks"], type: string): string => {
	const sizes = chunks.filter((c) => c.type === type).map((c) => Math.floor((c.base64.length * 3) / 4)).sort((a, b) => a - b);
	return sizes.length === 0 ? "none" : `${sizes[0]} / ${sizes[Math.floor(sizes.length / 2)]} / ${sizes.at(-1)} (${sizes.length})`;
};

const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response("<!DOCTYPE html><title>codecs</title>", { headers: { "content-type": "text/html" } }) });
const browsers = { chromium: await chromium.launch({ channel: "chromium" }), firefox: await firefox.launch() };
const pages: Record<string, Page> = {};
try {
	for (const [name, browser] of Object.entries(browsers)) {
		const page = await browser.newPage();
		page.setDefaultTimeout(300_000);
		await page.goto(`http://localhost:${server.port}/`);
		pages[name] = page;
		console.log(`${name} ${browser.version()}`);
	}
	const encoded: Record<string, Record<string, Encoded>> = { chromium: {}, firefox: {} };
	for (const name of Object.keys(pages)) {
		for (const config of CONFIGS) {
			const out = await pages[name]!.evaluate(encodeInPage, [config, FRAMES] as [Config, number]);
			encoded[name]![config.label] = out;
			const first = out.decoderConfigs[0];
			console.log(`ENCODE ${name} | ${config.label} | ${out.error ?? "ok"} | chunks ${out.chunks.length} | key at ${out.keyAt.join(",")} | key bytes ${bytesOf(out.chunks, "key")} | delta bytes ${bytesOf(out.chunks, "delta")} | encode ms ${stats(out.encodeMs)} | decoderConfig at ${out.decoderConfigs.map((d) => d.at).join(",") || "never"} | codec ${first?.codec} | description ${first?.descriptionHex === null || first === undefined ? "none" : `${first.descriptionHex.length / 2} bytes ${first.descriptionHex.slice(0, 64)}`} | coded ${first?.codedWidth}x${first?.codedHeight}`);
		}
	}
	for (const from of Object.keys(pages)) {
		for (const to of Object.keys(pages)) {
			for (const config of CONFIGS) {
				const source = encoded[from]![config.label]!;
				if (source.chunks.length === 0) { console.log(`DECODE ${from} -> ${to} | ${config.label} | nothing to decode`); continue; }
				const first = source.decoderConfigs[0];
				const out = await pages[to]!.evaluate(decodeInPage, [first?.codec ?? config.codec, first?.descriptionHex ?? null, source.chunks] as [string, string | null, Encoded["chunks"]]);
				console.log(`DECODE ${from} -> ${to} | ${config.label} | ${out.error ?? "ok"} | frames ${out.frames} of ${source.chunks.length} | ${out.sizes.join(",")} | decode ms ${stats(out.decodeMs)}`);
			}
		}
	}
} finally {
	for (const browser of Object.values(browsers)) await browser.close();
	server.stop(true);
}
