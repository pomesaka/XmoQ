import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { answerWaitMs, DEFAULT_ANSWER_WAIT_MS } from "../../src/core/limits.ts";
import type { MessageParameters } from "../../src/core/parameters.ts";

/** `fixtures/request-answer-wait/waits.tsv`, which the Rust implementation reads too. */
const rows = readFileSync(
	join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"..",
		"fixtures",
		"request-answer-wait",
		"waits.tsv",
	),
	"utf8",
)
	.split("\n")
	.filter((row) => row !== "")
	.map((row) => row.split("\t"));

function parametersOf(name: string, value: string): MessageParameters {
	switch (name) {
		case "-":
			return {};
		case "RENDEZVOUS_TIMEOUT":
			return { rendezvousTimeout: BigInt(value) };
		case "FILL_TIMEOUT":
			return { fillTimeout: BigInt(value) };
		case "SUBGROUP_DELIVERY_TIMEOUT":
			return { subgroupDeliveryTimeout: BigInt(value) };
		default:
			throw new Error(`a parameter this test knows: ${name}`);
	}
}

describe("answerWaitMs", () => {
	test("the table has the rows both implementations count on", () => {
		expect(rows.length).toBe(10);
		for (const row of rows) expect(row.length).toBe(5);
	});

	test.each(rows)(
		"%s",
		(_name, _request, parameter = "", value = "", wait = "") => {
			expect(
				answerWaitMs(DEFAULT_ANSWER_WAIT_MS, parametersOf(parameter, value)),
			).toBe(Number(wait));
		},
	);

	test.each<{
		baseMs: number;
		parameters: MessageParameters | undefined;
		wait: number;
	}>([
		{ baseMs: 1500, parameters: undefined, wait: 1500 },
		{ baseMs: 1500, parameters: { fillTimeout: 500n }, wait: 2000 },
		{
			baseMs: 0,
			parameters: { rendezvousTimeout: 18_446_744_073_709_551_615n },
			wait: 2_147_483_647,
		},
	])(
		"the wait begins from what it is given: $baseMs",
		({ baseMs, parameters, wait }) => {
			expect(answerWaitMs(baseMs, parameters)).toBe(wait);
		},
	);
});
