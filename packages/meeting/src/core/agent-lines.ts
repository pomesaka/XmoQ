// The lines a page of a meeting writes for an agent, one event a line of
// JSON, as `xmoq-agent` writes them (`crates/agent/src/events.rs`): an agent
// reads the folder of the page and the `--out` of the CLI in the same way.
// Both are tested with `fixtures/agent-lines/lines.tsv`.

/** One thing that happened, as the page writes it. */
export type AgentEvent =
	/** A participant of the meeting came or left. */
	| { type: "participant"; state: "joined" | "left"; id: string }
	/** A line of the room's chat. */
	| {
			type: "chat";
			number: bigint;
			from: string;
			text: string;
			/** The sender's clock, ms. */
			sentAt: number | bigint;
			/** The gatherer's clock, ms. */
			gatheredAt: number | bigint;
	  }
	/** Something said, as the transcriber read it. */
	| {
			type: "speech";
			number: bigint;
			/** The namespace of the broadcast whose voice it was. */
			from: readonly string[];
			text: string;
			startUs: number | bigint;
			endUs: number | bigint;
			/** The transcriber's clock, ms. */
			readAt: number | bigint;
	  }
	/** A picture of a participant, written as a PNG at `path`, from the folder. */
	| {
			type: "frame";
			path: string;
			group: bigint;
			object: bigint;
			/** The Timestamp of LOC of the frame, µs. */
			timestampUs: bigint;
			width: number;
			height: number;
			/** Why it was taken. The page takes one when asked alone; `every` is the CLI's `--frame-every-ms`, kept so that the two have one shape. */
			asked: "command" | "every";
			/** Whose picture it is. */
			id: string;
	  }
	/** Something went wrong, or a line of the inbox was not said. */
	| { type: "error"; message: string };

/** A value of a line: a string, a whole number, or a list of strings. */
type Field = string | number | bigint | readonly string[];

/**
 * A value as `serde_json` writes it. WHY `JSON.stringify` for a string: it
 * escapes what serde_json escapes (a quote, a backslash, and the controls
 * below U+0020, as `\n`, `\t` and the like or `\u001b` in lower case) and
 * leaves every other character as it is, as serde_json does; the rows of
 * `fixtures/agent-lines/` pin it.
 */
function written(value: Field): string {
	if (typeof value === "string") return JSON.stringify(value);
	if (typeof value === "bigint") return value.toString();
	if (typeof value === "number") {
		// WHY a throw: serde_json writes whole numbers, and `toString` would
		// write `NaN` or `1.5`, which is no line of the CLI.
		if (!Number.isSafeInteger(value)) {
			throw new RangeError(`a line has whole numbers, not ${value}`);
		}
		return value.toString();
	}
	return `[${value.map((item) => JSON.stringify(item)).join(",")}]`;
}

/** The fields of `event`, by the names the line has. */
function fieldsOf(event: AgentEvent): Record<string, Field> {
	switch (event.type) {
		case "participant":
			return { type: event.type, state: event.state, id: event.id };
		case "chat":
			return {
				type: event.type,
				number: event.number,
				from: event.from,
				text: event.text,
				sent_at: event.sentAt,
				gathered_at: event.gatheredAt,
			};
		case "speech":
			return {
				type: event.type,
				number: event.number,
				from: event.from,
				text: event.text,
				start_us: event.startUs,
				end_us: event.endUs,
				read_at: event.readAt,
			};
		case "frame":
			return {
				type: event.type,
				path: event.path,
				group: event.group,
				object: event.object,
				timestamp_us: event.timestampUs,
				width: event.width,
				height: event.height,
				asked: event.asked,
				id: event.id,
			};
		case "error":
			return { type: event.type, message: event.message };
	}
}

/**
 * The event as one line of JSON, without its line feed, written at `at`
 * (ms since the Unix epoch). The keys are in the order of their names, as
 * `serde_json` writes an object.
 */
export function agentLine(event: AgentEvent, at: number): string {
	const fields: Record<string, Field> = { ...fieldsOf(event), at };
	const names = Object.keys(fields).sort();
	return `{${names
		.map((name) => {
			const value = fields[name];
			return value === undefined
				? ""
				: `${JSON.stringify(name)}:${written(value)}`;
		})
		.join(",")}}`;
}

/**
 * Where the picture of a `frame` line is, from the folder: as `xmoq-agent`
 * names them, `frames/<when the writing began, ms>/<six digits>-g<group>-o<object>.png`.
 */
export function framePath(
	began: number,
	taken: number,
	group: bigint,
	object: bigint,
): string {
	return `frames/${began}/${String(taken).padStart(6, "0")}-g${group}-o${object}.png`;
}
