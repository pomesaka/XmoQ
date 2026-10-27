// The lines a page of a meeting writes for an agent, one event a line of
// JSON, as `xmoq-agent` writes them (`crates/agent/src/events.rs`), in files
// of `events/` named as it names them (`crates/agent/src/event_files.rs`):
// an agent reads the folder of the page and the `--out` of the CLI in the
// same way. Both are tested with `fixtures/agent-lines/lines.tsv` and
// `event-files.tsv`.

/** One thing that happened, as the page writes it. */
export type AgentEvent =
	/**
	 * The page began to write the folder: its first line. `room` is the
	 * chat's room (`meeting/<room>`), `id` the page's id in the meeting,
	 * which the `from` of its own voice ends with. `xmoq-agent` writes none.
	 */
	| { type: "reader"; state: "began"; room: string; id?: string }
	/** The page stopped writing the folder: its last line. None comes where the page is closed. */
	| { type: "reader"; state: "ended" }
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
			/**
			 * Said by the agent, through the page: written as `"by":"agent"`,
			 * and left out otherwise. WHY: the agent says its lines under the
			 * person's name, and `from` cannot tell them from the person's.
			 */
			byAgent?: boolean;
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
		case "reader":
			return event.state === "began"
				? {
						type: event.type,
						state: event.state,
						room: event.room,
						...(event.id === undefined ? {} : { id: event.id }),
					}
				: { type: event.type, state: event.state };
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
				...(event.byAgent === true ? { by: "agent" } : {}),
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

/** The directory of the files of events, in the folder. */
export const EVENTS_DIR = "events";

/**
 * The digits of the name of a file of `events/`, at the least: the Unix
 * time in milliseconds has 13 until the year 2286, so that the names sort
 * as their numbers.
 */
const EVENT_FILE_DIGITS = 13;

/** The number of a file of `events/` by its name, `<13 or more digits>.jsonl`; undefined for any other name. */
export function eventFileNumber(name: string): number | undefined {
	const match = /^(\d+)\.jsonl$/.exec(name);
	const digits = match?.[1];
	if (digits === undefined || digits.length < EVENT_FILE_DIGITS)
		return undefined;
	return Number(digits);
}

/**
 * The number of the next file of `events/`: the clock `now` (ms since the
 * Unix epoch), or one after `newest`, the number of the newest file there,
 * where that is not before it. WHY not the clock alone: two writes in the
 * same millisecond, a clock that went back, or a page that writes again in
 * the same folder would put a file before one a reader already read, which
 * a reader that reads the names after the last it read would never read.
 */
export function nextEventFile(now: number, newest: number | undefined): number {
	return newest === undefined ? now : Math.max(now, newest + 1);
}

/** The name of the file of `number` in `events/`. */
export function eventFileName(number: number): string {
	return `${String(number).padStart(EVENT_FILE_DIGITS, "0")}.jsonl`;
}
