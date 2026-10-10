// Runs inside the browser. Exposes the experiments as window.__spike; run.ts calls them.
// No network and no MoQ: encoded chunks are copied to byte arrays and handed straight to a decoder.
import type {
	AudioChunkRecord,
	AudioDecodeResult,
	AudioDecodedRecord,
	AudioRunParams,
	AudioRunResult,
	ChunkRecord,
	CodecCandidate,
	CodecFamily,
	DecodeCheck,
	DecodeResult,
	DecoderConfigRecord,
	EncodeResult,
	EnvironmentInfo,
	ErrorRecord,
	FrameRecord,
	HardwarePreference,
	Resolution,
	SpikeApi,
	SupportRow,
	VideoRunParams,
	VideoRunResult,
} from "./types";
import { provisionalBitrate } from "./types";

declare global {
	interface Window {
		__spike?: SpikeApi;
	}
}

const HARDWARE_PREFERENCES: HardwarePreference[] = ["no-preference", "prefer-hardware", "prefer-software"];

// How long serial mode waits for the output of the frame it just submitted.
const OUTPUT_WAIT_MS = 1000;
// After this many consecutive waits without output, serial mode stops waiting:
// the codec is holding frames back and every further wait would only add OUTPUT_WAIT_MS.
const MAX_CONSECUTIVE_LATE = 3;

const MARKER_BITS = 16;

// Decoded frames are scaled down to this size and kept, so that the frames of one decode scenario
// can be compared with the frames of the plain full decode.
const THUMBNAIL: Resolution = { width: 64, height: 36 };
type Thumbnails = Map<number, Uint8ClampedArray>;

// Mean absolute difference per colour channel, 0..255.
function thumbnailDifference(a: Uint8ClampedArray, b: Uint8ClampedArray): number {
	let sum = 0;
	for (let i = 0; i < a.length; i += 4) sum += Math.abs(a[i] - b[i]) + Math.abs(a[i + 1] - b[i + 1]) + Math.abs(a[i + 2] - b[i + 2]);
	return sum / ((a.length / 4) * 3);
}

// What the spike treats as "the bytes on the wire" for one chunk.
interface WireChunk {
	seq: number;
	type: EncodedVideoChunkType;
	timestamp: number;
	duration: number | null;
	data: Uint8Array;
	// The decoderConfig the encoder emitted together with this chunk, if any.
	config: WireConfig | null;
}

// Decoder configuration rebuilt from primitives and copied bytes, never the object the encoder handed out.
interface WireConfig {
	codec: string;
	codedWidth?: number;
	codedHeight?: number;
	colorSpace?: VideoColorSpaceInit;
	description?: Uint8Array;
}

interface InputInfo {
	index: number;
	timestamp: number;
	duration: number;
	keyRequested: boolean;
}

class OutputCounter {
	count = 0;
	#wake: (() => void) | null = null;

	bump(): void {
		this.count += 1;
		this.wake();
	}

	wake(): void {
		this.#wake?.();
	}

	// Resolves true once `count` reaches `target` and `stop()` is false, or false after the timeout.
	async waitFor(target: number, stop: () => boolean): Promise<boolean> {
		const deadline = performance.now() + OUTPUT_WAIT_MS;
		while (this.count < target && !stop()) {
			const remaining = deadline - performance.now();
			if (remaining <= 0) return false;
			await new Promise<void>((resolve) => {
				const timer = setTimeout(resolve, remaining);
				this.#wake = () => {
					clearTimeout(timer);
					resolve();
				};
			});
			this.#wake = null;
		}
		return this.count >= target;
	}
}

// Serial pacing: after each submission, wait for its output before submitting the next one.
class SerialPacer {
	late = 0;
	// Submission index at which waiting was given up, or null.
	abandonedAt: number | null = null;
	#consecutive = 0;
	readonly #enabled: boolean;
	readonly #counter: OutputCounter;

	constructor(enabled: boolean, counter: OutputCounter) {
		this.#enabled = enabled;
		this.#counter = counter;
	}

	// While true, the time from a submission to its output is a per-frame time.
	get waiting(): boolean {
		return this.#enabled && this.abandonedAt === null;
	}

	async after(index: number, target: number, stop: () => boolean): Promise<void> {
		if (!this.waiting) return;
		if (await this.#counter.waitFor(target, stop)) {
			this.#consecutive = 0;
			return;
		}
		this.late += 1;
		this.#consecutive += 1;
		if (this.#consecutive >= MAX_CONSECUTIVE_LATE) this.abandonedAt = index;
	}
}

function toErrorRecord(where: string, error: unknown, at: number | null): ErrorRecord {
	if (error instanceof Error) return { where, name: error.name, message: error.message, at };
	return { where, name: "unknown", message: String(error), at };
}

function toBytes(source: AllowSharedBufferSource): Uint8Array {
	if (ArrayBuffer.isView(source)) return new Uint8Array(source.buffer, source.byteOffset, source.byteLength).slice();
	return new Uint8Array(source).slice();
}

function toHex(bytes: Uint8Array): string {
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function toJsonRecord(value: unknown): Record<string, unknown> | null {
	if (typeof value !== "object" || value === null) return null;
	const parsed: unknown = JSON.parse(JSON.stringify(value));
	if (typeof parsed !== "object" || parsed === null) return null;
	return Object.fromEntries(Object.entries(parsed));
}

// ---- picture source -------------------------------------------------------------------------

// A deterministic picture: moving gradient, a moving bar, the frame number as text, and the same
// number as a row of black/white cells along the top edge. The cells survive lossy coding, so the
// decoded picture can be matched to the frame that was encoded.
function drawFrame(context: OffscreenCanvasRenderingContext2D, width: number, height: number, index: number): void {
	const hue = (index * 7) % 360;
	const gradient = context.createLinearGradient(0, 0, width, height);
	gradient.addColorStop(0, `hsl(${hue} 70% 50%)`);
	gradient.addColorStop(1, `hsl(${(hue + 120) % 360} 70% 30%)`);
	context.fillStyle = gradient;
	context.fillRect(0, 0, width, height);

	context.fillStyle = "#ffffff";
	context.fillRect((index * 8) % width, Math.floor(height * 0.75), Math.floor(width / 20), Math.floor(height / 8));

	context.font = `bold ${Math.floor(height / 3)}px monospace`;
	context.textAlign = "center";
	context.textBaseline = "middle";
	context.fillText(String(index), width / 2, height / 2);

	const cellWidth = width / MARKER_BITS;
	const cellHeight = Math.floor(height / 8);
	for (let bit = 0; bit < MARKER_BITS; bit += 1) {
		context.fillStyle = (index >> bit) & 1 ? "#ffffff" : "#000000";
		context.fillRect(Math.floor(bit * cellWidth), 0, Math.ceil(cellWidth), cellHeight);
	}
}

function readMarker(context: OffscreenCanvasRenderingContext2D, frame: VideoFrame, width: number, height: number): number {
	context.drawImage(frame, 0, 0, width, height);
	const cellWidth = width / MARKER_BITS;
	const row = Math.floor(height / 16);
	const pixels = context.getImageData(0, row, width, 1).data;
	let value = 0;
	for (let bit = 0; bit < MARKER_BITS; bit += 1) {
		const x = Math.floor((bit + 0.5) * cellWidth);
		const luma = (pixels[x * 4] + pixels[x * 4 + 1] + pixels[x * 4 + 2]) / 3;
		if (luma > 127) value |= 1 << bit;
	}
	return value;
}

function timestampOf(index: number, framerate: number): number {
	return Math.round((index * 1_000_000) / framerate);
}

// ---- bitstream inspection -------------------------------------------------------------------

function annexBUnitHeaders(data: Uint8Array): number[] {
	const headers: number[] = [];
	for (let i = 0; i + 3 < data.length; i += 1) {
		if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) {
			headers.push(data[i + 3]);
			i += 2;
		}
	}
	return headers;
}

function lengthPrefixedUnitHeaders(data: Uint8Array, lengthSize: number): number[] {
	const headers: number[] = [];
	let position = 0;
	while (position + lengthSize < data.length) {
		let length = 0;
		for (let i = 0; i < lengthSize; i += 1) length = length * 256 + data[position + i];
		headers.push(data[position + lengthSize]);
		position += lengthSize + length;
	}
	return headers;
}

function av1ObuTypes(data: Uint8Array): number[] {
	const types: number[] = [];
	let position = 0;
	while (position < data.length) {
		const header = data[position];
		types.push((header >> 3) & 0x0f);
		const hasExtension = (header >> 2) & 1;
		const hasSize = (header >> 1) & 1;
		position += 1 + hasExtension;
		if (!hasSize) break;
		let size = 0;
		for (let shift = 0; position < data.length; shift += 7) {
			const byte = data[position];
			position += 1;
			size += (byte & 0x7f) * 2 ** shift;
			if ((byte & 0x80) === 0) break;
		}
		position += size;
	}
	return types;
}

function inspectChunk(
	family: CodecFamily,
	data: Uint8Array,
	description: Uint8Array | null,
): { units: number[] | null; bitstreamKey: boolean | null } {
	switch (family) {
		case "h264": {
			// avcC: lengthSizeMinusOne is the low 2 bits of byte 4 (ISO/IEC 14496-15, AVCDecoderConfigurationRecord).
			const headers = description ? lengthPrefixedUnitHeaders(data, (description[4] & 3) + 1) : annexBUnitHeaders(data);
			const units = headers.map((header) => header & 0x1f);
			return { units, bitstreamKey: units.includes(5) };
		}
		case "hevc": {
			// hvcC: lengthSizeMinusOne is the low 2 bits of byte 21 (ISO/IEC 14496-15, HEVCDecoderConfigurationRecord).
			const headers = description ? lengthPrefixedUnitHeaders(data, (description[21] & 3) + 1) : annexBUnitHeaders(data);
			const units = headers.map((header) => (header >> 1) & 0x3f);
			// NAL unit types 16..23 are IRAP pictures.
			return { units, bitstreamKey: units.some((unit) => unit >= 16 && unit <= 23) };
		}
		case "av1":
			return { units: av1ObuTypes(data), bitstreamKey: null };
		case "vp8":
			// RFC 6386 section 9.1: bit 0 of the first byte is 0 for a key frame.
			return { units: null, bitstreamKey: (data[0] & 1) === 0 };
		case "vp9": {
			// VP9 uncompressed header: frame_marker(2) profile_low(1) profile_high(1) [reserved(1) if profile 3]
			// show_existing_frame(1) frame_type(1). frame_type 0 is a key frame.
			const profile = (((data[0] >> 4) & 1) << 1) | ((data[0] >> 5) & 1);
			const shift = profile === 3 ? 1 : 2;
			const showExisting = (data[0] >> (shift + 1)) & 1;
			return { units: null, bitstreamKey: showExisting === 0 && ((data[0] >> shift) & 1) === 0 };
		}
	}
}

// ---- support probe --------------------------------------------------------------------------

async function probeSupport(candidates: CodecCandidate[], resolutions: Resolution[], framerate: number): Promise<SupportRow[]> {
	const rows: SupportRow[] = [];
	for (const candidate of candidates) {
		for (const resolution of resolutions) {
			for (const hardwareAcceleration of HARDWARE_PREFERENCES) {
				const base = { family: candidate.family, codec: candidate.codec, ...resolution, hardwareAcceleration };
				try {
					const support = await VideoEncoder.isConfigSupported({
						codec: candidate.codec,
						width: resolution.width,
						height: resolution.height,
						framerate,
						bitrate: provisionalBitrate(resolution, framerate),
						hardwareAcceleration,
					});
					rows.push({ role: "encoder", ...base, supported: support.supported === true, error: null });
				} catch (error) {
					rows.push({ role: "encoder", ...base, supported: false, error: String(error) });
				}
				try {
					const support = await VideoDecoder.isConfigSupported({
						codec: candidate.codec,
						codedWidth: resolution.width,
						codedHeight: resolution.height,
						hardwareAcceleration,
					});
					rows.push({ role: "decoder", ...base, supported: support.supported === true, error: null });
				} catch (error) {
					rows.push({ role: "decoder", ...base, supported: false, error: String(error) });
				}
			}
		}
	}
	return rows;
}

// ---- encode ---------------------------------------------------------------------------------

function recordDecoderConfig(config: VideoDecoderConfig): { record: DecoderConfigRecord; wire: WireConfig } {
	const description = config.description ? toBytes(config.description) : null;
	const wire: WireConfig = { codec: config.codec };
	if (config.codedWidth !== undefined) wire.codedWidth = config.codedWidth;
	if (config.codedHeight !== undefined) wire.codedHeight = config.codedHeight;
	if (config.colorSpace) wire.colorSpace = { ...config.colorSpace };
	if (description) wire.description = description;
	return {
		wire,
		record: {
			keys: Object.keys(config),
			values: toJsonRecord({ ...config, description: undefined }) ?? {},
			codec: config.codec,
			codedWidth: config.codedWidth ?? null,
			codedHeight: config.codedHeight ?? null,
			displayAspectWidth: config.displayAspectWidth ?? null,
			displayAspectHeight: config.displayAspectHeight ?? null,
			colorSpace: toJsonRecord(config.colorSpace),
			descriptionHex: description ? toHex(description) : null,
		},
	};
}

// WHY Reflect: EncodedVideoChunkMetadata.svc is not in TypeScript 6.0.3's DOM library.
function temporalLayerIdOf(metadata: EncodedVideoChunkMetadata | undefined): number | null {
	const svc: unknown = metadata ? Reflect.get(metadata, "svc") : undefined;
	if (typeof svc !== "object" || svc === null) return null;
	const id: unknown = Reflect.get(svc, "temporalLayerId");
	return typeof id === "number" ? id : null;
}

interface EncodeOutcome {
	result: EncodeResult;
	wire: WireChunk[];
	// The first decoderConfig the encoder emitted.
	wireConfig: WireConfig | null;
}

async function encodeVideo(params: VideoRunParams, pacing: "serial" | "burst"): Promise<EncodeOutcome> {
	let size: Resolution = { width: params.width, height: params.height };
	let canvas = new OffscreenCanvas(size.width, size.height);
	let context = canvas.getContext("2d");
	if (!context) throw new Error("2d context unavailable");

	const inputs: InputInfo[] = [];
	const chunks: ChunkRecord[] = [];
	const wire: WireChunk[] = [];
	const errors: ErrorRecord[] = [];
	const submittedAt = new Map<number, number>();
	const counter = new OutputCounter();
	// Every decoderConfig the encoder emitted, in order.
	const emittedConfigs: WireConfig[] = [];
	const pacer = new SerialPacer(pacing === "serial", counter);
	let current: number | null = null;

	const encoder = new VideoEncoder({
		output(chunk, metadata) {
			const now = performance.now();
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);

			const recorded = metadata?.decoderConfig ? recordDecoderConfig(metadata.decoderConfig) : null;
			if (recorded) emittedConfigs.push(recorded.wire);
			const submitted = submittedAt.get(chunk.timestamp);
			const seq = chunks.length;
			wire.push({ seq, type: chunk.type, timestamp: chunk.timestamp, duration: chunk.duration, data, config: recorded?.wire ?? null });
			chunks.push({
				seq,
				type: chunk.type,
				timestamp: chunk.timestamp,
				duration: chunk.duration,
				byteLength: chunk.byteLength,
				encodeLatencyMs: submitted !== undefined ? now - submitted : null,
				metadataKeys: metadata ? Object.keys(metadata) : [],
				decoderConfig: recorded?.record ?? null,
				temporalLayerId: temporalLayerIdOf(metadata),
				...inspectChunk(params.family, data, emittedConfigs.at(-1)?.description ?? null),
			});
			counter.bump();
		},
		error(error) {
			errors.push(toErrorRecord("error callback", error, current));
			counter.wake();
		},
	});

	// WHY an intersection type: TypeScript 6.0.3's DOM library has `avc` but no `hevc` member on
	// VideoEncoderConfig. The member is defined by the HEVC codec registration.
	const config: VideoEncoderConfig & { hevc?: { format: "hevc" | "annexb" } } = {
		codec: params.codec,
		width: params.width,
		height: params.height,
		framerate: params.framerate,
		bitrate: params.bitrate,
		hardwareAcceleration: params.hardwareAcceleration,
		latencyMode: params.latencyMode,
	};
	if (params.family === "h264") config.avc = { format: params.format === "annexb" ? "annexb" : "avc" };
	if (params.family === "hevc") config.hevc = { format: params.format === "annexb" ? "annexb" : "hevc" };

	const started = performance.now();
	try {
		encoder.configure(config);
	} catch (error) {
		errors.push(toErrorRecord("configure() threw", error, null));
	}

	const reconfigureAt = params.reconfigure === "none" ? null : Math.floor(params.frameCount / 2);
	let reconfigure: EncodeResult["reconfigure"] = null;
	for (let index = 0; index < params.frameCount && encoder.state === "configured"; index += 1) {
		current = index;
		if (index === reconfigureAt) {
			const next = { ...config };
			if (params.reconfigure === "bitrate") next.bitrate = Math.round(params.bitrate / 2);
			if (params.reconfigure === "size") {
				size = { width: params.width / 2, height: params.height / 2 };
				next.width = size.width;
				next.height = size.height;
				canvas = new OffscreenCanvas(size.width, size.height);
				context = canvas.getContext("2d");
				if (!context) throw new Error("2d context unavailable");
			}
			try {
				encoder.configure(next);
			} catch (error) {
				errors.push(toErrorRecord("configure() threw", error, index));
			}
			reconfigure = { at: index, config: toJsonRecord(next) ?? {} };
		}
		const timestamp = timestampOf(index, params.framerate);
		const duration = timestampOf(index + 1, params.framerate) - timestamp;
		const keyRequested = params.keyFrameInterval !== null && index % params.keyFrameInterval === 0;
		inputs.push({ index, timestamp, duration, keyRequested });

		drawFrame(context, size.width, size.height, index);
		const frame = new VideoFrame(canvas, { timestamp, duration });
		if (pacer.waiting) submittedAt.set(timestamp, performance.now());
		try {
			encoder.encode(frame, { keyFrame: keyRequested });
		} catch (error) {
			errors.push(toErrorRecord("encode() threw", error, index));
		}
		frame.close();

		await pacer.after(index, index + 1, () => encoder.state !== "configured");
	}

	if (encoder.state === "configured") {
		try {
			await encoder.flush();
		} catch (error) {
			errors.push(toErrorRecord("flush() rejected", error, null));
		}
	}
	const totalMs = performance.now() - started;
	const finalState = encoder.state;
	if (encoder.state !== "closed") encoder.close();

	return {
		wire,
		wireConfig: emittedConfigs.at(0) ?? null,
		result: {
			pacing,
			configure: toJsonRecord(config) ?? {},
			reconfigure,
			inputs,
			chunks,
			errors,
			finalState,
			lateOutputs: pacer.late,
			serialAbandonedAt: pacer.abandonedAt,
			totalMs,
		},
	};
}

// ---- decode ---------------------------------------------------------------------------------

interface DecodeScenario {
	name: string;
	description: string;
	pacing: "serial" | "burst";
	config: WireConfig;
	chunks: WireChunk[];
	hardwareAcceleration: HardwarePreference;
	optimizeForLatency?: boolean;
	omitDuration?: boolean;
	// Call flush() after this many chunks, then go on decoding the rest.
	flushAfter?: number;
	// Call configure() again before each later chunk that the encoder emitted a decoderConfig with.
	followConfigs?: boolean;
}

function toConfigRecord(config: WireConfig): DecoderConfigRecord {
	return {
		keys: Object.keys(config),
		values: toJsonRecord({ ...config, description: undefined }) ?? {},
		codec: config.codec,
		codedWidth: config.codedWidth ?? null,
		codedHeight: config.codedHeight ?? null,
		displayAspectWidth: null,
		displayAspectHeight: null,
		colorSpace: toJsonRecord(config.colorSpace),
		descriptionHex: config.description ? toHex(config.description) : null,
	};
}

// `accepted` are the chunks decode() took without throwing; the frames are checked against those.
function checkFrames(accepted: WireChunk[], frames: FrameRecord[], inputs: Map<number, InputInfo>): DecodeCheck {
	const expected = accepted.map((chunk) => inputs.get(chunk.timestamp));
	let increasing = true;
	let timestampsMatch = frames.length === expected.length;
	let durationsMatch = frames.length === expected.length;
	let markersMatch = frames.length === expected.length;
	const wrongPictureAt: number[] = [];
	const differsFromFullAt: number[] = [];
	let maxDifferenceFromFull: number | null = null;
	let firstMismatch: string | null = null;
	const note = (text: string) => {
		firstMismatch ??= text;
	};

	for (let i = 0; i < frames.length; i += 1) {
		const frame = frames[i];
		if (i > 0 && frame.timestamp <= frames[i - 1].timestamp) {
			increasing = false;
			note(`output ${i}: timestamp ${frame.timestamp} after ${frames[i - 1].timestamp}`);
		}
		if (frame.differenceFromFull !== null) {
			maxDifferenceFromFull = Math.max(maxDifferenceFromFull ?? 0, frame.differenceFromFull);
			if (frame.differenceFromFull > 0) differsFromFullAt.push(i);
		}
		const input = expected[i];
		if (!input) continue;
		if (frame.timestamp !== input.timestamp) {
			timestampsMatch = false;
			note(`output ${i}: timestamp ${frame.timestamp}, expected ${input.timestamp}`);
		}
		if (frame.duration !== input.duration) {
			durationsMatch = false;
			note(`output ${i}: duration ${frame.duration}, expected ${input.duration}`);
		}
		if (frame.marker !== null && frame.marker !== input.index) {
			markersMatch = false;
			wrongPictureAt.push(i);
			note(`output ${i}: picture shows frame ${frame.marker}, expected ${input.index}`);
		}
	}
	if (frames.length !== expected.length) note(`decoded ${frames.length} frames, expected ${expected.length}`);
	return {
		expected: expected.length,
		decoded: frames.length,
		countMatch: frames.length === expected.length,
		increasing,
		timestampsMatch,
		durationsMatch,
		markersMatch,
		wrongPictureAt,
		differsFromFullAt,
		maxDifferenceFromFull,
		firstMismatch,
	};
}

// `reference`: thumbnails of the plain full decode, keyed by timestamp. null while producing them.
async function decodeVideo(
	scenario: DecodeScenario,
	size: Resolution,
	inputs: Map<number, InputInfo>,
	reference: Thumbnails | null,
): Promise<{ result: DecodeResult; thumbnails: Thumbnails }> {
	const canvas = new OffscreenCanvas(size.width, size.height);
	const context = canvas.getContext("2d", { willReadFrequently: true });
	const small = new OffscreenCanvas(THUMBNAIL.width, THUMBNAIL.height).getContext("2d", { willReadFrequently: true });
	if (!context || !small) throw new Error("2d context unavailable");
	const thumbnails: Thumbnails = new Map();
	const accepted: WireChunk[] = [];

	const frames: FrameRecord[] = [];
	const errors: ErrorRecord[] = [];
	const submittedAt = new Map<number, number>();
	const counter = new OutputCounter();
	const pacer = new SerialPacer(scenario.pacing === "serial", counter);
	const reconfiguredAt: number[] = [];
	let current: number | null = null;

	const decoder = new VideoDecoder({
		output(frame) {
			const now = performance.now();
			const submitted = submittedAt.get(frame.timestamp);
			// WHY not in burst mode: reading pixels back runs on the same thread as the output callback
			// and would be counted in the throughput.
			let marker: number | null = null;
			let differenceFromFull: number | null = null;
			if (scenario.pacing === "serial") {
				marker = readMarker(context, frame, size.width, size.height);
				small.drawImage(frame, 0, 0, THUMBNAIL.width, THUMBNAIL.height);
				const thumbnail = small.getImageData(0, 0, THUMBNAIL.width, THUMBNAIL.height).data;
				thumbnails.set(frame.timestamp, thumbnail);
				const expected = reference?.get(frame.timestamp);
				if (expected) differenceFromFull = thumbnailDifference(thumbnail, expected);
			}
			frames.push({
				timestamp: frame.timestamp,
				duration: frame.duration,
				codedWidth: frame.codedWidth,
				codedHeight: frame.codedHeight,
				displayWidth: frame.displayWidth,
				displayHeight: frame.displayHeight,
				format: frame.format,
				marker,
				differenceFromFull,
				decodeLatencyMs: submitted !== undefined ? now - submitted : null,
			});
			frame.close();
			counter.bump();
		},
		error(error) {
			errors.push(toErrorRecord("error callback", error, current));
			counter.wake();
		},
	});

	const config: VideoDecoderConfig = { ...scenario.config, hardwareAcceleration: scenario.hardwareAcceleration };
	if (scenario.optimizeForLatency !== undefined) config.optimizeForLatency = scenario.optimizeForLatency;

	const started = performance.now();
	try {
		decoder.configure(config);
	} catch (error) {
		errors.push(toErrorRecord("configure() threw", error, null));
	}

	let submitted = 0;
	for (let i = 0; i < scenario.chunks.length && decoder.state === "configured"; i += 1) {
		current = i;
		const wire = scenario.chunks[i];
		if (scenario.followConfigs && i > 0 && wire.config) {
			try {
				decoder.configure({ ...wire.config, hardwareAcceleration: scenario.hardwareAcceleration });
				reconfiguredAt.push(i);
			} catch (error) {
				errors.push(toErrorRecord("configure() threw", error, i));
			}
		}
		const init: EncodedVideoChunkInit = { type: wire.type, timestamp: wire.timestamp, data: wire.data };
		if (wire.duration !== null && !scenario.omitDuration) init.duration = wire.duration;
		if (pacer.waiting) submittedAt.set(wire.timestamp, performance.now());
		try {
			decoder.decode(new EncodedVideoChunk(init));
			submitted += 1;
			accepted.push(wire);
		} catch (error) {
			errors.push(toErrorRecord("decode() threw", error, i));
			continue;
		}

		await pacer.after(i, submitted, () => decoder.state !== "configured");

		if (scenario.flushAfter === i + 1 && decoder.state === "configured") {
			try {
				await decoder.flush();
			} catch (error) {
				errors.push(toErrorRecord("flush() rejected", error, i));
			}
		}
	}

	if (decoder.state === "configured") {
		try {
			await decoder.flush();
		} catch (error) {
			errors.push(toErrorRecord("flush() rejected", error, null));
		}
	}
	const totalMs = performance.now() - started;
	const finalState = decoder.state;
	if (decoder.state !== "closed") decoder.close();

	const result: DecodeResult = {
		scenario: scenario.name,
		description: scenario.description,
		pacing: scenario.pacing,
		config: toConfigRecord(scenario.config),
		optimizeForLatency: scenario.optimizeForLatency ?? null,
		submitted,
		reconfiguredAt,
		frames,
		errors,
		finalState,
		lateOutputs: pacer.late,
		serialAbandonedAt: pacer.abandonedAt,
		totalMs,
		check: checkFrames(accepted, frames, inputs),
	};
	return { result, thumbnails };
}

function buildScenarios(params: VideoRunParams, wire: WireChunk[], config: WireConfig): DecodeScenario[] {
	const scenarios: DecodeScenario[] = [];
	const add = (scenario: Omit<DecodeScenario, "hardwareAcceleration">) => {
		scenarios.push({ ...scenario, hardwareAcceleration: params.decoderHardwareAcceleration });
	};
	add({ name: "full", description: "all chunks from the first one, decoder configured once with the first decoderConfig", pacing: "serial", config, chunks: wire });
	if (params.scenarios === "reconfigure") {
		add({
			name: "follow-config",
			description: "all chunks, configure() called again with each later decoderConfig",
			pacing: "serial",
			config,
			chunks: wire,
			followConfigs: true,
		});
		return scenarios;
	}
	add({ name: "full-burst", description: "same input, submitted without waiting (throughput)", pacing: "burst", config, chunks: wire });
	if (params.scenarios !== "all") return scenarios;

	add({
		name: "full-optimize-for-latency",
		description: "all chunks, optimizeForLatency: true",
		pacing: "serial",
		config,
		chunks: wire,
		optimizeForLatency: true,
	});
	add({
		name: "delta-first",
		description: "new decoder, first chunk given is a delta (chunk 1)",
		pacing: "serial",
		config,
		chunks: wire.slice(1),
	});

	const keySeqs = wire.filter((chunk) => chunk.type === "key").map((chunk) => chunk.seq);
	const secondKey = keySeqs.at(1);
	if (secondKey !== undefined) {
		add({
			name: "late-join-at-key",
			description: `new decoder, starts at the second key chunk (chunk ${secondKey}), config taken from the first chunk's metadata`,
			pacing: "serial",
			config,
			chunks: wire.slice(secondKey),
		});
		add({
			name: "late-join-mislabelled",
			description: `new decoder, starts at delta chunk ${secondKey + 1} relabelled as type "key"`,
			pacing: "serial",
			config,
			chunks: wire.slice(secondKey + 1).map((chunk, i) => (i === 0 ? { ...chunk, type: "key" } : chunk)),
		});
	}

	const dropped = 5;
	if (wire.length > dropped + 1 && wire[dropped].type === "delta") {
		add({
			name: "drop-one-delta",
			description: `all chunks except delta chunk ${dropped}`,
			pacing: "serial",
			config,
			chunks: wire.filter((chunk) => chunk.seq !== dropped),
		});
	}

	add({
		name: "flush-then-delta",
		description: "flush() after 10 chunks, then continue with delta chunk 10",
		pacing: "serial",
		config,
		chunks: wire,
		flushAfter: 10,
	});
	add({
		name: "no-duration",
		description: "all chunks, built without duration",
		pacing: "serial",
		config,
		chunks: wire,
		omitDuration: true,
	});

	const minimal: WireConfig = { codec: config.codec };
	if (config.description) minimal.description = config.description;
	add({
		name: "minimal-config",
		description: "all chunks, config reduced to codec and description",
		pacing: "serial",
		config: minimal,
		chunks: wire,
	});
	if (config.description) {
		const { description: _omitted, ...withoutDescription } = config;
		add({
			name: "no-description",
			description: "all chunks, config without description",
			pacing: "serial",
			config: withoutDescription,
			chunks: wire,
		});
	}
	return scenarios;
}

async function runVideo(params: VideoRunParams): Promise<VideoRunResult> {
	if (params.scenarios === "encode-only") {
		const { result } = await encodeVideo(params, "burst");
		return { params, encode: result, encodeBurst: null, decodes: [] };
	}

	const serial = await encodeVideo(params, "serial");
	const burst = params.scenarios === "reconfigure" ? null : await encodeVideo(params, "burst");
	const decodes: DecodeResult[] = [];
	if (serial.wireConfig && serial.wire.length > 0) {
		const inputs = new Map(serial.result.inputs.map((input) => [input.timestamp, input]));
		// The first scenario is the plain full decode; its frames are the reference for the others.
		let reference: Thumbnails | null = null;
		for (const scenario of buildScenarios(params, serial.wire, serial.wireConfig)) {
			const decoded = await decodeVideo(scenario, params, inputs, reference);
			reference ??= decoded.thumbnails;
			decodes.push(decoded.result);
		}
	}
	return { params, encode: serial.result, encodeBurst: burst?.result ?? null, decodes };
}

// ---- audio ----------------------------------------------------------------------------------

interface AudioWireChunk {
	type: EncodedAudioChunkType;
	timestamp: number;
	duration: number | null;
	data: Uint8Array;
}

async function decodeAudio(
	name: string,
	description: string,
	config: AudioDecoderConfig,
	chunks: AudioWireChunk[],
): Promise<AudioDecodeResult> {
	const outputs: AudioDecodedRecord[] = [];
	const errors: ErrorRecord[] = [];
	let current: number | null = null;
	const decoder = new AudioDecoder({
		output(data) {
			outputs.push({
				timestamp: data.timestamp,
				duration: data.duration,
				numberOfFrames: data.numberOfFrames,
				sampleRate: data.sampleRate,
				numberOfChannels: data.numberOfChannels,
				format: data.format,
			});
			data.close();
		},
		error(error) {
			errors.push(toErrorRecord("error callback", error, current));
		},
	});
	try {
		decoder.configure(config);
	} catch (error) {
		errors.push(toErrorRecord("configure() threw", error, null));
	}
	for (let i = 0; i < chunks.length && decoder.state === "configured"; i += 1) {
		current = i;
		const wire = chunks[i];
		const init: EncodedAudioChunkInit = { type: wire.type, timestamp: wire.timestamp, data: wire.data };
		if (wire.duration !== null) init.duration = wire.duration;
		try {
			decoder.decode(new EncodedAudioChunk(init));
		} catch (error) {
			errors.push(toErrorRecord("decode() threw", error, i));
		}
	}
	if (decoder.state === "configured") {
		try {
			await decoder.flush();
		} catch (error) {
			errors.push(toErrorRecord("flush() rejected", error, null));
		}
	}
	const finalState = decoder.state;
	if (decoder.state !== "closed") decoder.close();
	return { scenario: name, description, errors, finalState, outputs };
}

async function runAudio(params: AudioRunParams): Promise<AudioRunResult> {
	const encoderConfig: AudioEncoderConfig = {
		codec: params.codec,
		sampleRate: params.sampleRate,
		numberOfChannels: params.numberOfChannels,
		bitrate: params.bitrate,
	};
	const decoderProbe: AudioDecoderConfig = {
		codec: params.codec,
		sampleRate: params.sampleRate,
		numberOfChannels: params.numberOfChannels,
	};

	const support: AudioRunResult["support"] = [];
	try {
		const result = await AudioEncoder.isConfigSupported(encoderConfig);
		support.push({ role: "encoder", config: toJsonRecord(encoderConfig) ?? {}, supported: result.supported === true, error: null });
	} catch (error) {
		support.push({ role: "encoder", config: toJsonRecord(encoderConfig) ?? {}, supported: false, error: String(error) });
	}
	try {
		const result = await AudioDecoder.isConfigSupported(decoderProbe);
		support.push({ role: "decoder", config: toJsonRecord(decoderProbe) ?? {}, supported: result.supported === true, error: null });
	} catch (error) {
		support.push({ role: "decoder", config: toJsonRecord(decoderProbe) ?? {}, supported: false, error: String(error) });
	}

	const inputs: AudioRunResult["inputs"] = [];
	const chunks: AudioChunkRecord[] = [];
	const wire: AudioWireChunk[] = [];
	const encodeErrors: ErrorRecord[] = [];
	const emittedConfigs: AudioDecoderConfig[] = [];
	let current: number | null = null;

	const encoder = new AudioEncoder({
		output(chunk, metadata) {
			const data = new Uint8Array(chunk.byteLength);
			chunk.copyTo(data);
			let decoderConfig: AudioChunkRecord["decoderConfig"] = null;
			if (metadata?.decoderConfig) {
				const emitted = metadata.decoderConfig;
				const description = emitted.description ? toBytes(emitted.description) : null;
				decoderConfig = {
					keys: Object.keys(emitted),
					values: toJsonRecord({ ...emitted, description: undefined }) ?? {},
					codec: emitted.codec,
					sampleRate: emitted.sampleRate ?? null,
					numberOfChannels: emitted.numberOfChannels ?? null,
					descriptionHex: description ? toHex(description) : null,
				};
				const rebuilt: AudioDecoderConfig = {
					codec: emitted.codec,
					sampleRate: emitted.sampleRate,
					numberOfChannels: emitted.numberOfChannels,
				};
				if (description) rebuilt.description = description;
				emittedConfigs.push(rebuilt);
			}
			wire.push({ type: chunk.type, timestamp: chunk.timestamp, duration: chunk.duration, data });
			chunks.push({
				seq: chunks.length,
				type: chunk.type,
				timestamp: chunk.timestamp,
				duration: chunk.duration,
				byteLength: chunk.byteLength,
				metadataKeys: metadata ? Object.keys(metadata) : [],
				decoderConfig,
			});
		},
		error(error) {
			encodeErrors.push(toErrorRecord("error callback", error, current));
		},
	});

	const started = performance.now();
	try {
		encoder.configure(encoderConfig);
	} catch (error) {
		encodeErrors.push(toErrorRecord("configure() threw", error, null));
	}
	for (let index = 0; index < params.inputCount && encoder.state === "configured"; index += 1) {
		current = index;
		const firstFrame = index * params.framesPerInput;
		const timestamp = Math.round((firstFrame * 1_000_000) / params.sampleRate);
		// A 440 Hz sine with continuous phase across inputs, interleaved.
		const samples = new Float32Array(params.framesPerInput * params.numberOfChannels);
		for (let frame = 0; frame < params.framesPerInput; frame += 1) {
			const value = 0.5 * Math.sin((2 * Math.PI * 440 * (firstFrame + frame)) / params.sampleRate);
			for (let channel = 0; channel < params.numberOfChannels; channel += 1) {
				samples[frame * params.numberOfChannels + channel] = value;
			}
		}
		const data = new AudioData({
			format: "f32",
			sampleRate: params.sampleRate,
			numberOfFrames: params.framesPerInput,
			numberOfChannels: params.numberOfChannels,
			timestamp,
			data: samples,
		});
		inputs.push({ index, timestamp, duration: data.duration, numberOfFrames: data.numberOfFrames });
		try {
			encoder.encode(data);
		} catch (error) {
			encodeErrors.push(toErrorRecord("encode() threw", error, index));
		}
		data.close();
	}
	if (encoder.state === "configured") {
		try {
			await encoder.flush();
		} catch (error) {
			encodeErrors.push(toErrorRecord("flush() rejected", error, null));
		}
	}
	const encodeMs = performance.now() - started;
	if (encoder.state !== "closed") encoder.close();

	const decodes: AudioDecodeResult[] = [];
	const emittedConfig = emittedConfigs.at(0);
	if (emittedConfig && wire.length > 0) {
		decodes.push(await decodeAudio("full", "all chunks, decoder config as emitted by the encoder", emittedConfig, wire));
		decodes.push(await decodeAudio("mid-stream", "new decoder, starts at chunk 10", emittedConfig, wire.slice(10)));
		decodes.push(await decodeAudio("probe-config", "all chunks, config with codec, sampleRate and numberOfChannels only", decoderProbe, wire));
	}
	return { params, support, inputs, chunks, encodeErrors, encodeMs, decodes };
}

// ---- environment ----------------------------------------------------------------------------

async function environment(): Promise<EnvironmentInfo> {
	let uaData: Record<string, unknown> | null = null;
	// WHY Reflect: navigator.userAgentData is not in TypeScript 6.0.3's DOM library.
	const data: unknown = Reflect.get(navigator, "userAgentData");
	if (typeof data === "object" && data !== null) {
		const getHighEntropyValues: unknown = Reflect.get(data, "getHighEntropyValues");
		if (typeof getHighEntropyValues === "function") {
			const hints = ["architecture", "bitness", "platform", "platformVersion", "fullVersionList", "model"];
			uaData = toJsonRecord(await Reflect.apply(getHighEntropyValues, data, [hints]));
		}
	}

	let webglVendor: string | null = null;
	let webglRenderer: string | null = null;
	const gl = new OffscreenCanvas(1, 1).getContext("webgl");
	const info = gl?.getExtension("WEBGL_debug_renderer_info");
	if (gl && info) {
		webglVendor = String(gl.getParameter(info.UNMASKED_VENDOR_WEBGL));
		webglRenderer = String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL));
	}

	return {
		userAgent: navigator.userAgent,
		uaData,
		hardwareConcurrency: navigator.hardwareConcurrency,
		crossOriginIsolated: self.crossOriginIsolated,
		webglVendor,
		webglRenderer,
	};
}

window.__spike = { environment, probeSupport, runVideo, runAudio };
