// The page of the meeting. Runs on Bun, with the server (`serve.ts`),
// which serves it where it says.

import { join } from "node:path";
import { chatHtml, type Page, pageHtml, type Served } from "../lib/shell.ts";

const body = (served: Served): string =>
	`<canvas width="640" height="360"></canvas>
		<p><button id="sound">sound</button></p><div id="tiles" style="display: flex; flex-wrap: wrap; gap: 4px;"></div>
		<pre id="status">loading</pre>
		${
			served.roomsUrl === null
				? "<p>no rooms: bun install, and start again; a meeting takes its seat from the room</p>"
				: `${served.noChat === undefined ? chatHtml("6em") : `<p>${served.noChat}</p>`}
		<p><button id="agent-link">link a folder for Claude Code</button> <button id="agent-call">call Claude Code</button> <span id="agent-note"></span></p>`
		}`;

export const MEETING_PAGES: Record<string, Page> = {
	"/meet": {
		entry: join(import.meta.dir, "meet.ts"),
		html: (served) =>
			pageHtml("meet", [{ href: "/meet", text: "meet" }], body(served), "/meet.js"),
	},
};
