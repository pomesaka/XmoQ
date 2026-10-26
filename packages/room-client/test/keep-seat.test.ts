import { afterEach, beforeEach, expect, jest, test } from "bun:test";
import { keepSeat, type Seat } from "../src/index.ts";

const ROOM = { rooms: "http://rooms.test/", name: "blue" };

const seat = (expiresAt: number, token: string): Seat => ({
	participant: "a1",
	publishToken: token,
	subscribeToken: null,
	lease: "l1",
	expiresAt,
	renewMs: 20_000,
});

const seatBody = (expiresAt: number, token: string) => ({
	participant: "a1",
	publish: { namespace: ["xmoq", "meet", "blue", "a1"], token },
	subscribe: { namespace: ["xmoq", "meet", "blue", "*"], token: null },
	lease: "l1",
	expiresAt,
	renewMs: 20_000,
});

const realFetch = globalThis.fetch;
let asked: { url: string; method: string; body: string }[] = [];
let answers: Response[] = [];

beforeEach(() => {
	jest.useFakeTimers();
	asked = [];
	answers = [];
	globalThis.fetch = Object.assign(
		async (input: string | URL | Request, init?: RequestInit) => {
			asked.push({
				url: String(input),
				method: init?.method ?? "GET",
				body: String(init?.body ?? ""),
			});
			const answer = answers.shift();
			if (answer === undefined) throw new Error("no answer");
			return answer;
		},
		{ preconnect: realFetch.preconnect },
	);
});
afterEach(() => {
	globalThis.fetch = realFetch;
	jest.useRealTimers();
});

const turns = async (): Promise<void> => {
	for (let turn = 0; turn < 20; turn++) await Promise.resolve();
};

test("renews the seat every renewMs with its lease, and has the newest tokens", async () => {
	answers.push(Response.json(seatBody(80_000, "t2")));
	const kept = keepSeat(ROOM, seat(60_000, "t1"));
	jest.advanceTimersByTime(19_999);
	await turns();
	expect(asked).toEqual([]);
	jest.advanceTimersByTime(1);
	await turns();
	expect(asked).toEqual([
		{
			url: "http://rooms.test/meetings/blue/seats/a1/renew",
			method: "POST",
			body: '{"lease":"l1"}',
		},
	]);
	expect(kept.seat.publishToken).toBe("t2");
	expect(kept.seat.expiresAt).toBe(80_000);
});

test.each([
	{
		name: "a seat that is gone",
		answer: () => Response.json({ gone: true }, { status: 404 }),
		lost: { type: "gone" },
	},
	{
		name: "an answer it does not take",
		answer: () => new Response("", { status: 500 }),
		lost: { type: "failed" },
	},
])(
	"$name: the seat is lost, and is not renewed again",
	async ({ answer, lost }) => {
		answers.push(answer());
		const kept = keepSeat(ROOM, seat(60_000, "t1"));
		jest.advanceTimersByTime(20_000);
		await turns();
		expect(await kept.lost).toMatchObject(lost);
		jest.advanceTimersByTime(60_000);
		await turns();
		expect(asked.length).toBe(1);
	},
);

test("leave stops the renewals and deletes the seat with its lease", async () => {
	answers.push(new Response(null, { status: 204 }));
	const kept = keepSeat(ROOM, seat(60_000, "t1"));
	await kept.leave();
	jest.advanceTimersByTime(60_000);
	await turns();
	expect(asked).toEqual([
		{
			url: "http://rooms.test/meetings/blue/seats/a1",
			method: "DELETE",
			body: '{"lease":"l1"}',
		},
	]);
});
