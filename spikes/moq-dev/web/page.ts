// Phase 0 spike page: publish or receive exactly one MoQ object with @moq/net.
// Driven by query parameters so run.ts can reuse one page for every case.
import * as Moq from "@moq/net";

interface SpikeState {
	events: string[];
	// Set once the role has finished: the publisher wrote its object, or the subscriber compared one.
	done: boolean;
	ok: boolean;
	received?: string;
	error?: string;
}

declare global {
	interface Window {
		__spike: SpikeState;
	}
	// The DOM lib bundled with TypeScript does not declare the `protocols` option yet.
	interface WebTransportOptions {
		protocols?: string[];
	}
}

const state: SpikeState = { events: [], done: false, ok: false };
window.__spike = state;

function event(message: string) {
	state.events.push(message);
	console.log(`[spike] ${message}`);
}

function required(params: URLSearchParams, name: string): string {
	const value = params.get(name);
	if (!value) throw new Error(`missing query parameter: ${name}`);
	return value;
}

async function connect(params: URLSearchParams, wiring: { publish?: Moq.Origin.Consumer; consume?: Moq.Origin.Producer }) {
	const url = new URL(required(params, "relay"));
	// WHY websocket is disabled: @moq/net races a WebSocket fallback against WebTransport.
	// The spike must prove the WebTransport path, so the fallback may not win.
	const websocket = { enabled: false };
	// `protocols` replaces the library's default offer, which is how a single version is pinned.
	const protocols = params.get("protocols")?.split(",");
	const webtransport = protocols ? { protocols } : undefined;
	event(`connecting relay=${url} protocols=${protocols?.join(",") ?? "(library default)"}`);
	const connection = await Moq.Connection.connect({ url, websocket, webtransport, ...wiring });
	event("connected");
	return connection;
}

async function publish(params: URLSearchParams) {
	const payload = required(params, "payload");
	const origin = new Moq.Origin.Producer();
	const connection = await connect(params, { publish: origin.consume() });

	const broadcast = origin.createBroadcast(Moq.Path.from(required(params, "broadcast")));
	const track = broadcast.createTrack(required(params, "track"), { timescale: Moq.Time.Timescale.MILLI });
	broadcast.announce();

	const write = () => {
		const group = track.appendGroup();
		group.writeFrame({ payload: new TextEncoder().encode(payload), timestamp: Moq.Time.Timestamp.now() });
		group.close();
	};

	write();
	// The track stays open so a subscriber that arrives later is still served.
	event(`published payload=${JSON.stringify(payload)}`);
	state.ok = true;
	state.done = true;

	// Without `interval` the object is written once, before any subscriber exists, so a
	// subscriber only gets it as a late join. With `interval` the same payload is written
	// again as a new group every N ms, so a subscriber also sees live delivery.
	const interval = Number(params.get("interval") ?? 0);
	if (interval > 0) setInterval(write, interval);

	await connection.closed;
}

async function subscribe(params: URLSearchParams) {
	const expected = required(params, "payload");
	const origin = new Moq.Origin.Producer();
	const connection = await connect(params, { consume: origin });

	const broadcast = origin.request(Moq.Path.from(required(params, "broadcast")));
	let active = broadcast.active.peek();
	while (!active) {
		await broadcast.active.changed();
		active = broadcast.active.peek();
	}
	event("broadcast is announced");

	const track = active.track(required(params, "track")).subscribe({ priority: 0 });
	const group = await track.recvGroup();
	if (!group) throw new Error("track ended without a group");
	const received = await group.readString();
	if (received === undefined) throw new Error("group ended without a frame");

	state.received = received;
	event(`received payload=${JSON.stringify(received)}`);
	if (received !== expected) throw new Error(`payload mismatch: expected ${JSON.stringify(expected)}`);
	event("match");
	state.ok = true;
	state.done = true;

	connection.close();
	origin.close();
}

async function main() {
	const params = new URLSearchParams(location.search);
	const role = required(params, "role");
	switch (role) {
		case "publish":
			return publish(params);
		case "subscribe":
			return subscribe(params);
		default:
			throw new Error(`unknown role: ${role}`);
	}
}

main().catch((cause: unknown) => {
	state.error = cause instanceof Error ? `${cause.name}: ${cause.message}` : String(cause);
	event(`error ${state.error}`);
	state.done = true;
});
