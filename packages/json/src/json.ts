// A reader of JSON (RFC 8259) that keeps the text of each number, for the
// payloads of the tracks that Rust writes and reads with `serde_json`: the
// chat's (`@xmoq/chat`) and the transcripts' (`@xmoq/transcript`).
//
// WHY NOT `JSON.parse`: the same bytes are to give the same value, or the
// same reason why there is none, in every reader (`fixtures/chat-tracks/`,
// `fixtures/transcript-tracks/`). `JSON.parse` makes the same number of
// `1e3` and `1000`, of `1.0` and `1`, and of `-0` and `0`, where
// `serde_json` has a float for the first of each, which is no integer. It
// also takes what `serde_json` refuses: an escaped lone surrogate
// (`"\ud800"`), a number beyond a double (`1e400`, which it makes
// Infinity), and nesting of any depth. The rows of the tables that are
// about JSON's edges were checked against `serde_json` by the Rust tests,
// which read the same tables.
//
// WHY a package of its own, in the lowest layer: each package of the
// tracks reads its payloads with it, and packages of one layer may not
// depend on each other (`scripts/ci/check-layers.sh`).

/** A value as this reader keeps it: of a number its text. */
export type JsonValue =
	| { kind: "object"; members: Map<string, JsonValue> }
	| { kind: "array"; items: JsonValue[] }
	| { kind: "string"; value: string }
	| { kind: "number"; text: string }
	| { kind: "literal"; value: boolean | null };

/**
 * How deep arrays and objects may be nested: the outermost counts as 1.
 *
 * WHY 127: `serde_json` 1 counts down from 128 at each `[` and `{` and
 * fails where it reaches 0, so 127 levels are read and 128 are not
 * (the rows "a member more 127 deep" and "128 deep" of `said.tsv`).
 */
const MAX_DEPTH = 127;

const NUMBER = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/y;

const LITERALS: [string, boolean | null][] = [
	["true", true],
	["false", false],
	["null", null],
];

/** Why the text is no JSON; nothing outside this file sees it. */
class NotJson extends Error {}

/**
 * Reads one JSON value that is the whole of `text`, with whitespace around
 * it. Undefined for text that is no JSON, or that `serde_json` would not
 * read (above).
 *
 * A member that comes twice counts with the last of its values, as with
 * `serde_json` and `JSON.parse`.
 */
export function readJson(text: string): JsonValue | undefined {
	let at = 0;
	const space = (): void => {
		while (at < text.length) {
			const code = text.charCodeAt(at);
			// RFC 8259 §2: space, horizontal tab, line feed, carriage return.
			if (code !== 0x20 && code !== 0x09 && code !== 0x0a && code !== 0x0d) {
				return;
			}
			at += 1;
		}
	};
	const expect = (character: string): void => {
		if (text[at] !== character) throw new NotJson(`${character} at ${at}`);
		at += 1;
	};
	const hex4 = (): number => {
		const digits = text.slice(at, at + 4);
		if (!/^[0-9a-fA-F]{4}$/.test(digits)) throw new NotJson(`\\u at ${at}`);
		at += 4;
		return Number.parseInt(digits, 16);
	};
	const string = (): string => {
		expect('"');
		let value = "";
		for (;;) {
			const character = text[at];
			if (character === undefined) throw new NotJson("an open string");
			at += 1;
			if (character === '"') return value;
			if (character.charCodeAt(0) < 0x20) {
				throw new NotJson(`a control character at ${at - 1}`);
			}
			if (character !== "\\") {
				value += character;
				continue;
			}
			const escaped = text[at];
			at += 1;
			switch (escaped) {
				case '"':
				case "\\":
				case "/":
					value += escaped;
					break;
				case "b":
					value += "\b";
					break;
				case "f":
					value += "\f";
					break;
				case "n":
					value += "\n";
					break;
				case "r":
					value += "\r";
					break;
				case "t":
					value += "\t";
					break;
				case "u": {
					const unit = hex4();
					if (unit >= 0xdc00 && unit <= 0xdfff) {
						throw new NotJson("a lone trailing surrogate");
					}
					if (unit >= 0xd800 && unit <= 0xdbff) {
						// Only a second escape that is a trailing surrogate makes
						// a pair; anything else leaves this one alone.
						if (text.slice(at, at + 2) !== "\\u") {
							throw new NotJson("a lone leading surrogate");
						}
						at += 2;
						const trailing = hex4();
						if (trailing < 0xdc00 || trailing > 0xdfff) {
							throw new NotJson("a lone leading surrogate");
						}
						value += String.fromCharCode(unit, trailing);
						break;
					}
					value += String.fromCharCode(unit);
					break;
				}
				default:
					throw new NotJson(`an escape at ${at - 1}`);
			}
		}
	};
	const value = (depth: number): JsonValue => {
		space();
		const first = text[at];
		if (first === "{" || first === "[") {
			if (depth + 1 > MAX_DEPTH) throw new NotJson("nested too deep");
		}
		switch (first) {
			case "{": {
				at += 1;
				const members = new Map<string, JsonValue>();
				space();
				if (text[at] === "}") {
					at += 1;
					return { kind: "object", members };
				}
				for (;;) {
					space();
					const key = string();
					space();
					expect(":");
					members.set(key, value(depth + 1));
					space();
					if (text[at] === "}") {
						at += 1;
						return { kind: "object", members };
					}
					expect(",");
				}
			}
			case "[": {
				at += 1;
				const items: JsonValue[] = [];
				space();
				if (text[at] === "]") {
					at += 1;
					return { kind: "array", items };
				}
				for (;;) {
					items.push(value(depth + 1));
					space();
					if (text[at] === "]") {
						at += 1;
						return { kind: "array", items };
					}
					expect(",");
				}
			}
			case '"':
				return { kind: "string", value: string() };
			default: {
				for (const [literal, read] of LITERALS) {
					if (text.startsWith(literal, at)) {
						at += literal.length;
						return { kind: "literal", value: read };
					}
				}
				NUMBER.lastIndex = at;
				const number = NUMBER.exec(text)?.[0];
				if (number === undefined) throw new NotJson(`a value at ${at}`);
				// `serde_json` refuses a number a double cannot hold
				// ("number out of range"); `Number` makes Infinity of it.
				if (!Number.isFinite(Number(number))) {
					throw new NotJson("a number out of range");
				}
				at += number.length;
				return { kind: "number", text: number };
			}
		}
	};
	try {
		const read = value(0);
		space();
		return at === text.length ? read : undefined;
	} catch (error) {
		if (error instanceof NotJson) return undefined;
		throw error;
	}
}

// WHY `fatal` and `ignoreBOM`: `serde_json` reads bytes that are not UTF-8,
// and a byte order mark, as no JSON. Without `fatal` the decoder puts
// U+FFFD in place of the first, and without `ignoreBOM` it drops the second.
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

/** The one value of a payload of UTF-8 JSON, read as `readJson` reads text; undefined for bytes that are not UTF-8, or no JSON. */
export function readJsonPayload(payload: Uint8Array): JsonValue | undefined {
	let text: string;
	try {
		text = decoder.decode(payload);
	} catch {
		return undefined;
	}
	return readJson(text);
}

/** The members of `value` where it is an object. */
export const membersOf = (
	value: JsonValue | undefined,
): Map<string, JsonValue> | undefined =>
	value?.kind === "object" ? value.members : undefined;

/**
 * A number written as a whole number from 0 to `most`, with no fraction,
 * exponent or sign, as `as_u64` of `serde_json` takes it (it has a float for
 * `-0`, `1.0` and `1e3`); undefined for any other value. `most` is at most
 * `Number.MAX_SAFE_INTEGER`, which a number holds exactly.
 */
export function wholeNumberOf(
	value: JsonValue | undefined,
	most: number = Number.MAX_SAFE_INTEGER,
): number | undefined {
	if (value?.kind !== "number" || !/^(?:0|[1-9][0-9]*)$/.test(value.text)) {
		return undefined;
	}
	// WHY by the text's length first: a longer one would lose its last
	// digits in a number, and could come out at `most`.
	if (value.text.length > String(Number.MAX_SAFE_INTEGER).length) {
		return undefined;
	}
	const read = Number(value.text);
	return read <= most && read <= Number.MAX_SAFE_INTEGER ? read : undefined;
}

/**
 * `value` as `JSON.parse` would give it: a number by `Number` of its text,
 * an object as a plain object of its members (the last of a member that
 * came twice). For a reader that checks a document by the types of its
 * fields, as `@xmoq/msf` does its catalogs.
 */
export function plainOf(value: JsonValue): unknown {
	switch (value.kind) {
		case "object": {
			const plain: Record<string, unknown> = {};
			for (const [key, member] of value.members) {
				// WHY defineProperty: a member named `__proto__` is a member,
				// as `JSON.parse` makes it, and not the prototype.
				Object.defineProperty(plain, key, {
					value: plainOf(member),
					enumerable: true,
					writable: true,
					configurable: true,
				});
			}
			return plain;
		}
		case "array":
			return value.items.map(plainOf);
		case "string":
			return value.value;
		case "number":
			return Number(value.text);
		case "literal":
			return value.value;
	}
}
