// Shapes shared by the page (page.ts) and the driver (run.ts, report.ts).
// Everything here is JSON-serialisable, because it crosses page.evaluate.

export type HardwarePreference = "no-preference" | "prefer-hardware" | "prefer-software";
export type CodecFamily = "h264" | "hevc" | "vp8" | "vp9" | "av1";
export type BitstreamFormat = "native" | "annexb";

export interface Resolution {
	width: number;
	height: number;
}

export interface CodecCandidate {
	family: CodecFamily;
	codec: string;
	note: string;
}

export interface SupportRow {
	role: "encoder" | "decoder";
	family: CodecFamily;
	codec: string;
	width: number;
	height: number;
	hardwareAcceleration: HardwarePreference;
	supported: boolean;
	// Set when isConfigSupported rejected instead of resolving.
	error: string | null;
}

export interface EnvironmentInfo {
	userAgent: string;
	uaData: Record<string, unknown> | null;
	hardwareConcurrency: number;
	crossOriginIsolated: boolean;
	webglVendor: string | null;
	webglRenderer: string | null;
}

export interface VideoRunParams {
	family: CodecFamily;
	codec: string;
	width: number;
	height: number;
	framerate: number;
	bitrate: number;
	frameCount: number;
	// Frames with index % keyFrameInterval === 0 are encoded with { keyFrame: true }.
	// null: keyFrame is never requested.
	keyFrameInterval: number | null;
	hardwareAcceleration: HardwarePreference;
	// Applied to the VideoDecoder of every decode scenario.
	decoderHardwareAcceleration: HardwarePreference;
	latencyMode: "quality" | "realtime";
	// "native" is the registration's default: "avc" for H.264, "hevc" for HEVC. Ignored by other families.
	format: BitstreamFormat;
	scenarios: "all" | "main" | "encode-only" | "reconfigure";
	// Halfway through, call VideoEncoder.configure() again with half the bitrate or half the width and height.
	reconfigure: "none" | "bitrate" | "size";
}

export interface DecoderConfigRecord {
	keys: string[];
	// Every member except description, as JSON.
	values: Record<string, unknown>;
	codec: string;
	codedWidth: number | null;
	codedHeight: number | null;
	displayAspectWidth: number | null;
	displayAspectHeight: number | null;
	colorSpace: Record<string, unknown> | null;
	descriptionHex: string | null;
}

export interface ChunkRecord {
	seq: number;
	type: string;
	timestamp: number;
	duration: number | null;
	byteLength: number;
	// Time from encode() to the output callback. null in burst mode.
	encodeLatencyMs: number | null;
	metadataKeys: string[];
	decoderConfig: DecoderConfigRecord | null;
	temporalLayerId: number | null;
	// NAL unit types (H.264, HEVC) or OBU types (AV1) found in the chunk. null for VP8 and VP9.
	units: number[] | null;
	// Key/delta as read from the bitstream itself. null when the parser has no rule for the family.
	bitstreamKey: boolean | null;
}

export interface ErrorRecord {
	where: string;
	name: string;
	message: string;
	// Index of the chunk or frame being submitted when the error surfaced, if known.
	at: number | null;
}

export interface EncodeResult {
	pacing: "serial" | "burst";
	configure: Record<string, unknown>;
	reconfigure: { at: number; config: Record<string, unknown> } | null;
	inputs: { index: number; timestamp: number; duration: number; keyRequested: boolean }[];
	chunks: ChunkRecord[];
	errors: ErrorRecord[];
	finalState: string;
	// Frames that had no output within the wait window in serial mode (encoder kept them until later).
	lateOutputs: number;
	// Serial mode stops waiting after several late outputs in a row; frames from this index on have no per-frame time.
	serialAbandonedAt: number | null;
	totalMs: number;
}

export interface FrameRecord {
	timestamp: number;
	duration: number | null;
	codedWidth: number;
	codedHeight: number;
	displayWidth: number;
	displayHeight: number;
	format: string | null;
	// Frame number read back from the picture. null when markers were not read.
	marker: number | null;
	// Mean absolute difference per colour channel (0..255) between a 64x36 thumbnail of this frame and
	// the thumbnail of the frame with the same timestamp in the plain full decode. null when not compared.
	differenceFromFull: number | null;
	decodeLatencyMs: number | null;
}

export interface DecodeCheck {
	// Chunks that decode() accepted without throwing.
	expected: number;
	decoded: number;
	countMatch: boolean;
	// Output timestamps strictly increasing.
	increasing: boolean;
	timestampsMatch: boolean;
	durationsMatch: boolean;
	markersMatch: boolean;
	// Output indices whose picture shows another frame number than the chunk they came from.
	wrongPictureAt: number[];
	// Output indices whose thumbnail differs from the plain full decode, and the largest difference.
	differsFromFullAt: number[];
	maxDifferenceFromFull: number | null;
	firstMismatch: string | null;
}

export interface DecodeResult {
	scenario: string;
	description: string;
	pacing: "serial" | "burst";
	config: DecoderConfigRecord | null;
	optimizeForLatency: boolean | null;
	submitted: number;
	// Chunk indices before which configure() was called again.
	reconfiguredAt: number[];
	frames: FrameRecord[];
	errors: ErrorRecord[];
	finalState: string;
	lateOutputs: number;
	serialAbandonedAt: number | null;
	totalMs: number;
	check: DecodeCheck | null;
}

export interface VideoRunResult {
	params: VideoRunParams;
	encode: EncodeResult;
	encodeBurst: EncodeResult | null;
	decodes: DecodeResult[];
}

export interface AudioRunParams {
	codec: string;
	sampleRate: number;
	numberOfChannels: number;
	bitrate: number;
	// Frames per AudioData handed to the encoder.
	framesPerInput: number;
	inputCount: number;
}

export interface AudioChunkRecord {
	seq: number;
	type: string;
	timestamp: number;
	duration: number | null;
	byteLength: number;
	metadataKeys: string[];
	decoderConfig: (Omit<DecoderConfigRecord, "codedWidth" | "codedHeight" | "displayAspectWidth" | "displayAspectHeight" | "colorSpace"> & {
		sampleRate: number | null;
		numberOfChannels: number | null;
	}) | null;
}

export interface AudioDecodedRecord {
	timestamp: number;
	duration: number;
	numberOfFrames: number;
	sampleRate: number;
	numberOfChannels: number;
	format: string | null;
}

export interface AudioDecodeResult {
	scenario: string;
	description: string;
	errors: ErrorRecord[];
	finalState: string;
	outputs: AudioDecodedRecord[];
}

export interface AudioRunResult {
	params: AudioRunParams;
	support: { role: "encoder" | "decoder"; config: Record<string, unknown>; supported: boolean; error: string | null }[];
	inputs: { index: number; timestamp: number; duration: number; numberOfFrames: number }[];
	chunks: AudioChunkRecord[];
	encodeErrors: ErrorRecord[];
	encodeMs: number;
	decodes: AudioDecodeResult[];
}

export interface SpikeApi {
	environment(): Promise<EnvironmentInfo>;
	probeSupport(candidates: CodecCandidate[], resolutions: Resolution[], framerate: number): Promise<SupportRow[]>;
	runVideo(params: VideoRunParams): Promise<VideoRunResult>;
	runAudio(params: AudioRunParams): Promise<AudioRunResult>;
}

export interface VideoRunEntry {
	params: VideoRunParams;
	result: VideoRunResult | null;
	// Set when the run threw or did not finish in time.
	failure: string | null;
	// name=value properties reported through the DevTools Media domain while the run was active.
	mediaProperties: string[];
}

export interface RunFile {
	label: string;
	finishedAt: string;
	browserVersion: string;
	executablePath: string;
	headless: boolean;
	chromeArgs: string[];
	environment: EnvironmentInfo;
	// Feature status lines scraped from chrome://gpu. null when the page could not be read.
	gpuFeatureStatus: string[] | null;
	candidates: CodecCandidate[];
	support: SupportRow[];
	video: VideoRunEntry[];
	audio: AudioRunResult[];
}

// WHY bits-per-pixel: the spike needs some bitrate for every resolution it probes.
// 0.1 bit per pixel per frame is a provisional value for this check, not a project setting.
export function provisionalBitrate(resolution: Resolution, framerate: number): number {
	return Math.round(resolution.width * resolution.height * framerate * 0.1);
}
