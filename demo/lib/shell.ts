// The frame of a page of the demo, which each application fills with its
// own body (`pages.ts` of each). Runs on Bun, with the server.

/** A page: its title, the links of its application, and its body. */
export function pageHtml(
	title: string,
	links: { href: string; text: string }[],
	body: string,
	script: string,
): string {
	return `<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<title>XmoQ demo: ${title}</title>
		<link rel="icon" href="data:," />
		<style>
			body { font-family: monospace; background: #111; color: #ddd; margin: 16px; }
			canvas { display: block; max-width: 100%; background: #000; }
			a { color: #f2c14e; }
		</style>
	</head>
	<body>
		<p>XmoQ demo, ${title} · ${links.map((link) => `<a href="${link.href}">${link.text}</a>`).join(" · ")}</p>
		${body}
		<script type="module" src="${script}"></script>
	</body>
</html>
`;
}

/** What the server tells the pages of an application, and why a part is missing where it is. */
export interface Served {
	/** The rooms, where they run. */
	roomsUrl: string | null;
	/** Why the pages have no chat, where the gatherer does not run. */
	noChat: string | undefined;
	/** Why the pages have no captions, where the transcriber does not run. */
	noCaptions: string | undefined;
}

/** A page an application serves: the script bundled for it, and its HTML. */
export interface Page {
	entry: string;
	html(served: Served): string;
}

/** The chat under the picture: its lines, a form and a note. */
export const chatHtml = (height: string): string =>
	`<div id="chat" style="height: ${height}; overflow-y: auto; border: 1px solid #444; padding: 4px;"></div>
		<form><input size="40" placeholder="say something" /> <button>send</button></form>
		<p id="chat-note"></p>`;
