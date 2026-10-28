// The page of the pet. Runs on Bun, with the server (`serve.ts`), which
// serves it where it says.

import { join } from "node:path";
import { type Page, pageHtml } from "../lib/shell.ts";

export const PET_PAGES: Record<string, Page> = {
	"/pet": {
		entry: join(import.meta.dir, "pet.ts"),
		html: () =>
			pageHtml(
				"pet",
				[{ href: "/pet", text: "pet" }],
				`<canvas width="960" height="600" style="cursor: crosshair; border-radius: 6px;"></canvas>
		<p id="controls"></p>
		<pre id="status">loading</pre>`,
				"/pet.js",
			),
	},
};
