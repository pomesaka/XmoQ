// A UDP relay that loses, delays and jitters what passes through it, for a
// QUIC connection between a page and the relay. No setting of the system is
// touched.
//
// Usage: node proxy.mjs --listen 45386 --to 45382 [--control 45387]
//
// The link is changed while it runs:
//   curl 'http://127.0.0.1:45387/set?loss=0.05&delay=50&jitter=10'
// `loss` is the share of datagrams that are dropped, each way; `delay` and
// `jitter` are milliseconds: a datagram is held for `delay` plus up to
// `jitter`, each way, so datagrams can pass each other. `/stats` says how
// many passed and how many were dropped.
import { createSocket } from "node:dgram";
import { createServer } from "node:http";

const argument = (name, otherwise) => {
	const at = process.argv.indexOf(`--${name}`);
	return at === -1 ? otherwise : Number(process.argv[at + 1]);
};
const listen = argument("listen", 45386);
const to = argument("to", 4443);
const control = argument("control", listen + 1);

const link = { loss: 0, delay: 0, jitter: 0 };
const stats = { passed: 0, dropped: 0 };
const pass = (send) => {
	if (Math.random() < link.loss) {
		stats.dropped += 1;
		return;
	}
	stats.passed += 1;
	const wait = link.delay + Math.random() * link.jitter;
	if (wait <= 0) send();
	else setTimeout(send, wait);
};

// One socket towards the relay for every client address, so that the
// relay's answers find their way back.
const front = createSocket("udp4");
const clients = new Map();
front.on("message", (data, from) => {
	const key = `${from.address}:${from.port}`;
	let back = clients.get(key);
	if (back === undefined) {
		back = createSocket("udp4");
		back.on("message", (answer) =>
			pass(() => front.send(answer, from.port, from.address)),
		);
		clients.set(key, back);
	}
	const upstream = back;
	pass(() => upstream.send(data, to, "127.0.0.1"));
});
front.bind(listen, "127.0.0.1");

createServer((request, response) => {
	const url = new URL(request.url ?? "/", "http://127.0.0.1");
	if (url.pathname === "/set") {
		for (const name of ["loss", "delay", "jitter"]) {
			const value = url.searchParams.get(name);
			if (value !== null) link[name] = Number(value);
		}
	}
	response.setHeader("content-type", "application/json");
	response.end(JSON.stringify({ link, stats }));
}).listen(control, "127.0.0.1", () => {
	console.log(`link: udp 127.0.0.1:${listen} -> 127.0.0.1:${to}; control http://127.0.0.1:${control}`);
});
