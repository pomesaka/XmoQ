// Runs every pairing of `run.ts` in which at least one client is an own one,
// one after the other, and prints a table of the outcomes.
//
// Usage: bun matrix.ts [--relay moqtail] [--relay own] [--base-port N]
//                      [--only TEXT] [--pairing-timeout-seconds N]
//   --relay: the relays to run the pairings through (default: both).
//   --only: runs the pairings whose name contains TEXT. The name begins with
//     the relay, as in "own relay: own-rust to own-browser".
//   --pairing-timeout-seconds: how long one pairing may take before it is
//     stopped and counted as failed.
//
// The exit code is 0 only if every pairing ended as the table below expects.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const here = import.meta.dir;
const logDir = join(here, ".logs", "matrix");

const CLIENTS = [
	"own-browser",
	"own-rust",
	"moqtail-browser",
	"moqtail-rust",
] as const;
type Client = (typeof CLIENTS)[number];

const isOwn = (client: Client): boolean => client.startsWith("own-");

interface Pairing {
	name: string;
	/** The arguments of `run.ts`, without the relay and the port. */
	args: string[];
	/**
	 * Absent when the Object has to arrive. Otherwise the run has to fail, and
	 * its log has to show this: the reason, not only that something failed.
	 */
	expectedFailure?: RegExp;
}

function transfers(): Pairing[] {
	const pairings: Pairing[] = [];
	for (const publisher of CLIENTS) {
		for (const subscriber of CLIENTS) {
			// A run between two moqtail peers checks nothing of this repository.
			if (!isOwn(publisher) && !isOwn(subscriber)) continue;
			// Only the own clients report how their subscription ended.
			const awaitsDone = isOwn(subscriber);
			pairings.push({
				name: `${publisher} to ${subscriber}${awaitsDone ? ", then PUBLISH_DONE" : ""}`,
				args: [
					"--publisher",
					publisher,
					"--subscriber",
					subscriber,
					...(awaitsDone ? ["--await-publish-done"] : []),
				],
			});
		}
	}
	return pairings;
}

/**
 * One publisher and three subscribers: every subscriber has to get the Object
 * and then see PUBLISH_DONE. One row per own client as the subscribers.
 */
const ONE_TO_MANY: Pairing[] = [
	{
		name: "own-rust to 3 own-browser, then PUBLISH_DONE",
		args: [
			"--publisher",
			"own-rust",
			"--subscriber",
			"own-browser",
			"--subscribers",
			"3",
			"--await-publish-done",
		],
	},
	{
		name: "moqtail-browser to 3 own-rust, then PUBLISH_DONE",
		args: [
			"--publisher",
			"moqtail-browser",
			"--subscriber",
			"own-rust",
			"--subscribers",
			"3",
			"--await-publish-done",
		],
	},
];

/** A SUBSCRIBE for a track nobody published: the relay answers REQUEST_ERROR DOES_NOT_EXIST (0x10). */
const REFUSALS: Pairing[] = [
	{
		name: "own-browser subscribes to a track nobody published",
		args: [
			"--publisher",
			"moqtail-rust",
			"--subscriber",
			"own-browser",
			"--subscribe-track",
			"missing",
		],
		expectedFailure: /RequestFailedError: REQUEST_ERROR 0x10/,
	},
	{
		name: "own-rust subscribes to a track nobody published",
		args: [
			"--publisher",
			"moqtail-rust",
			"--subscriber",
			"own-rust",
			"--subscribe-track",
			"missing",
		],
		expectedFailure: /SUBSCRIBE rejected: error_code=0x10/,
	},
];

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", multiple: true, default: ["moqtail", "own"] },
		// WHY one port per pairing: a pairing that is killed at its time limit
		// can leave its relay running, and that relay must not hold the port of
		// the pairings after it.
		"base-port": { type: "string", default: "44400" },
		only: { type: "string", default: "" },
		// Provisional. A pairing takes about a second; `run.ts` gives each of
		// its own steps 20 seconds.
		"pairing-timeout-seconds": { type: "string", default: "120" },
	},
});

/** After SIGTERM, how long `run.ts` gets to stop what it started. */
const KILL_GRACE_MS = 10_000;

/** The pairing that is running, so that a signal to this process reaches it. */
let running: Bun.Subprocess | undefined;
let interrupted = false;

// WHY forwarded: `run.ts` stops its relay, browser and clients when it gets a
// signal. Without this, stopping the matrix leaves them running, and the relay
// keeps its port.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
	process.once(signal, () => {
		interrupted = true;
		running?.kill(signal);
	});
}

/** A pairing and the relay it runs through; the name begins with the relay. */
interface RelayPairing extends Pairing {
	relay: string;
}

interface Outcome {
	pairing: RelayPairing;
	ok: boolean;
	detail: string;
	seconds: number;
	log: string;
}

/** The line of a run's log that says how it ended. */
function lastDriverLine(log: string): string {
	const lines = log
		.split("\n")
		.filter((candidate) => /^\[driver\] (RESULT|FAILED) /.test(candidate));
	return lines.at(-1) ?? "the run printed neither RESULT nor FAILED";
}

async function runPairing(
	pairing: RelayPairing,
	port: number,
	timeoutMs: number,
): Promise<Outcome> {
	const started = performance.now();
	const logFile = join(
		logDir,
		`${pairing.name.replaceAll(/[^a-z0-9]+/gi, "-").toLowerCase()}.log`,
	);
	const relay = pairing.relay;
	const child = Bun.spawn(
		[
			process.execPath,
			join(here, "run.ts"),
			...pairing.args,
			"--relay",
			relay,
			"--port",
			String(port),
		],
		{ cwd: here, stdout: "pipe", stderr: "pipe" },
	);
	running = child;
	let timedOut = false;
	let killer: ReturnType<typeof setTimeout> | undefined;
	// SIGTERM first, so that `run.ts` stops its relay and clients itself.
	const limit = setTimeout(() => {
		timedOut = true;
		child.kill("SIGTERM");
		killer = setTimeout(() => child.kill("SIGKILL"), KILL_GRACE_MS);
	}, timeoutMs);
	const [stdout, stderr, exitCode] = await Promise.all([
		new Response(child.stdout).text(),
		new Response(child.stderr).text(),
		child.exited,
	]);
	clearTimeout(limit);
	clearTimeout(killer);
	running = undefined;
	const log = stdout + stderr;
	writeFileSync(logFile, log);
	const seconds = (performance.now() - started) / 1000;
	const outcome = (ok: boolean, detail: string): Outcome => ({
		pairing,
		ok,
		detail,
		seconds,
		log: logFile,
	});

	// Checked first: a pairing stopped at its limit proved nothing, also where
	// a failure was the expected outcome.
	if (timedOut) {
		return outcome(
			false,
			`stopped after the limit of ${timeoutMs / 1000}s; ${lastDriverLine(log)}`,
		);
	}
	if (pairing.expectedFailure === undefined) {
		const matched = exitCode === 0 && /"match":true/.test(log);
		return outcome(
			matched,
			matched ? "the payload matched" : lastDriverLine(log),
		);
	}
	const refused = exitCode !== 0 && pairing.expectedFailure.test(log);
	return outcome(
		refused,
		refused
			? `failed as expected (${pairing.expectedFailure.source})`
			: `expected a failure showing ${pairing.expectedFailure.source}; got exit ${exitCode}: ${lastDriverLine(log)}`,
	);
}

async function main(): Promise<void> {
	const pairings: RelayPairing[] = options.relay
		.flatMap((relay) =>
			[...transfers(), ...ONE_TO_MANY, ...REFUSALS].map((pairing) => ({
				...pairing,
				name: `${relay} relay: ${pairing.name}`,
				relay,
			})),
		)
		.filter((pairing) => pairing.name.includes(options.only));
	if (pairings.length === 0) {
		throw new Error(`no pairing contains "${options.only}"`);
	}
	// WHY the digits are checked: `Number("")` is 0.
	if (!/^[0-9]+$/.test(options["base-port"])) {
		throw new Error(`--base-port "${options["base-port"]}" is not a port`);
	}
	const basePort = Number(options["base-port"]);
	const lastPort = basePort + pairings.length - 1;
	if (basePort < 1 || lastPort > 65535) {
		throw new Error(
			`--base-port ${basePort} with ${pairings.length} pairings needs the ports ${basePort} to ${lastPort}, outside 1 to 65535`,
		);
	}
	const timeoutSeconds = Number(options["pairing-timeout-seconds"]);
	if (
		!/^[0-9]+(\.[0-9]+)?$/.test(options["pairing-timeout-seconds"]) ||
		timeoutSeconds <= 0
	) {
		throw new Error(
			`--pairing-timeout-seconds "${options["pairing-timeout-seconds"]}" is not a positive number`,
		);
	}
	mkdirSync(logDir, { recursive: true });

	const started = performance.now();
	const outcomes: Outcome[] = [];
	// WHY one after the other: the pairings share one certificate directory
	// and one Chromium installation, and a failure is easier to read alone.
	for (const [index, pairing] of pairings.entries()) {
		// A signal stops the pairing that is running; the rest is not started.
		if (interrupted) break;
		const outcome = await runPairing(
			pairing,
			basePort + index,
			timeoutSeconds * 1000,
		);
		outcomes.push(outcome);
		console.log(
			`${outcome.ok ? "ok  " : "FAIL"} ${pairing.name} (${outcome.seconds.toFixed(1)}s): ${outcome.detail}`,
		);
		if (!outcome.ok) {
			console.log(`---- ${outcome.log}`);
			console.log(readFileSync(outcome.log, "utf8"));
			console.log("----");
		}
	}

	const failed = outcomes.filter((outcome) => !outcome.ok);
	const total = ((performance.now() - started) / 1000).toFixed(1);
	const table = [
		"| Pairing | Result | Seconds |",
		"|---|---|---|",
		...outcomes.map(
			(outcome) =>
				`| ${outcome.pairing.name} | ${outcome.ok ? outcome.detail : `**FAILED**: ${outcome.detail}`} | ${outcome.seconds.toFixed(1)} |`,
		),
	].join("\n");
	console.log(`\n${table}\n`);
	console.log(
		`${outcomes.length - failed.length} of ${outcomes.length} pairings ended as expected in ${total}s; logs in ${logDir}`,
	);
	// The job summary of GitHub Actions shows the same table.
	const summary = process.env.GITHUB_STEP_SUMMARY;
	if (summary !== undefined) {
		writeFileSync(summary, `${table}\n`, { flag: "a" });
	}
	if (interrupted) {
		console.log(
			`interrupted after ${outcomes.length} of ${pairings.length} pairings`,
		);
	}
	if (failed.length > 0 || interrupted) process.exitCode = 1;
}

main().catch((error: unknown) => {
	console.log(
		`FAILED ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
