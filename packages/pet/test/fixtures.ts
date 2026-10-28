import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * The rows of a table of `fixtures/pet-tracks/`, which `xmoq-pet-tracks`
 * reads too. WHY not filter empty lines away: every row ends with one line
 * feed; an empty row is an error of the table.
 */
export function petRows(file: string, columns: number): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"pet-tracks",
		file,
	);
	const table = readFileSync(path, "utf8");
	if (!table.endsWith("\n")) {
		throw new Error(`${path}: no line feed at its end`);
	}
	const rows = table
		.slice(0, -1)
		.split("\n")
		.map((row) => row.split("\t"));
	for (const row of rows) {
		if (row.length !== columns) {
			throw new Error(
				`${path}: a row of other than ${columns} columns: ${row}`,
			);
		}
	}
	return rows;
}
