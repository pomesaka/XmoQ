import { expect, test } from "bun:test";
import { protocolRefusal } from "../../src/webtransport/connect.ts";

// §3.1: only a session that negotiated "moqt-18" speaks draft 18.
test.each([
	{ protocol: "moqt-18", refusal: undefined },
	{
		protocol: "moqt-17",
		refusal: "the server selected protocol moqt-17, not moqt-18",
	},
	{
		// A browser without `WebTransport.protocol`, or a server that chose none.
		protocol: undefined,
		refusal: "the server selected protocol undefined, not moqt-18",
	},
	{ protocol: "", refusal: "the server selected protocol , not moqt-18" },
])("protocolRefusal($protocol)", ({ protocol, refusal }) => {
	expect(protocolRefusal(protocol)).toBe(refusal);
});
