// Opens the demo's publishing page in a headless Chromium and keeps it open
// until SIGTERM or SIGINT, for `scripts/agent-demo.sh`, which has no window to
// open it in.
//
// Usage: bun demo/open-publish.ts --port 33080 [--voice FILE.wav | --tone]
//
// With `--voice` (the script's default) the page sends that file as its
// microphone (`?audio=mic`), which Chromium plays over and over; with
// `--tone` the page's own tone, in which the transcriber hears no words.
import { parseArgs } from "node:util";
import { chromium } from "playwright-core";

const { values: options } = parseArgs({
	options: {
		port: { type: "string", default: "8080" },
		voice: { type: "string" },
		tone: { type: "boolean", default: false },
	},
});
if (options.tone === (options.voice !== undefined)) {
	throw new Error("give one of --voice FILE.wav and --tone");
}
const browser = await chromium.launch({
	headless: true,
	// As interop/harness.ts launches it: the full browser that
	// `playwright-core install chromium` puts in.
	channel: "chromium",
	// WHY off: closing the browser here, before this process ends, is what
	// leaves no Chromium behind when `scripts/agent-demo.sh stop` ends this
	// process by its PID.
	handleSIGINT: false,
	handleSIGTERM: false,
	args:
		options.voice === undefined
			? []
			: [
					"--use-fake-ui-for-media-stream",
					"--use-fake-device-for-media-stream",
					`--use-file-for-fake-audio-capture=${options.voice}`,
				],
});
let closing = false;
const close = async (): Promise<void> => {
	closing = true;
	await browser.close();
	process.exit(0);
};
process.on("SIGTERM", close);
process.on("SIGINT", close);
browser.on("disconnected", () => {
	if (closing) return;
	console.log("the browser is gone");
	process.exit(1);
});
const page = await browser.newPage();
page.on("console", (message) => console.log(`[publish] ${message.text()}`));
page.on("pageerror", (error) => console.log(`[publish] pageerror: ${error.message}`));
const url = `http://localhost:${options.port}/publish${options.voice === undefined ? "" : "?audio=mic"}`;
await page.goto(url);
console.log(`opened ${url} in Chromium ${browser.version()}`);
