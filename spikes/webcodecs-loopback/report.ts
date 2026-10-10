// Turns out/<label>.json into Markdown on stdout.
//
//   bun report.ts out/<label>.json
import type { ChunkRecord, DecodeResult, ErrorRecord, HardwarePreference, RunFile, SupportRow, VideoRunEntry } from "./types";

const path = process.argv[2];
if (!path) throw new Error("usage: bun report.ts out/<label>.json");
const file: RunFile = await Bun.file(path).json();

const lines: string[] = [];
const out = (line = "") => lines.push(line);

function table(header: string[], rows: (string | number)[][]): void {
	out(`| ${header.join(" | ")} |`);
	out(`|${header.map(() => "---").join("|")}|`);
	for (const row of rows) out(`| ${row.join(" | ")} |`);
	out();
}

function quantile(sorted: number[], q: number): number {
	return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
}

// "min / median / p95 / max", or "-" for an empty list.
function spread(values: number[], digits: number): string {
	if (values.length === 0) return "-";
	const sorted = [...values].sort((a, b) => a - b);
	return [sorted[0], quantile(sorted, 0.5), quantile(sorted, 0.95), sorted[sorted.length - 1]].map((value) => value.toFixed(digits)).join(" / ");
}

function numbers(values: (number | null)[]): number[] {
	return values.filter((value) => value !== null);
}

function ranges(values: number[]): string {
	const parts: string[] = [];
	for (let i = 0; i < values.length; i += 1) {
		let end = i;
		while (end + 1 < values.length && values[end + 1] === values[end] + 1) end += 1;
		parts.push(end > i ? `${values[i]}-${values[end]}` : `${values[i]}`);
		i = end;
	}
	return parts.join(",");
}

function summariseErrors(errors: ErrorRecord[]): string {
	if (errors.length === 0) return "none";
	const groups = new Map<string, number[]>();
	for (const error of errors) {
		const key = `${error.where}: ${error.name}: ${error.message}`;
		const at = groups.get(key) ?? [];
		if (error.at !== null) at.push(error.at);
		groups.set(key, at);
	}
	return [...groups].map(([key, at]) => `${key}${at.length > 0 ? ` (at ${ranges(at)})` : ""}`).join("; ");
}

function runName(entry: VideoRunEntry): string {
	const p = entry.params;
	const format = p.family === "h264" || p.family === "hevc" ? ` ${p.format === "annexb" ? "annexb" : p.family === "h264" ? "avc" : "hevc"}` : "";
	return `\`${p.codec}\`${format} ${p.width}x${p.height}`;
}

function implementation(entry: VideoRunEntry, kind: "Encoder" | "Decoder"): string {
	const valuesOf = (prefix: string) => entry.mediaProperties.filter((property) => property.startsWith(prefix)).map((property) => property.slice(prefix.length));
	const names = valuesOf(`kVideo${kind}Name=`);
	if (names.length === 0) return "not reported";
	return `${names.join(", ")} (platform: ${valuesOf(`kIsPlatformVideo${kind}=`).join(", ") || "?"})`;
}

function check(decode: DecodeResult | undefined): string {
	if (!decode?.check) return "not run";
	const c = decode.check;
	const differs =
		c.maxDifferenceFromFull === null
			? ""
			: c.differsFromFullAt.length === 0
				? "; pictures identical to the full decode"
				: `; pictures differ from the full decode at outputs ${ranges(c.differsFromFullAt)} (largest mean difference ${c.maxDifferenceFromFull.toFixed(2)} of 255)`;
	if (c.countMatch && c.increasing && c.timestampsMatch && c.durationsMatch && c.markersMatch) return `${c.decoded}/${c.expected} match${differs}`;
	const wrong = c.wrongPictureAt.length > 0 ? `; wrong frame number at outputs ${ranges(c.wrongPictureAt)}` : "";
	return `${c.decoded}/${c.expected}: ${c.firstMismatch}${wrong}${differs}`;
}

// Serial times of one decode, or "held" when the decoder kept the first frames back and the run stopped waiting.
function serialDecode(decode: DecodeResult | undefined): string {
	if (!decode) return "-";
	if (decode.serialAbandonedAt !== null) return "held";
	const times = spread(numbers(decode.frames.map((frame) => frame.decodeLatencyMs)), 2);
	return decode.lateOutputs > 0 ? `${times} (${decode.lateOutputs} late)` : times;
}

function shortName(entry: VideoRunEntry, kind: "Encoder" | "Decoder"): string {
	return implementation(entry, kind).replace("VideoEncoder", "").replace("VideoDecoder", "").replace("platform: ", "");
}

const loopbackRuns = file.video.filter((entry) => entry.params.scenarios === "all" || entry.params.scenarios === "main");

// ---- environment ----------------------------------------------------------------------------

out(`## ${file.label}`);
out();
out(`- Finished: ${file.finishedAt}`);
out(`- Browser version: ${file.browserVersion}`);
out(`- Executable: \`${file.executablePath}\``);
out(`- Headless: ${file.headless}; extra arguments: ${JSON.stringify(file.chromeArgs)}`);
out(`- User agent: ${file.environment.userAgent}`);
out(`- Platform: ${JSON.stringify(file.environment.uaData)}`);
out(`- hardwareConcurrency ${file.environment.hardwareConcurrency}, crossOriginIsolated ${file.environment.crossOriginIsolated}`);
out(`- WebGL renderer: ${file.environment.webglRenderer}`);
out(`- chrome://gpu feature status: ${file.gpuFeatureStatus ? file.gpuFeatureStatus.join("; ") : "not read"}`);
out();

// ---- support --------------------------------------------------------------------------------

const preferences: HardwarePreference[] = ["no-preference", "prefer-hardware", "prefer-software"];
const sizes = [...new Set(file.support.map((row) => `${row.width}x${row.height}`))];

function supportCell(rows: SupportRow[], role: "encoder" | "decoder", codec: string, size: string): string {
	return preferences
		.map((preference) => {
			const row = rows.find((r) => r.role === role && r.codec === codec && `${r.width}x${r.height}` === size && r.hardwareAcceleration === preference);
			if (!row) return "?";
			if (row.error) return "err";
			return row.supported ? "yes" : "no";
		})
		.join(" / ");
}

for (const role of ["encoder", "decoder"] satisfies SupportRow["role"][]) {
	out(`### isConfigSupported: ${role}`);
	out();
	out("Each cell: `no-preference` / `prefer-hardware` / `prefer-software`.");
	out();
	table(
		["Codec string", "Meaning", ...sizes],
		file.candidates.map((candidate) => [`\`${candidate.codec}\``, candidate.note, ...sizes.map((size) => supportCell(file.support, role, candidate.codec, size))]),
	);
}
const rejected = file.support.filter((row) => row.error);
out(`isConfigSupported rejections: ${rejected.length === 0 ? "none" : [...new Set(rejected.map((row) => `${row.role} ${row.codec}: ${row.error}`))].join("; ")}`);
out();

// ---- loopback -------------------------------------------------------------------------------

const failed = file.video.filter((entry) => entry.result === null);
out("### Runs that did not finish");
out();
out(failed.length === 0 ? "None." : failed.map((entry) => `- ${runName(entry)} ${entry.params.hardwareAcceleration} ${entry.params.scenarios}: ${entry.failure}`).join("\n"));
out();

out("### Loopback (encode, copy to bytes, decode)");
out();
out('"Match" means: same number of frames, output timestamps strictly increasing, each timestamp and duration equal to the input frame\'s, and the frame number read from the decoded picture equal to the input frame\'s.');
out();
table(
	["Run", "Encoder preference", "Encoder used", "Decoder used", "latencyMode", "Frames in", "Chunks out", "Encode errors", "Decoded (full)", "Decoded (full-burst)"],
	loopbackRuns.map((entry) => {
		const result = entry.result;
		if (!result) return [runName(entry), entry.params.hardwareAcceleration, "-", "-", entry.params.latencyMode, "-", "-", entry.failure ?? "", "-", "-"];
		return [
			runName(entry),
			entry.params.hardwareAcceleration,
			implementation(entry, "Encoder"),
			implementation(entry, "Decoder"),
			entry.params.latencyMode,
			result.encode.inputs.length,
			result.encode.chunks.length,
			summariseErrors(result.encode.errors),
			check(result.decodes.find((decode) => decode.scenario === "full")),
			check(result.decodes.find((decode) => decode.scenario === "full-burst")),
		];
	}),
);

// ---- key frames -----------------------------------------------------------------------------

function unitsAt(chunks: ChunkRecord[], seq: number): string {
	const chunk = chunks.at(seq);
	if (!chunk) return "-";
	return chunk.units ? `[${chunk.units.join(",")}]` : "n/a";
}

out("### Key frames and decoderConfig");
out();
out('Units: NAL unit types for H.264 and HEVC, OBU types for AV1. "Label vs bitstream" compares `EncodedVideoChunk.type` with the frame type parsed from the bytes (not parsed for AV1).');
out();
table(
	["Run", "Encoder", "latencyMode", "keyFrame requested at", "Key chunks at", "Label vs bitstream", "decoderConfig on chunks", "description bytes", "Units: chunk 0", "Units: chunk 1", "Units: 2nd key"],
	file.video
		.filter((entry) => entry.params.reconfigure === "none")
		.flatMap((entry) => {
			const result = entry.result;
			if (!result) return [];
			const chunks = result.encode.chunks;
			const keys = chunks.filter((chunk) => chunk.type === "key").map((chunk) => chunk.seq);
			const mislabelled = chunks.filter((chunk) => chunk.bitstreamKey !== null && chunk.bitstreamKey !== (chunk.type === "key"));
			const configs = chunks.filter((chunk) => chunk.decoderConfig);
			const description = configs.at(0)?.decoderConfig?.descriptionHex;
			return [
				[
					runName(entry),
					implementation(entry, "Encoder"),
					entry.params.latencyMode,
					ranges(result.encode.inputs.filter((input) => input.keyRequested).map((input) => input.index)) || "never",
					`${ranges(keys)} of ${chunks.length}`,
					chunks.every((chunk) => chunk.bitstreamKey === null) ? "not parsed" : mislabelled.length === 0 ? "agree" : `differ at ${ranges(mislabelled.map((chunk) => chunk.seq))}`,
					ranges(configs.map((chunk) => chunk.seq)) || "none",
					description ? description.length / 2 : "absent",
					unitsAt(chunks, 0),
					unitsAt(chunks, 1),
					keys.length > 1 ? unitsAt(chunks, keys[1]) : "-",
				],
			];
		}),
);

out("### decoderConfig members as emitted");
out();
const configShapes = new Map<string, string[]>();
for (const entry of file.video) {
	const config = entry.result?.encode.chunks.find((chunk) => chunk.decoderConfig)?.decoderConfig;
	if (!config) continue;
	const { codedWidth: _width, codedHeight: _height, ...rest } = config.values;
	const shape = `${JSON.stringify(rest)}, description ${config.descriptionHex ? `${config.descriptionHex.length / 2} bytes` : "absent"}`;
	configShapes.set(shape, [...(configShapes.get(shape) ?? []), `${runName(entry)} ${entry.params.hardwareAcceleration}`]);
}
out("codedWidth and codedHeight (always present, equal to the configured size) are left out below.");
out();
for (const [shape, names] of configShapes) out(`- ${shape}\n  - ${[...new Set(names)].join("; ")}`);
out();
const descriptions = new Map<string, string>();
for (const entry of loopbackRuns) {
	const hex = entry.result?.encode.chunks.find((chunk) => chunk.decoderConfig)?.decoderConfig?.descriptionHex;
	if (hex) descriptions.set(`${runName(entry)} ${entry.params.hardwareAcceleration} ${entry.params.latencyMode}`, hex);
}
out("Descriptions:");
out();
for (const [name, hex] of descriptions) out(`- ${name}: \`${hex}\``);
out();
const otherMetadata = new Set(file.video.flatMap((entry) => entry.result?.encode.chunks.flatMap((chunk) => chunk.metadataKeys) ?? []));
out(`Metadata members seen on any chunk: ${[...otherMetadata].join(", ") || "none"}`);
out();

// ---- reconfigure ----------------------------------------------------------------------------

out("### Encoder reconfigured halfway");
out();
out("60 frames, `keyFrame` never requested, `configure()` called again before frame 30 with half the bitrate or half the width and height.");
out();
table(
	["Run", "Encoder", "Change", "Chunks out", "Key chunks at", "decoderConfig on chunks", "Second decoderConfig", "Encode errors", "Decoder configured once", "Decoder reconfigured at each decoderConfig"],
	file.video
		.filter((entry) => entry.params.reconfigure !== "none")
		.map((entry) => {
			const result = entry.result;
			if (!result) return [runName(entry), "-", entry.params.reconfigure, "-", "-", "-", "-", entry.failure ?? "", "-", "-"];
			const chunks = result.encode.chunks;
			const configs = chunks.filter((chunk) => chunk.decoderConfig);
			const second = configs.at(1)?.decoderConfig;
			const outcome = (name: string) => {
				const decode = result.decodes.find((d) => d.scenario === name);
				if (!decode) return "not run";
				const reconfigured = decode.reconfiguredAt.length > 0 ? `, configure() before chunk ${decode.reconfiguredAt.join(",")}` : "";
				return `${decode.frames.length} frames, errors: ${summariseErrors(decode.errors)}, ${decode.finalState}, ${check(decode)}${reconfigured}`;
			};
			return [
				runName(entry),
				implementation(entry, "Encoder"),
				entry.params.reconfigure,
				chunks.length,
				ranges(chunks.filter((chunk) => chunk.type === "key").map((chunk) => chunk.seq)),
				ranges(configs.map((chunk) => chunk.seq)) || "none",
				second ? `${second.codedWidth}x${second.codedHeight}, description ${second.descriptionHex ? `${second.descriptionHex.length / 2} bytes` : "absent"}` : "-",
				summariseErrors(result.encode.errors),
				outcome("full"),
				outcome("follow-config"),
			];
		}),
);

// ---- decode scenarios -----------------------------------------------------------------------

out("### Decode scenarios");
out();
out("Runs with the same outcome are listed together.");
out();
const outcomes = new Map<string, Map<string, string[]>>();
for (const entry of file.video.filter((e) => e.params.scenarios === "all")) {
	for (const decode of entry.result?.decodes ?? []) {
		if (decode.scenario === "full-burst") continue;
		const title = `${decode.scenario}: ${decode.description.replace(/chunk \d+/g, "chunk N")}`;
		const outcome = `accepted ${decode.submitted}/${decode.submitted + decode.errors.filter((error) => error.where === "decode() threw").length} submitted, frames out ${decode.frames.length}, errors: ${summariseErrors(decode.errors)}, final state ${decode.finalState}, check: ${check(decode)}`;
		const byOutcome = outcomes.get(title) ?? new Map<string, string[]>();
		byOutcome.set(outcome, [...(byOutcome.get(outcome) ?? []), `${runName(entry)} ${implementation(entry, "Decoder")}`]);
		outcomes.set(title, byOutcome);
	}
}
for (const [title, byOutcome] of outcomes) {
	out(`**${title}**`);
	out();
	for (const [outcome, names] of byOutcome) out(`- ${outcome}\n  - ${names.join("; ")}`);
	out();
}

const noDuration = file.video.flatMap((entry) => entry.result?.decodes.filter((decode) => decode.scenario === "no-duration") ?? []);
out(`Frame durations when chunks carry no duration: ${[...new Set(noDuration.flatMap((decode) => decode.frames.map((frame) => String(frame.duration))))].join(", ") || "not run"}`);
out();
const frameShapes = new Map<string, Set<string>>();
for (const entry of loopbackRuns) {
	for (const decode of entry.result?.decodes ?? []) {
		for (const frame of decode.frames) {
			const shape = `${frame.format} coded ${frame.codedWidth}x${frame.codedHeight}`;
			const key = `${implementation(entry, "Decoder")} ${entry.params.codec} ${entry.params.width}x${entry.params.height}`;
			frameShapes.set(key, (frameShapes.get(key) ?? new Set()).add(shape));
		}
	}
}
out("Decoded VideoFrame format and coded size (display size equalled the encoded size in every run unless listed):");
out();
for (const [key, shapes] of frameShapes) out(`- ${key}: ${[...shapes].join("; ")}`);
out();

// ---- size and time --------------------------------------------------------------------------

out("### Chunk size and time");
out();
out('Sizes in bytes and times in milliseconds, as min / median / p95 / max. "Serial" submits one frame and waits for its output before the next; the time is from `encode()` or `decode()` to the output callback. "Held" means the output of each of the first three chunks did not arrive within 1000 ms while nothing further was submitted; the run then stops waiting and has no serial time. "Burst" submits everything, then flushes; the figure is total time divided by frames, and for encoding it includes drawing the canvas.');
out();
table(
	["Run", "Encoder / decoder", "latencyMode", "Bitrate set", "Key bytes (n)", "Delta bytes (n)", "Measured bit/s", "Encode serial ms", "Encode burst ms/frame", "Decode serial ms", "Decode serial ms, optimizeForLatency", "Decode burst ms/frame"],
	loopbackRuns.flatMap((entry) => {
		const result = entry.result;
		if (!result) return [];
		const chunks = result.encode.chunks;
		const keys = chunks.filter((chunk) => chunk.type === "key");
		const deltas = chunks.filter((chunk) => chunk.type === "delta");
		const full = result.decodes.find((decode) => decode.scenario === "full");
		const burst = result.decodes.find((decode) => decode.scenario === "full-burst");
		const optimized = result.decodes.find((decode) => decode.scenario === "full-optimize-for-latency");
		const seconds = result.encode.inputs.length / entry.params.framerate;
		const bytes = chunks.reduce((sum, chunk) => sum + chunk.byteLength, 0);
		return [
			[
				runName(entry),
				`${shortName(entry, "Encoder")} / ${shortName(entry, "Decoder")}`,
				entry.params.latencyMode,
				entry.params.bitrate,
				`${spread(keys.map((chunk) => chunk.byteLength), 0)} (${keys.length})`,
				`${spread(deltas.map((chunk) => chunk.byteLength), 0)} (${deltas.length})`,
				Math.round((bytes * 8) / seconds),
				`${spread(numbers(chunks.map((chunk) => chunk.encodeLatencyMs)), 2)}${result.encode.lateOutputs > 0 ? ` (${result.encode.lateOutputs} late)` : ""}`,
				result.encodeBurst ? (result.encodeBurst.totalMs / result.encodeBurst.inputs.length).toFixed(2) : "-",
				serialDecode(full),
				optimized ? serialDecode(optimized) : "not run",
				burst && burst.frames.length > 0 ? (burst.totalMs / burst.frames.length).toFixed(2) : "-",
			],
		];
	}),
);

// ---- audio ----------------------------------------------------------------------------------

out("### Audio");
out();
if (file.audio.length === 0) out("Not run.");
for (const run of file.audio) {
	const p = run.params;
	out(`#### \`${p.codec}\` ${p.sampleRate} Hz, ${p.numberOfChannels} ch, ${p.bitrate} bit/s, ${p.inputCount} inputs of ${p.framesPerInput} frames`);
	out();
	out(`- isConfigSupported: ${run.support.map((row) => `${row.role} ${row.error ?? row.supported}`).join(", ")}`);
	out(`- Encode errors: ${summariseErrors(run.encodeErrors)}`);
	const inputFrames = run.inputs.reduce((sum, input) => sum + input.numberOfFrames, 0);
	const types = [...new Set(run.chunks.map((chunk) => chunk.type))];
	const durations = [...new Set(run.chunks.map((chunk) => chunk.duration))];
	out(`- Input: ${run.inputs.length} AudioData, ${inputFrames} frames, timestamps ${run.inputs.at(0)?.timestamp} to ${run.inputs.at(-1)?.timestamp}`);
	out(`- Output: ${run.chunks.length} chunks, types ${types.join(", ")}, durations ${durations.join(", ")} µs, timestamps ${run.chunks.slice(0, 4).map((chunk) => chunk.timestamp).join(", ")} ... ${run.chunks.at(-1)?.timestamp}`);
	out(`- Chunk bytes: ${spread(run.chunks.map((chunk) => chunk.byteLength), 0)}`);
	const configs = run.chunks.filter((chunk) => chunk.decoderConfig);
	const config = configs.at(0)?.decoderConfig;
	out(`- decoderConfig on chunks ${ranges(configs.map((chunk) => chunk.seq)) || "none"}: ${config ? `${JSON.stringify(config.values)}, description ${config.descriptionHex ? `\`${config.descriptionHex}\`` : "absent"}` : "-"}`);
	for (const decode of run.decodes) {
		const frames = decode.outputs.reduce((sum, output) => sum + output.numberOfFrames, 0);
		const shapes = [...new Set(decode.outputs.map((output) => `${output.format} ${output.sampleRate} Hz ${output.numberOfChannels} ch`))];
		const first = decode.outputs.slice(0, 3);
		out(`- Decode "${decode.scenario}" (${decode.description}): ${decode.outputs.length} AudioData, ${frames} frames, first frame counts ${first.map((output) => output.numberOfFrames).join(", ")}, first timestamps ${first.map((output) => output.timestamp).join(", ")}, last ${decode.outputs.at(-1)?.timestamp}, ${shapes.join("; ")}, errors: ${summariseErrors(decode.errors)}, final state ${decode.finalState}`);
	}
	out();
}

console.log(lines.join("\n"));
