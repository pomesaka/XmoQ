// The properties of an Object as LOC reads them: those outside Immutable
// Properties and those inside it.

import type { KeyValuePair } from "@xmoq/moqt";
import { propertiesToSearch } from "@xmoq/moqt/properties";

/**
 * The properties to look for LOC's Timestamp and Timescale in (LOC §2.2):
 * those outside Immutable Properties, then those inside it. Transport §12.7:
 * "When looking for the value of a property, processors MUST search both
 * the mutable properties and the contents of Immutable Properties."
 *
 * WHY the inside is opened here and not in `@xmoq/loc`: the two packages
 * are both of layer 0 and cannot depend on each other, how a Key-Value-Pair
 * is read is `@xmoq/moqt`'s alone, and `src/core` of this package does not
 * use MoQ Transport, so what it is handed is opened before. A Timestamp
 * both outside and inside is then two Timestamps, which `readLocTiming`
 * takes for invalid: §12.7 lets a property be in both only where it
 * "allows multiple values", which LOC does not say of the Timestamp.
 *
 * WHY the properties as they came where the inside cannot be opened: an
 * Object with such Immutable Properties ends its Track as malformed in the
 * session of `@xmoq/moqt` (§12.7), and never reaches here from a
 * subscription or a FETCH of it; one made by hand is read as it is.
 */
export function locProperties(
	properties: readonly KeyValuePair[],
): readonly KeyValuePair[] {
	try {
		const searched = propertiesToSearch(properties, "Object");
		return "properties" in searched ? searched.properties : properties;
	} catch {
		return properties;
	}
}
