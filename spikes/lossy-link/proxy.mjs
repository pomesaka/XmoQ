// A UDP relay that loses, delays and jitters what passes through it, for a
// QUIC connection between a page and the relay. No setting of the system is
// touched.
//
// Usage: node proxy.mjs --listen 45386 --to 45382 [--control 45387]
//
// The link is changed while it runs:
//   curl 'http://127.0.0.1:45387/set?loss=0.05&delay=50&jitter=10'
// `loss` is the share of datagrams that are dropped, each way (`up` and
// `down` the same for one way alone: towards the relay, and from it); `delay` and
// `jitter` are milliseconds: a datagram is held for `delay` plus up to
// `jitter`, each way, so datagrams can pass each other. `/stats` says how
// many passed and how many were dropped.
//
// A link with no more than so much room: `rate` (kbit/s of UDP payload, each
// way on its own, shared by every client; 0 for no limit) and `queue` (how
// many milliseconds of datagrams may wait for the link; one that would wait
// longer is dropped, and counted as `overflowed`). `delay` comes after the wait.
//   curl 'http://127.0.0.1:45387/set?rate=3000&queue=100&delay=50'
import { createSocket } from "node:dgram";
import { createServer } from "node:http";

const argument = (name, otherwise) => {
	const at = process.argv.indexOf(`--${name}`);
	return at === -1 ? otherwise : Number(process.argv[at + 1]);
};
const listen = argument("listen", 45386);
const to = argument("to", 4443);
const control = argument("control", listen + 1);

// `up` and `down` are loss for one way alone, on top of `loss`: from the
// client to the relay, and from the relay to the client.
const link = { loss: 0, delay: 0, jitter: 0, up: 0, down: 0, rate: 0, queue: 100 };
const stats = { passed: 0, dropped: 0, overflowed: 0 };
// When the link of each way has sent what is waiting for it. No queue is
// kept: a datagram's turn is counted when it comes, and a timer sends it then.
const free = { up: 0, down: 0 };
// How long the datagram waits for the link, or undefined if the queue is full.
const waitForLink = (way, length) => {
	if (link.rate <= 0) return 0;
	const now = performance.now();
	const begins = Math.max(now, free[way]);
	if (begins - now > link.queue) return undefined;
	free[way] = begins + (length * 8) / link.rate;
	return free[way] - now;
};
// Every datagram of the last while: when, which way, how long, and whether
// it was dropped. `/log` gives them and begins anew.
let log = [];
const pass = (send, way, length) => {
	const dropped = Math.random() < link.loss || Math.random() < link[way];
	if (log.length < 200_000) log.push([Date.now(), way, length, dropped ? 1 : 0]);
	if (dropped) {
		stats.dropped += 1;
		return;
	}
	const queued = waitForLink(way, length);
	if (queued === undefined) {
		stats.overflowed += 1;
		if (log.length > 0) log[log.length - 1][3] = 2;
		return;
	}
	stats.passed += 1;
	const wait = queued + link.delay + Math.random() * link.jitter;
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
			pass(() => front.send(answer, from.port, from.address), "down", answer.length),
		);
		clients.set(key, back);
	}
	const upstream = back;
	pass(() => upstream.send(data, to, "127.0.0.1"), "up", data.length);
});
front.bind(listen, "127.0.0.1");

createServer((request, response) => {
	const url = new URL(request.url ?? "/", "http://127.0.0.1");
	if (url.pathname === "/set") {
		for (const name of ["loss", "delay", "jitter", "up", "down", "rate", "queue"]) {
			const value = url.searchParams.get(name);
			if (value !== null) link[name] = Number(value);
		}
	}
	response.setHeader("content-type", "application/json");
	if (url.pathname === "/log") {
		response.end(JSON.stringify(log));
		log = [];
		return;
	}
	response.end(JSON.stringify({ link, stats }));
}).listen(control, "127.0.0.1", () => {
	console.log(`link: udp 127.0.0.1:${listen} -> 127.0.0.1:${to}; control http://127.0.0.1:${control}`);
});
