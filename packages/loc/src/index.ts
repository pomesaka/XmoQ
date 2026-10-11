// The revision of LOC this package is, for everything that uses it.
//
// **To follow another revision, this import is the one line to change**
// (README, "Following another revision"). Nothing outside this package
// names a revision.
import { revision } from "./draft-04/index.ts";

export type {
	AnnexBPayload,
	LocProperty,
	LocRevision,
	LocTiming,
	LocTimingRead,
} from "./core/revision.ts";

/** The document this package implements, with its revision. */
export const LOC_REVISION: string = revision.name;

export const {
	locProperties,
	readLocTiming,
	microsecondsOf,
	toAnnexB,
	beginsWithStartCode,
} = revision;
