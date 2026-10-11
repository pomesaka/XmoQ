// draft-ietf-moq-loc-04, as far as this package implements it. The one
// place that says which document the files of this directory follow.
//
// WHY this revision, on MoQ Transport draft 18, a pair no document names
// (decided on 2026-10-11): none of its six property numbers is one draft 18
// defines, all are below 0x4000, and moqlivemock's publisher sends its
// Timestamp, 0x10, on draft 18 (bytes in fixtures/loc-04/).
// WHY NOT LOC-02, which MSF-01 cites: its Timestamp is 0x06, which draft 18
// has for SUBGROUP_DELIVERY_TIMEOUT. WHY NOT LOC-03, which cites draft 18:
// it has two numbers for the Timestamp, 0x0A in its text and 0x06 in its
// table. WHY NOT draft 19, which LOC-04 cites: the peers of the interop run
// speak draft 18. The README has the tables.

import type { LocRevision } from "../core/revision.ts";
import { locProperties, microsecondsOf, readLocTiming } from "./properties.ts";
import { beginsWithStartCode, toAnnexB } from "./video-payload.ts";

export const revision: LocRevision = {
	name: "draft-ietf-moq-loc-04",
	locProperties,
	readLocTiming,
	microsecondsOf,
	toAnnexB,
	beginsWithStartCode,
};
