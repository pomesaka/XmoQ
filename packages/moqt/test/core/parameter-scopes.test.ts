import { expect, test } from "bun:test";
import {
	type ParameterScope,
	parameterAllowedIn,
} from "../../src/core/parameters.ts";
import { fixtureTable } from "../helpers.ts";

/** The messages of the table's README, each a scope of `ParameterScope`. */
const MESSAGES: readonly ParameterScope[] = [
	"SUBSCRIBE",
	"SUBSCRIBE_OK",
	"PUBLISH",
	"PUBLISH_NAMESPACE",
	"FETCH",
	"FETCH_OK",
	"SUBSCRIBE_NAMESPACE",
	"TRACK_STATUS",
	"PUBLISH_OK",
	"PUBLISH_NAMESPACE_OK",
	"TRACK_STATUS_OK",
	"SUBSCRIBE_NAMESPACE_OK",
	"REQUEST_UPDATE_OK",
	"REQUEST_UPDATE",
	"REQUEST_UPDATE_OF_FETCH",
	"REQUEST_UPDATE_OF_SUBSCRIBE_NAMESPACE",
];

const ROWS = fixtureTable("parameter-scopes", "scopes.tsv");

test("the table has a row for each parameter", () => {
	expect(ROWS.length).toBe(13);
});

// The rows of `fixtures/parameter-scopes/scopes.tsv`, which the Rust
// implementation reads as well: of each parameter, every message of the
// list, where it may appear and where not (§10.2.1).
test.each(ROWS)("%s (%s)", (_name, type, allowed) => {
	const named = (allowed ?? "").split(",");
	const unknown = named.filter(
		(message) => !MESSAGES.some((each) => each === message),
	);
	expect(unknown).toEqual([]);
	const allows: string[] = MESSAGES.filter((message) =>
		parameterAllowedIn(BigInt(type ?? "no type"), message),
	);
	expect([...allows].sort()).toEqual([...named].sort());
});
