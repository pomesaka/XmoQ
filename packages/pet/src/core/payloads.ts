// What an actor does on `acted`, the garden as it is now on `garden`, and
// where the pet is and what it does on `state`: the JSON of
// `fixtures/pet-tracks/`, which `xmoq-pet-tracks` reads too, so that the
// same bytes give the same action, garden or state, or the same reason why
// there is none.
//
// `acted`, one Object a Group: `{"kind": ..., "to": [x, y] | "text": ... | "thing": id, "sentAt": number}`.
// `garden`, one Object a Group: `{"things", "events", "at"}`, a thing
// `{"id", "kind", "at", "by"}` and an event `{"id", "kind", "by", "text", "at"}`.
// `state`, one Object a Group: `{"from", "to", "began", "arrives", "facing",
// "doing", "mood", "bubble", "by", "at"}`, in that order. A time is
// milliseconds since the Unix epoch, by the clock of who wrote it.

import {
	type JsonValue,
	membersOf,
	readJsonPayload,
	wholeNumberOf,
} from "@xmoq/json";

/**
 * The far side of the floor: a place is two whole numbers from 0 to it.
 * WHY whole numbers: `JSON.parse` and `serde_json` write a fraction, and a
 * whole number of a float, differently (`1` and `1.0`).
 */
export const FLOOR = 1000;

/** The most bytes of UTF-8 the text of `talk` may have. */
export const TALK_BYTES = 120;

/** The most bytes of UTF-8 the pet's `bubble` may have. */
export const BUBBLE_BYTES = 64;

/** The gladdest mood; 0 is the crossest. */
export const MOOD_MOST = 4;

/** The most things on the floor of a garden. */
export const MOST_THINGS = 8;

/** The most events a garden holds. */
export const MOST_EVENTS = 16;

/** The largest time, and the largest id, taken: 2^53 - 1, the largest integer a JavaScript number holds exactly. */
const MAX_TIME = Number.MAX_SAFE_INTEGER;

/** A place on the floor: across the room, and into it. */
export type Place = [number, number];

/**
 * What an actor does, before it is sent. Who may do which is the garden's
 * to say: a page sends the first four, the pet `eat` and `take`.
 */
export type Action =
	| { kind: "pet" | "call" }
	/** Puts a bowl of food at `to` (`feed`), or throws the ball to `to` (`throw`). */
	| { kind: "feed" | "throw"; to: Place }
	/** Says `text`, 1 to `TALK_BYTES` bytes of UTF-8. */
	| { kind: "talk"; text: string }
	/** Eats from the bowl `thing` (`eat`), or takes the ball `thing` in its mouth (`take`): its id. */
	| { kind: "eat" | "take"; thing: number };

/** An action as it is on `acted`. */
export type Acted = Action & {
	/** The sender's clock; only its form is checked. */
	sentAt: number;
};

const THING_KINDS = ["bowl", "ball"] as const;
export type ThingKind = (typeof THING_KINDS)[number];

/** A thing on the floor of a garden. */
export interface Thing {
	/** Used once in a room, things and events counted apart. */
	id: number;
	kind: ThingKind;
	/** Where it lies. */
	at: Place;
	/** The name of the actor that put it there. */
	by: string;
}

const EVENT_KINDS = ["call", "pet", "talk"] as const;
export type EventKind = (typeof EVENT_KINDS)[number];

/**
 * What happened in a garden that leaves nothing on the floor. WHY NOT
 * `Event`: the DOM has a global of that name, which a page that imports
 * this would have to tell apart.
 */
export interface GardenEvent {
	/** Used once in a room; a reader that read it before knows it again by it. */
	id: number;
	kind: EventKind;
	/** The name of the actor that did it. */
	by: string;
	/** The words of a `talk`, 1 to `TALK_BYTES` bytes of UTF-8; null for any other kind. */
	text: string | null;
	/** The garden's clock when it took it. */
	at: number;
}

/** The garden as it is now. */
export interface Garden {
	/** At most `MOST_THINGS`. */
	things: Thing[];
	/** The newest last, at most `MOST_EVENTS`. */
	events: GardenEvent[];
	/** The garden's clock when it wrote this. */
	at: number;
}

const FACINGS = ["left", "right"] as const;
export type Facing = (typeof FACINGS)[number];

const DOINGS = [
	"idle",
	"walk",
	"sit",
	"sleep",
	"jump",
	"eat",
	"chase",
	"flee",
	"approach",
] as const;
export type Doing = (typeof DOINGS)[number];

/**
 * The pet, as it publishes itself: it goes from `from` at `began` to `to`
 * at `arrives` (the same place and time where it stays). A reader places
 * it at a moment between them by its own clock against `at`.
 */
export interface State {
	from: Place;
	to: Place;
	began: number;
	/** Not before `began`. */
	arrives: number;
	facing: Facing;
	doing: Doing;
	/** From 0 (cross) to `MOOD_MOST` (glad). */
	mood: number;
	/** What it says, 1 to `BUBBLE_BYTES` bytes of UTF-8. */
	bubble: string | null;
	/** The name of whom it reacts to. */
	by: string | null;
	/** The pet's clock when it wrote this. */
	at: number;
}

/** Why a payload of `acted`, `garden` or `state` is none: the words of the tables. */
export type Malformed =
	/** Not UTF-8, not JSON, or no JSON object. */
	| "not-an-object"
	/** Of `acted`: `kind` is missing or none of the kinds. */
	| "kind"
	/** Of `acted`: `to` on a `feed` or a `throw` is missing or no place, or is there on another kind. Of `state`: no place. */
	| "to"
	/** Of `acted`: `text` on a `talk` is missing, no string, empty or longer than `TALK_BYTES`, or is there on another kind. */
	| "text"
	/** Of `acted`: `thing` on an `eat` or a `take` is missing or no id, or is there on another kind. */
	| "thing"
	/** Of `acted`: `sentAt` is no time. */
	| "sent-at"
	/** Of `garden`: `things` is missing, no array, longer than `MOST_THINGS`, or holds what is no thing. */
	| "things"
	/** Of `garden`: `events` is missing, no array, longer than `MOST_EVENTS`, or holds what is no event. */
	| "events"
	| "from"
	| "began"
	/** No time, or before `began`. */
	| "arrives"
	| "facing"
	| "doing"
	| "mood"
	/** Neither null nor a string of 1 to `BUBBLE_BYTES` bytes of UTF-8. */
	| "bubble"
	/** Neither null nor a string that is not empty. */
	| "by"
	| "at";

export type Read<T> = { ok: true; value: T } | { ok: false; why: Malformed };

const encoder = new TextEncoder();

const members = (payload: Uint8Array): Map<string, JsonValue> | undefined =>
	membersOf(readJsonPayload(payload));

const string = (value: JsonValue | undefined): string | undefined =>
	value?.kind === "string" ? value.value : undefined;

/** A time: a whole number from 0 to `MAX_TIME`, as `as_u64` of `serde_json` takes it. */
const time = (value: JsonValue | undefined): number | undefined =>
	wholeNumberOf(value, MAX_TIME);

const isNull = (value: JsonValue | undefined): boolean =>
	value?.kind === "literal" && value.value === null;

function place(value: JsonValue | undefined): Place | undefined {
	if (value?.kind !== "array" || value.items.length !== 2) return undefined;
	const x = wholeNumberOf(value.items[0], FLOOR);
	const y = wholeNumberOf(value.items[1], FLOOR);
	return x === undefined || y === undefined ? undefined : [x, y];
}

const bytesOf = (text: string): number => encoder.encode(text).length;

/** Whether `text` is a string of 1 to `most` bytes of UTF-8 that can be written. */
const fits = (text: string, most: number): boolean =>
	isWellFormed(text) && text !== "" && bytesOf(text) <= most;

function oneOf<T extends string>(
	words: readonly T[],
	value: JsonValue | undefined,
): T | undefined {
	const text = string(value);
	return words.find((word) => word === text);
}

const KINDS = ["pet", "feed", "call", "throw", "talk", "eat", "take"] as const;

/**
 * Reads a payload of `acted`. Members other than those of its kind are
 * passed over, so that a sender may add some; `to`, `text` or `thing` on a
 * kind that has none is no action. The order of the checks is that of
 * `read_action` of `xmoq-pet-tracks`: `kind`, the member of the kind, the
 * first of `to`, `text` and `thing` there on a kind without it, `sentAt`.
 */
export function readActed(payload: Uint8Array): Read<Acted> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const kind = oneOf(KINDS, object.get("kind"));
	if (kind === undefined) return { ok: false, why: "kind" };
	const action = actionOf(kind, object);
	if (typeof action === "string") return { ok: false, why: action };
	const sentAt = time(object.get("sentAt"));
	if (sentAt === undefined) return { ok: false, why: "sent-at" };
	return { ok: true, value: { ...action, sentAt } };
}

const ACTION_MEMBERS = ["to", "text", "thing"] as const;
type ActionMember = (typeof ACTION_MEMBERS)[number];

/** The action of `kind` with its `to`, `text` or `thing`, or the member that makes it none. */
function actionOf(
	kind: Action["kind"],
	object: Map<string, JsonValue>,
): Action | ActionMember {
	const action = ownMemberOf(kind, object);
	if (typeof action === "string") return action;
	const foreign = ACTION_MEMBERS.find(
		(member) => !(member in action) && object.has(member),
	);
	return foreign ?? action;
}

/** The action of `kind` with the member it has, or that member where it is wrong. */
function ownMemberOf(
	kind: Action["kind"],
	object: Map<string, JsonValue>,
): Action | ActionMember {
	switch (kind) {
		case "pet":
		case "call":
			return { kind };
		case "feed":
		case "throw": {
			const to = place(object.get("to"));
			return to === undefined ? "to" : { kind, to };
		}
		case "talk": {
			const text = string(object.get("text"));
			return text === undefined || !fits(text, TALK_BYTES)
				? "text"
				: { kind, text };
		}
		case "eat":
		case "take": {
			const thing = time(object.get("thing"));
			return thing === undefined ? "thing" : { kind, thing };
		}
	}
}

/** The array of at most `most` objects, each one `read`; undefined where any is not. */
function listed<T>(
	value: JsonValue | undefined,
	most: number,
	read: (object: Map<string, JsonValue>) => T | undefined,
): T[] | undefined {
	if (value?.kind !== "array" || value.items.length > most) return undefined;
	const items: T[] = [];
	for (const item of value.items) {
		const object = membersOf(item);
		const got = object === undefined ? undefined : read(object);
		if (got === undefined) return undefined;
		items.push(got);
	}
	return items;
}

/** A name of an actor: a string that is not empty. */
const nameOf = (value: JsonValue | undefined): string | undefined => {
	const text = string(value);
	return text !== undefined && fits(text, Number.POSITIVE_INFINITY)
		? text
		: undefined;
};

function thingOf(object: Map<string, JsonValue>): Thing | undefined {
	const id = time(object.get("id"));
	const kind = oneOf(THING_KINDS, object.get("kind"));
	const at = place(object.get("at"));
	const by = nameOf(object.get("by"));
	return id === undefined ||
		kind === undefined ||
		at === undefined ||
		by === undefined
		? undefined
		: { id, kind, at, by };
}

/** WHY `text` must be there, as null on a kind other than `talk`: the garden writes every member, as the pet does `bubble`. */
function eventOf(object: Map<string, JsonValue>): GardenEvent | undefined {
	const id = time(object.get("id"));
	const kind = oneOf(EVENT_KINDS, object.get("kind"));
	const by = nameOf(object.get("by"));
	const at = time(object.get("at"));
	const value = object.get("text");
	if (
		id === undefined ||
		kind === undefined ||
		by === undefined ||
		at === undefined ||
		value === undefined
	) {
		return undefined;
	}
	if (kind !== "talk") {
		return isNull(value) ? { id, kind, by, text: null, at } : undefined;
	}
	const text = string(value);
	return text !== undefined && fits(text, TALK_BYTES)
		? { id, kind, by, text, at }
		: undefined;
}

/**
 * Reads a payload of `garden`. Members other than the ones named, of the
 * garden, a thing or an event, are passed over; one thing or event that is
 * wrong makes the whole no garden, as `read_garden` of `xmoq-pet-tracks`.
 */
export function readGarden(payload: Uint8Array): Read<Garden> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const things = listed(object.get("things"), MOST_THINGS, thingOf);
	if (things === undefined) return { ok: false, why: "things" };
	const events = listed(object.get("events"), MOST_EVENTS, eventOf);
	if (events === undefined) return { ok: false, why: "events" };
	const at = time(object.get("at"));
	if (at === undefined) return { ok: false, why: "at" };
	return { ok: true, value: { things, events, at } };
}

/** `bubble` and `by`: null, or a string `fits` takes. */
function nullOr(
	value: JsonValue | undefined,
	most: number,
): { read: string | null } | undefined {
	if (isNull(value)) return { read: null };
	const text = string(value);
	return text !== undefined && fits(text, most) ? { read: text } : undefined;
}

/** Reads a payload of `state`. Members other than the ten are passed over. */
export function readState(payload: Uint8Array): Read<State> {
	const object = members(payload);
	if (object === undefined) return { ok: false, why: "not-an-object" };
	const from = place(object.get("from"));
	if (from === undefined) return { ok: false, why: "from" };
	const to = place(object.get("to"));
	if (to === undefined) return { ok: false, why: "to" };
	const began = time(object.get("began"));
	if (began === undefined) return { ok: false, why: "began" };
	const arrives = time(object.get("arrives"));
	if (arrives === undefined || arrives < began) {
		return { ok: false, why: "arrives" };
	}
	const facing = oneOf(FACINGS, object.get("facing"));
	if (facing === undefined) return { ok: false, why: "facing" };
	const doing = oneOf(DOINGS, object.get("doing"));
	if (doing === undefined) return { ok: false, why: "doing" };
	const mood = wholeNumberOf(object.get("mood"), MOOD_MOST);
	if (mood === undefined) return { ok: false, why: "mood" };
	const bubble = nullOr(object.get("bubble"), BUBBLE_BYTES);
	if (bubble === undefined) return { ok: false, why: "bubble" };
	const by = nullOr(object.get("by"), Number.POSITIVE_INFINITY);
	if (by === undefined) return { ok: false, why: "by" };
	const at = time(object.get("at"));
	if (at === undefined) return { ok: false, why: "at" };
	return {
		ok: true,
		value: {
			from,
			to,
			began,
			arrives,
			facing,
			doing,
			mood,
			bubble: bubble.read,
			by: by.read,
			at,
		},
	};
}

/** A UTF-16 unit of a surrogate that has no partner. */
const LONE_SURROGATE =
	/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

/**
 * Whether a string can be sent: a JavaScript string can hold a surrogate
 * without its partner, which `JSON.stringify` writes as an escape that
 * `serde_json` refuses.
 */
const isWellFormed = (text: string): boolean => !LONE_SURROGATE.test(text);

/**
 * Why `action` is no action a reader takes, or undefined where it is one:
 * the place of a `feed` or a `throw` beyond the floor, the text of a
 * `talk` that does not fit `TALK_BYTES` or has a lone surrogate, or the
 * `thing` of an `eat` or a `take` that is no id.
 */
export function checkAction(action: Action): ActionMember | undefined {
	switch (action.kind) {
		case "feed":
		case "throw":
			return isPlace(action.to) ? undefined : "to";
		case "talk":
			return fits(action.text, TALK_BYTES) ? undefined : "text";
		case "eat":
		case "take":
			return isWhole(action.thing) ? undefined : "thing";
		default:
			return undefined;
	}
}

const isWhole = (value: number): boolean =>
	Number.isSafeInteger(value) && value >= 0;

const isPlace = (at: Place): boolean =>
	at.every(
		(field) => Number.isSafeInteger(field) && field >= 0 && field <= FLOOR,
	);

function jsonString(text: string): string {
	if (!isWellFormed(text)) {
		throw new RangeError("a string with a lone surrogate cannot be sent");
	}
	// `JSON.stringify` escapes what `serde_json` escapes, in the same way:
	// `"`, `\` and the control characters, with `\u00xx` in lower case for
	// those that have no short escape (the row "talk with what a terminal
	// would act on").
	return JSON.stringify(text);
}

function checkTime(what: string, value: number): void {
	if (!isWhole(value)) {
		throw new RangeError(
			`${what} is a whole number from 0 to 2^53 - 1, not ${value}`,
		);
	}
}

function placeJson(what: string, at: Place): string {
	if (!isPlace(at)) {
		throw new RangeError(
			`${what} is two whole numbers from 0 to ${FLOOR}, not ${at}`,
		);
	}
	return `[${at[0]},${at[1]}]`;
}

/**
 * The payload of an action on `acted`, its members in the order of the
 * table. Throws a `RangeError` for what `checkAction` refuses and for a
 * time no reader takes.
 */
export function actedPayload(acted: Acted): Uint8Array {
	const why = checkAction(acted);
	if (why !== undefined) throw new RangeError(`no action: ${why}`);
	checkTime("sentAt", acted.sentAt);
	const kind = `"kind":${JSON.stringify(acted.kind)}`;
	const sentAt = `"sentAt":${acted.sentAt}`;
	switch (acted.kind) {
		case "feed":
		case "throw":
			return encoder.encode(
				`{${kind},"to":${placeJson("to", acted.to)},${sentAt}}`,
			);
		case "talk":
			return encoder.encode(
				`{${kind},"text":${jsonString(acted.text)},${sentAt}}`,
			);
		case "eat":
		case "take":
			return encoder.encode(`{${kind},"thing":${acted.thing},${sentAt}}`);
		default:
			return encoder.encode(`{${kind},${sentAt}}`);
	}
}

function nameJson(what: string, name: string): string {
	if (!fits(name, Number.POSITIVE_INFINITY)) {
		throw new RangeError(`${what} is a name that is not empty`);
	}
	return jsonString(name);
}

function thingJson(thing: Thing): string {
	checkTime("the id of a thing", thing.id);
	return `{"id":${thing.id},"kind":${JSON.stringify(thing.kind)},"at":${placeJson("at", thing.at)},"by":${nameJson("by", thing.by)}}`;
}

function eventJson(event: GardenEvent): string {
	checkTime("the id of an event", event.id);
	checkTime("the at of an event", event.at);
	if ((event.kind === "talk") !== (event.text !== null)) {
		throw new RangeError("an event has a text if and only if it is a talk");
	}
	return `{"id":${event.id},"kind":${JSON.stringify(event.kind)},"by":${nameJson("by", event.by)},"text":${nullOrJson("text", event.text, TALK_BYTES)},"at":${event.at}}`;
}

/**
 * The payload of a garden on `garden`, its members in the order of the
 * table; the garden writes it, and the tests write it back to see that it
 * is the garden's form. Throws a `RangeError` for a value no reader takes.
 */
export function gardenPayload(garden: Garden): Uint8Array {
	if (garden.things.length > MOST_THINGS) {
		throw new RangeError(`at most ${MOST_THINGS} things`);
	}
	if (garden.events.length > MOST_EVENTS) {
		throw new RangeError(`at most ${MOST_EVENTS} events`);
	}
	checkTime("at", garden.at);
	return encoder.encode(
		`{"things":[${garden.things.map(thingJson).join(",")}],"events":[${garden.events.map(eventJson).join(",")}],"at":${garden.at}}`,
	);
}

function nullOrJson(what: string, text: string | null, most: number): string {
	if (text === null) return "null";
	if (!fits(text, most)) {
		throw new RangeError(`${what} is null or 1 to ${most} bytes of UTF-8`);
	}
	return jsonString(text);
}

/**
 * The payload of a state on `state`, its members in the order of the
 * table; the pet writes it, and the tests write it back to see that it is
 * the pet's form. Throws a `RangeError` for a value no reader takes.
 */
export function statePayload(state: State): Uint8Array {
	checkTime("began", state.began);
	checkTime("arrives", state.arrives);
	checkTime("at", state.at);
	if (state.arrives < state.began) {
		throw new RangeError("arrives is not before began");
	}
	if (
		!Number.isSafeInteger(state.mood) ||
		state.mood < 0 ||
		state.mood > MOOD_MOST
	) {
		throw new RangeError(
			`mood is a whole number from 0 to ${MOOD_MOST}, not ${state.mood}`,
		);
	}
	return encoder.encode(
		`{"from":${placeJson("from", state.from)},"to":${placeJson("to", state.to)},"began":${state.began},"arrives":${state.arrives},"facing":${JSON.stringify(state.facing)},"doing":${JSON.stringify(state.doing)},"mood":${state.mood},"bubble":${nullOrJson("bubble", state.bubble, BUBBLE_BYTES)},"by":${nullOrJson("by", state.by, Number.POSITIVE_INFINITY)},"at":${state.at}}`,
	);
}
