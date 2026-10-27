import { expect, test } from "bun:test";
import { readWhole } from "../src/web/agent-folder.ts";

test.each<{
	name: string;
	/** What each `getFile` gives, in turn: the text of the file, or the name of the DOMException it fails with. */
	reads: ({ text: string } | { fails: string })[];
	read: string | { fails: string };
}>([
	{
		name: "a file is read as it is",
		reads: [{ text: "a" }],
		read: "a",
	},
	{
		name: "a file written anew while it was read is read again",
		reads: [
			{ fails: "NotFoundError" },
			{ fails: "NotFoundError" },
			{ text: "b" },
		],
		read: "b",
	},
	{
		name: "a file that is gone after the third read fails with that",
		reads: [
			{ fails: "NotFoundError" },
			{ fails: "NotFoundError" },
			{ fails: "NotFoundError" },
			{ text: "never read" },
		],
		read: { fails: "NotFoundError" },
	},
	{
		name: "another failure is not read again",
		reads: [{ fails: "NotAllowedError" }, { text: "never read" }],
		read: { fails: "NotAllowedError" },
	},
])("$name", async ({ reads, read }) => {
	let at = 0;
	const file = {
		async getFile(): Promise<File> {
			const next = reads[at];
			at += 1;
			if (next === undefined) throw new Error("read more than the test has");
			if ("fails" in next) throw new DOMException("in the test", next.fails);
			return new File([next.text], "inbox.txt");
		},
	};
	const outcome = await readWhole(file).then(
		(bytes) => new TextDecoder().decode(bytes),
		(error: unknown) => ({
			fails: error instanceof DOMException ? error.name : String(error),
		}),
	);
	expect(outcome).toEqual(read);
});
