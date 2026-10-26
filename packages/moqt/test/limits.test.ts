import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	MAX_MESSAGE_PAYLOAD_LENGTH,
	MAX_NEW_SESSION_URI_LENGTH,
	MAX_REASON_PHRASE_LENGTH,
} from "../src/core/control-message.ts";
import {
	EARLY_CLOSED_ALIAS_LIMIT,
	HELD_OBJECT_OVERHEAD,
	HELD_PROPERTY_OVERHEAD,
	RETIRED_ALIAS_LIMIT,
} from "../src/core/incoming-subgroups.ts";
import {
	MAX_KEY_VALUE_PAIRS,
	MAX_VALUE_LENGTH,
} from "../src/core/key-value-pair.ts";
import { DEFAULT_ANSWER_WAIT_MS, resolveLimits } from "../src/core/limits.ts";
import { REQUEST_IDS_AHEAD_LIMIT } from "../src/core/peer-requests.ts";
import {
	MAX_HELD_FETCH_UPDATES,
	MAX_NAMESPACES_OF_A_SUBSCRIPTION,
	MAX_UNANSWERED_UPDATES,
} from "../src/core/requests.ts";
import {
	MAX_FULL_TRACK_NAME_LENGTH,
	MAX_TRACK_NAMESPACE_FIELDS,
} from "../src/core/track.ts";
import { MAX_CLOSE_REASON_BYTES } from "../src/webtransport/close-info.ts";
import {
	DEFAULT_FIRST_BYTES_TIMEOUT_MS,
	DEFAULT_IDLE_SUBGROUP_STREAM_TIMEOUT_MS,
	DEFAULT_MAX_PEER_BIDI_STREAMS,
	DEFAULT_MAX_PEER_UNI_STREAMS,
	DEFAULT_SETUP_TIMEOUT_MS,
} from "../src/webtransport/connection.ts";

const defaults = resolveLimits();

/**
 * This session's value of each row of `fixtures/limits/limits.tsv`, by its
 * name; undefined where it has none (`-`). `limits.rs` of `xmoq-moqt-io`
 * has the Rust side's.
 */
const OURS: Record<string, number | undefined> = {
	"namespace-fields": MAX_TRACK_NAMESPACE_FIELDS,
	"full-track-name-bytes": MAX_FULL_TRACK_NAME_LENGTH,
	"reason-phrase-bytes": MAX_REASON_PHRASE_LENGTH,
	"new-session-uri-bytes": MAX_NEW_SESSION_URI_LENGTH,
	"control-message-payload-bytes": MAX_MESSAGE_PAYLOAD_LENGTH,
	"close-reason-bytes": MAX_CLOSE_REASON_BYTES,
	"key-value-pair-value-bytes": MAX_VALUE_LENGTH,
	"key-value-pairs": MAX_KEY_VALUE_PAIRS,
	"retired-track-aliases": RETIRED_ALIAS_LIMIT,
	"early-closed-track-aliases": EARLY_CLOSED_ALIAS_LIMIT,
	"request-ids-ahead": REQUEST_IDS_AHEAD_LIMIT,
	"unanswered-request-updates": MAX_UNANSWERED_UPDATES,
	"namespaces-of-a-subscription": MAX_NAMESPACES_OF_A_SUBSCRIPTION,
	"early-objects-bytes": defaults.maxHeldSubgroupBytes,
	"early-object-overhead-bytes": HELD_OBJECT_OVERHEAD,
	"early-property-overhead-bytes": HELD_PROPERTY_OVERHEAD,
	"answer-wait-ms": DEFAULT_ANSWER_WAIT_MS,
	"first-bytes-wait-ms": DEFAULT_FIRST_BYTES_TIMEOUT_MS,
	"idle-subgroup-stream-ms": DEFAULT_IDLE_SUBGROUP_STREAM_TIMEOUT_MS,
	"setup-wait-ms": DEFAULT_SETUP_TIMEOUT_MS,
	"peer-uni-streams": DEFAULT_MAX_PEER_UNI_STREAMS,
	"peer-request-streams": DEFAULT_MAX_PEER_BIDI_STREAMS,
	"peer-requests": defaults.maxPeerRequests,
	"subgroup-stream-bytes": undefined,
	"object-payload-bytes": defaults.maxObjectPayloadLength,
	"object-properties-bytes": defaults.maxObjectPropertiesLength,
	"session-subgroup-bytes": defaults.maxSessionSubgroupBytes,
	"peer-fetches": defaults.maxPeerFetches,
	"held-fetch-updates": MAX_HELD_FETCH_UPDATES,
	"ended-by-publish-done": undefined,
	"held-before-setup": undefined,
};

/** The rows of the table: every row ends with one line feed, as its README says. */
function rows(): string[][] {
	const path = join(
		import.meta.dir,
		"..",
		"..",
		"..",
		"fixtures",
		"limits",
		"limits.tsv",
	);
	const table = readFileSync(path, "utf8");
	if (!table.endsWith("\n"))
		throw new Error(`${path}: no line feed at its end`);
	const read = table
		.slice(0, -1)
		.split("\n")
		.map((row) => row.split("\t"));
	if (read.some((row) => row.length !== 3)) {
		throw new Error(`${path}: a row of other than three columns`);
	}
	return read;
}

const table = rows();

// WHY the count: a row that is added has a value on this side to be written
// here, or a `-` that is to be looked at; one that is gone, a value here
// that nothing compares.
test("the table has a row for each value here, and no other", () => {
	expect([table.length, table.map(([name]) => name).sort()]).toEqual([
		31,
		Object.keys(OURS).sort(),
	]);
});

test.each(table)("%s", (name, _rust, typescript) => {
	if (name === undefined || !(name in OURS)) {
		throw new Error(`no value here for the row ${name}`);
	}
	expect(OURS[name]).toEqual(
		typescript === "-" ? undefined : Number(typescript),
	);
});
