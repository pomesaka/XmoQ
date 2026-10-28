// The garden of a room (`crates/garden`, `xmoq-garden`) and its pet
// (`crates/pet`, `xmoq-pet`) through the own relay, with pages as actors of
// the garden (`joinGarden` of `@xmoq/pet`): each announces
// `xmoq/pet/<room>/<name>`, says what it does to the garden on its track
// `acted`, and follows the room's `garden` and the pet's `state` with
// SUBSCRIBE and a Joining FETCH of the latest Group. The pet runs without
// `--decisions-url`, so its fixed table decides (`fixed` of
// crates/pet/src/decide.rs).
//
// Usage: bun pet.ts [--relay own] [--port N] [--verbose]
//
// What has to be seen, written before the first run (2026-10-28):
//
//   - The garden and the pet each print `WATCHING prefix=xmoq/pet`.
//   - Page ann joins room r: the garden prints `ACTOR room="r" name="ann"`
//     and `SUBSCRIBED room="r" name="ann"` (the page is heard), and the pet
//     prints `ANNOUNCED room="r" ...` and `SUBSCRIBED room="r"`. The page is
//     given a garden and a state.
//   - ann feeds to [300, 700]: the page is given a garden with a bowl at
//     [300, 700] by "ann"; the pet prints `DECIDED room="r" source=table
//     ... reaction=eat ...` and `ACTED_SENT room="r" kind=eat thing=<the
//     bowl>`, the garden `ACTED room="r" name="pet" kind=eat`; the page is
//     then given a state doing `eat` and a garden without the bowl.
//   - ann throws the ball to [800, 600]: a garden with a ball at [800, 600]
//     by "ann"; `reaction=chase` and `ACTED_SENT ... kind=take thing=<the
//     ball>`; a state doing `chase` and a garden without the ball.
//   - ann pets: a garden with an event `pet` by "ann", and a state doing
//     `approach` by "ann" with the bubble `♥`.
//   - The garden prints no `ACTED_DROPPED`, and no page is given an Object
//     of `garden` or `state` that is none.
//   - Page bob comes then, and is given a garden and a state within 2 s of
//     beginning to join. How long each took, and whether the garden and the
//     pet answered a FETCH meanwhile (`FETCH_ANSWERED room="r"`), go in the
//     summary: both also send their track again every 2 s
//     (`GARDEN_AGAIN_MS`, `STATE_AGAIN_MS`), so the first may come either
//     way.

import { parseArgs } from "node:util";
import type { Page } from "playwright-core";
import {
	captureGroup,
	type LoggedProcess,
	log,
	openConnectedPage,
	pollFor,
	type RunContext,
	runCleanups,
	spawnGarden,
	spawnPet,
	startRun,
	stopEverythingOnExit,
	withTimeout,
} from "./harness.ts";
import type {
	OwnPageExtras,
	PageGarden,
	PageGardenAction,
	PageGardenReport,
	PagePetState,
} from "./xmoq-page.ts";

// The driver is type-checked without the DOM library; `window` is only used
// inside the functions that Playwright runs in the page.
declare const window: { readonly xmoqOwn: OwnPageExtras };

const { values: options } = parseArgs({
	options: {
		relay: { type: "string", default: "own" },
		port: { type: "string", default: "24399" },
		verbose: { type: "boolean", default: false },
	},
});

stopEverythingOnExit();

const ROOM = "r";

/** How soon a page that comes has to be given a garden and a state: by the time each is sent again (`GARDEN_AGAIN_MS`, `STATE_AGAIN_MS`). */
const GIVEN_WITHIN_MS = 2_000;

function expect(what: string, got: string, wanted: string): void {
	if (got !== wanted) throw new Error(`${what}: ${got}, not ${wanted}`);
}

const report = (page: Page): Promise<PageGardenReport> =>
	page.evaluate(() => window.xmoqOwn.gardenReport());

/** Opens a page and has it join the garden as `name`; returns it and when it began to join. */
async function actor(
	context: RunContext,
	name: string,
): Promise<{ page: Page; joiningAt: number }> {
	const page = await openConnectedPage(context, `page:${name}`, "own-browser");
	const joiningAt = Date.now();
	await withTimeout(
		page.evaluate(([room, as]) => window.xmoqOwn.joinGarden(room, as), [
			ROOM,
			name,
		] satisfies [string, string]),
		`page ${name} to join the garden`,
	);
	return { page, joiningAt };
}

/** Has the page do `action`, and returns what became of it: "sent", "kept-back <why>" or "not-sent <why>". */
async function act(page: Page, action: PageGardenAction): Promise<string> {
	const outcome: unknown = JSON.parse(
		await page.evaluate((done) => window.xmoqOwn.gardenAct(done), action),
	);
	if (typeof outcome !== "object" || outcome === null) {
		throw new Error(`no outcome: ${String(outcome)}`);
	}
	const type = "type" in outcome ? String(outcome.type) : "?";
	return "why" in outcome ? `${type} ${String(outcome.why)}` : type;
}

/** A state, as the run compares it. */
const shown = (state: PagePetState): string =>
	`doing=${state.doing} by=${state.by ?? "none"} bubble=${state.bubble ?? "none"}`;

/** A garden, as the summary shows it. */
const floor = (garden: PageGarden): string =>
	garden.things
		.map((thing) => `${thing.kind} ${thing.id} at ${thing.at.join(",")}`)
		.join("; ") || "nothing";

/** Waits for the page to be given a garden, after the first `after` it had, that `fits`; returns it and its index. */
async function gardenOf(
	page: Page,
	after: number,
	fits: (garden: PageGarden) => boolean,
	what: string,
): Promise<{ garden: PageGarden; index: number }> {
	return pollFor(async () => {
		const { gardens } = await report(page);
		for (let index = after; index < gardens.length; index += 1) {
			const garden = gardens[index];
			if (garden !== undefined && fits(garden)) return { garden, index };
		}
		return undefined;
	}, `the page to be given a garden ${what}`);
}

/** Waits for the page to be given a state, after the first `after` it had, that `fits`. */
async function stateOf(
	page: Page,
	after: number,
	fits: (state: PagePetState) => boolean,
	what: string,
): Promise<PagePetState> {
	return pollFor(async () => {
		const { states } = await report(page);
		return states.slice(after).find(fits);
	}, `the page to be given a state ${what}`);
}

/**
 * Has ann put a thing down with `action`, and waits for what the pet does
 * of it: the garden with the thing, the pet's decision and its action on
 * the garden, a state doing `doing`, and a garden without the thing.
 */
async function putDown(
	garden: LoggedProcess,
	pet: LoggedProcess,
	ann: Page,
	put: {
		action: PageGardenAction & { to: [number, number] };
		thing: "bowl" | "ball";
		reaction: string;
		sent: "eat" | "take";
		doing: string;
	},
): Promise<string> {
	const before = await report(ann);
	expect(
		`what became of ${put.action.kind}`,
		await act(ann, put.action),
		"sent",
	);
	const at = put.action.to.join(",");
	const { garden: down, index } = await gardenOf(
		ann,
		before.gardens.length,
		(given) =>
			given.things.some(
				(thing) =>
					thing.kind === put.thing &&
					thing.at.join(",") === at &&
					thing.by === "ann",
			),
		`with a ${put.thing} at [${at}] by ann`,
	);
	const thing = down.things.find(
		(given) => given.kind === put.thing && given.at.join(",") === at,
	);
	if (thing === undefined) throw new Error(`no ${put.thing} in ${floor(down)}`);
	const decided = await pet.waitForLine(
		new RegExp(
			`^DECIDED room="r" source=table why=\\S+ reaction=${put.reaction} mood=\\d+ bubble=\\S+$`,
		),
	);
	const sent = await pet.waitForLine(
		new RegExp(
			`^ACTED_SENT room="r" kind=${put.sent} thing=${thing.id} group=\\d+$`,
		),
	);
	await garden.waitForLine(
		new RegExp(`^ACTED room="r" name="pet" kind=${put.sent}$`),
	);
	const state = await stateOf(
		ann,
		before.states.length,
		(given) => given.doing === put.doing,
		`doing ${put.doing}`,
	);
	expect(
		`the state after ${put.action.kind}`,
		`doing=${state.doing} by=${state.by ?? "none"}`,
		`doing=${put.doing} by=ann`,
	);
	const { garden: after } = await gardenOf(
		ann,
		index + 1,
		(given) => given.things.every((other) => other.id !== thing.id),
		`without the ${put.thing} ${thing.id}`,
	);
	return `${put.action.kind} to [${at}]: a garden with ${floor(down)}, ${decided[0]}, ${sent[0]}, a state ${shown(state)}, then a garden with ${floor(after)}`;
}

async function main(): Promise<void> {
	if (options.relay !== "own") {
		throw new Error(`--relay is own, not ${options.relay}`);
	}
	let summary: string;
	try {
		const context = await startRun({
			relay: "own",
			port: options.port,
			clients: ["own-browser"],
			verbose: options.verbose,
		});
		const garden = spawnGarden(context, "garden");
		const pet = spawnPet(context, "pet");
		await garden.waitForLine(/^WATCHING prefix=xmoq\/pet$/);
		await pet.waitForLine(/^WATCHING prefix=xmoq\/pet$/);

		const { page: ann } = await actor(context, "ann");
		await garden.waitForLine(/^ACTOR room="r" name="ann"$/);
		await garden.waitForLine(/^SUBSCRIBED room="r" name="ann"$/);
		await pet.waitForLine(/^ANNOUNCED room="r" /);
		await pet.waitForLine(/^SUBSCRIBED room="r"$/);
		// WHY a garden and a state on the page before the first action: the
		// pet takes the first garden it reads as it is, without reacting, so
		// a bowl put down before it read one would be taken for one that was
		// there.
		await pollFor(async () => {
			const { heard, gardens, states } = await report(ann);
			return heard && gardens.length > 0 && states.length > 0
				? true
				: undefined;
		}, "page ann to be heard and given a garden and a state");

		const fed = await putDown(garden, pet, ann, {
			action: { kind: "feed", to: [300, 700] },
			thing: "bowl",
			reaction: "eat",
			sent: "eat",
			doing: "eat",
		});
		const thrown = await putDown(garden, pet, ann, {
			action: { kind: "throw", to: [800, 600] },
			thing: "ball",
			reaction: "chase",
			sent: "take",
			doing: "chase",
		});

		const beforePet = await report(ann);
		expect(
			"what became of the petting",
			await act(ann, { kind: "pet" }),
			"sent",
		);
		const { garden: petted } = await gardenOf(
			ann,
			beforePet.gardens.length,
			(given) =>
				given.events.some(
					(event) => event.kind === "pet" && event.by === "ann",
				),
			"with an event pet by ann",
		);
		const decided = await pet.waitForLine(
			/^DECIDED room="r" source=table why=\S+ reaction=approach mood=\d+ bubble=♥$/,
		);
		const approach = await stateOf(
			ann,
			beforePet.states.length,
			(state) => state.doing === "approach",
			"doing approach",
		);
		expect(
			"the state after the petting",
			shown(approach),
			"doing=approach by=ann bubble=♥",
		);
		const event = petted.events.find((given) => given.kind === "pet");

		const gardenFetches = garden.matches(/^FETCH_ANSWERED room="r" /).length;
		const petFetches = pet.matches(/^FETCH_ANSWERED room="r" /).length;
		const { page: bob, joiningAt } = await actor(context, "bob");
		const firstGarden = await gardenOf(bob, 0, () => true, "at all");
		const firstState = await stateOf(bob, 0, () => true, "at all");
		const gardenTook = firstGarden.garden.seenAt - joiningAt;
		const stateTook = firstState.seenAt - joiningAt;
		if (gardenTook > GIVEN_WITHIN_MS || stateTook > GIVEN_WITHIN_MS) {
			throw new Error(
				`page bob was given its first garden ${gardenTook} ms and its first state ${stateTook} ms after it began to join, not both within ${GIVEN_WITHIN_MS} ms`,
			);
		}
		const answered = (side: LoggedProcess, before: number): string =>
			side
				.matches(/^FETCH_ANSWERED room="r" (.*)$/)
				.slice(before)
				.map((match) => captureGroup(match, 1))
				.join("; ") || "none";

		const dropped = garden.matches(/^ACTED_DROPPED /).map((match) => match[0]);
		if (dropped.length > 0) {
			throw new Error(`the garden dropped actions: ${dropped.join("; ")}`);
		}
		const { malformed: annMalformed } = await report(ann);
		const { malformed: bobMalformed } = await report(bob);
		if (annMalformed.length + bobMalformed.length > 0) {
			throw new Error(
				`a page had an Object that was none: ${[...annMalformed, ...bobMalformed].join(", ")}`,
			);
		}
		summary = `through the own relay, xmoq-garden and xmoq-pet without the decisions API, page ann joined the garden of room r; ${fed}; ${thrown}; pet: a garden with the event pet ${event?.id ?? "?"} by ann, ${decided[0]}, a state ${shown(approach)}; page bob, coming after, was given its first garden (${floor(firstGarden.garden)}) ${gardenTook} ms and its first state (${shown(firstState)}) ${stateTook} ms after it began to join; FETCH answered meanwhile by the garden: ${answered(garden, gardenFetches)}, by the pet: ${answered(pet, petFetches)}`;
	} finally {
		await runCleanups();
	}
	log("driver", `RESULT ${JSON.stringify({ relay: "own" })}`);
	log("driver", `SUMMARY ${summary}`);
}

main().catch((error: unknown) => {
	log(
		"driver",
		`FAILED pet: ${error instanceof Error ? error.message : String(error)}`,
	);
	process.exitCode = 1;
});
