import { expect, test } from "bun:test";
import {
	actorNamespace,
	actorOf,
	PET_NAME,
	roomNamespace,
	trackOf,
} from "../src/core/names.ts";
import { petRows } from "./fixtures.ts";

const table = petRows("names.tsv", 4);
const encoder = new TextEncoder();
const decoder = new TextDecoder();
const textOf = (namespace: Uint8Array[]): string[] =>
	namespace.map((field) => decoder.decode(field));

test("the table has its rows", () => {
	expect(table.length).toBe(22);
});

// Each row is read both ways: what the fields are read as, and, where they
// are somebody's, that this side makes the same fields.
test.each(table)("%s", (_case, fields, track, read) => {
	const parsed: unknown = JSON.parse(fields ?? "");
	if (!Array.isArray(parsed)) throw new Error(`no fields ${fields}`);
	const texts = parsed.map(String);
	const namespace = texts.map((field) => encoder.encode(field));
	const words = (read ?? "").split(" ");
	if (track === "-") {
		const actor = actorOf(namespace);
		expect(
			actor === undefined ? "none" : `actor ${actor.room} ${actor.name}`,
		).toBe(read ?? "");
		if (actor !== undefined) {
			expect(textOf(actorNamespace(actor.room, actor.name))).toEqual(texts);
		}
		return;
	}
	const of = trackOf(namespace, encoder.encode(track ?? ""));
	switch (of?.track) {
		case undefined:
			expect(words).toEqual(["none"]);
			break;
		case "garden":
			expect(words).toEqual(["garden", of.room]);
			expect(textOf(roomNamespace(of.room))).toEqual(texts);
			break;
		case "acted":
			expect(words).toEqual(["acted", of.room, of.name]);
			expect(textOf(actorNamespace(of.room, of.name))).toEqual(texts);
			break;
		case "state":
			expect(words).toEqual(["state", of.room]);
			expect(textOf(actorNamespace(of.room, PET_NAME))).toEqual(texts);
			break;
	}
});

// What the table, being text, cannot hold.
test("a field not UTF-8 is nobody's", () => {
	const namespace = [
		encoder.encode("xmoq"),
		encoder.encode("pet"),
		Uint8Array.of(0xff),
		encoder.encode("alice"),
	];
	expect(actorOf(namespace)).toBeUndefined();
});
