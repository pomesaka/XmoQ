// The pages of the broadcast: one that publishes, and one that watches.
// Runs on Bun, with the server (`serve.ts`), which serves them where it
// says.

import { join } from "node:path";
import { chatHtml, type Page, pageHtml, type Served } from "../lib/shell.ts";

/** The reactions a page offers: the demo's choice. */
const REACTIONS = ["👍", "❤️", "😂", "👏"];

const LINKS = [
	{ href: "/publish", text: "publish" },
	{ href: "/watch", text: "watch" },
];

/** The room's line and its chat, or why there is neither. */
const roomHtml = (served: Served): string =>
	served.roomsUrl === null
		? "<p>no rooms: bun install, and start again</p>"
		: `<p id="room"></p>
		${
			served.noChat === undefined
				? `${chatHtml("8em")}
		<p>${REACTIONS.map((emoji) => `<button data-reaction="${emoji}">${emoji}</button>`).join(" ")} <span id="reactions"></span></p>`
				: `<p>${served.noChat}</p>`
		}`;

export const BROADCAST_PAGES: Record<string, Page> = {
	"/publish": {
		entry: join(import.meta.dir, "publish.ts"),
		html: (served) =>
			pageHtml(
				"publish",
				LINKS,
				`<canvas width="640" height="360"></canvas>
		<p id="controls"></p>
		<pre id="status">loading</pre>
		${roomHtml(served)}`,
				"/publish.js",
			),
	},
	"/watch": {
		entry: join(import.meta.dir, "watch.ts"),
		html: (served) =>
			pageHtml(
				"watch",
				LINKS,
				`<canvas width="640" height="360"></canvas>
		<p id="caption" style="min-height: 1.5em; font-size: 1.2em; color: #fff;">${served.noCaptions ?? ""}</p><p id="controls"><button id="sound">sound</button></p>
		<pre id="status">loading</pre>
		${roomHtml(served)}`,
				"/watch.js",
			),
	},
};
