import { expect, test } from "bun:test";
import { keyOfSetting, type Role, tokenText } from "../src/token.ts";

/** `fixtures/room-token/tokens.tsv`, which `crates/room-token` reads as well. */
const rows = (
	await Bun.file(
		new URL("../../../fixtures/room-token/tokens.tsv", import.meta.url),
	).text()
)
	.split("\n")
	.filter((line) => line !== "")
	.map((line) => line.split("\t"));

test("the table has its three rows of six columns", () => {
	expect(rows.map((columns) => columns.length)).toEqual([6, 6, 6]);
});

test.each(rows)("%s", async (_name, keyHex, namespace, role, expiry, token) => {
	const key = keyOfSetting(keyHex);
	if (role !== "pub" && role !== "sub") throw new Error(`no role ${role}`);
	const asked: Role = role;
	if (key === undefined || namespace === undefined)
		throw new Error("a row without a key or a namespace");
	expect(
		await tokenText(key, namespace.split("/"), asked, Number(expiry)),
	).toBe(token ?? "");
});

test.each<{
	name: string;
	setting: string | undefined;
	key: number[] | undefined;
}>([
	{ name: "two digits a byte", setting: "00ff", key: [0x00, 0xff] },
	{ name: "upper case as well", setting: "AB", key: [0xab] },
	{ name: "none where it is not set", setting: undefined, key: undefined },
])("the key of AUTH_KEY_HEX: $name", ({ setting, key }) => {
	const bytes = keyOfSetting(setting);
	expect(bytes === undefined ? undefined : [...bytes]).toEqual(key);
});

test.each<{ name: string; setting: string }>([
	{ name: "one that is set and empty", setting: "" },
	{ name: "an odd count", setting: "abc" },
	{ name: "what is not hex", setting: "zz" },
])("AUTH_KEY_HEX is an error: $name", ({ setting }) => {
	expect(() => keyOfSetting(setting)).toThrow(RangeError);
});
