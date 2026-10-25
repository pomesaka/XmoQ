# The lines an agent reads and writes

The lines of `events.jsonl` that a reader writes for an agent, and how a line the agent writes back is read: [`xmoq-agent`](../../crates/agent/README.md) (`xmoq_agent::events::Event`), and the meeting's page through a folder (`agentLine` of `@xmoq/meeting`, `packages/meeting/src/core/agent-lines.ts`). An agent reads the two in the same way, so the two write the same line for the same event.

| File | Read by |
|---|---|
| `lines.tsv` | `crates/agent` (`src/events.rs`) and `@xmoq/meeting` (`test/agent-lines.test.ts`) |
| `inbox.tsv` | `crates/agent` (`src/input.rs`) and `@xmoq/meeting` (`test/agent-inbox.test.ts`) |

## `lines.tsv`

One line a row, two columns separated by a tab, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique. Each reader's test has its own event for each name |
| 2 | The line, as it is written: one object of JSON, its keys in the order of their names, `at` the writer's clock in milliseconds since the Unix epoch |

The rows pin what is not written down elsewhere: the order of the keys, how a string is escaped (what a terminal would act on, and a line break, as `\u001b` and `\n`; other characters as they are), numbers without a fraction or an exponent, `id` on a `frame` of a meeting, and the `path` of a `frame` from the folder the line is in (`frames/<when the writing began, ms>/<six digits>-g<group>-o<object>.png`).

## `inbox.tsv`

What a line an agent writes asks for: a line of stdin of `xmoq-agent`, and a line of `inbox.txt` of a page's folder. Three columns separated by tabs, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | The line, without its line feed, as a string of JSON (so that a carriage return and a tab can be in it) |
| 3 | What it asks for: `say <text>`, `frame <id>` (`frame -` for none named), `unknown <name>`, or `nothing`. What follows the first space is as it is, spaces and all |
