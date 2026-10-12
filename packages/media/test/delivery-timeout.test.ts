import { expect, test } from "bun:test";
import {
	type DeliveryTimeouts,
	deliveryTimeoutParameters,
} from "../src/index.ts";

test.each<{
	timeouts: DeliveryTimeouts | undefined;
	parameters: {
		subgroupDeliveryTimeout?: bigint;
		objectDeliveryTimeout?: bigint;
	};
}>([
	{ timeouts: undefined, parameters: {} },
	{ timeouts: {}, parameters: {} },
	{
		timeouts: { subgroupMs: 500 },
		parameters: { subgroupDeliveryTimeout: 500n },
	},
	{ timeouts: { objectMs: 250 }, parameters: { objectDeliveryTimeout: 250n } },
	{
		timeouts: { subgroupMs: 0, objectMs: 1000 },
		parameters: { subgroupDeliveryTimeout: 0n, objectDeliveryTimeout: 1000n },
	},
])("the parameters of $timeouts", ({ timeouts, parameters }) => {
	expect(deliveryTimeoutParameters(timeouts)).toEqual(parameters);
});

test.each<{ timeouts: DeliveryTimeouts }>([
	{ timeouts: { subgroupMs: -1 } },
	{ timeouts: { subgroupMs: 1.5 } },
	{ timeouts: { objectMs: Number.NaN } },
	{ timeouts: { objectMs: 2 ** 53 } },
])("a time that is none is refused: $timeouts", ({ timeouts }) => {
	expect(() => deliveryTimeoutParameters(timeouts)).toThrow(RangeError);
});
