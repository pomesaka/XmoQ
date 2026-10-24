import { createByteReader, ShortBufferError } from "./bytes.ts";
import {
	decodeKeyValuePairs,
	isMandatoryTrackProperty,
	type KeyValuePair,
} from "./key-value-pair.ts";

/** Property Type of Immutable Properties (§12.7), a Track or Object Property. */
export const IMMUTABLE_PROPERTIES = 0x0bn;

/**
 * The properties §12.7 has a processor search, or why there are none: "When
 * looking for the value of a property, processors MUST search both the
 * mutable properties and the contents of Immutable Properties."
 */
export type PropertiesToSearch =
	/**
	 * The mutable properties without Immutable Properties, in the order they
	 * came, followed by the Key-Value-Pairs inside each Immutable Properties.
	 */
	| { properties: KeyValuePair[] }
	/** The Track is malformed (§2.4.2, §12.7). */
	| { malformed: string };

/**
 * Reads the Immutable Properties among the properties of an Object or a
 * Track (§12.7) and returns what is searched for the value of a property.
 *
 * - An Object with more than one Immutable Properties: "An Object MUST NOT
 *   contain more than one instance of this property". The text gives no
 *   consequence; malformed, since §2.4.2 has a Track that "does not conform
 *   to MOQT constraints" malformed. A Track may have more than one: nothing
 *   in §12.7 forbids it, and the contents of each are searched.
 * - Immutable Properties inside Immutable Properties: malformed (§12.7, "An
 *   Object contains an Immutable Properties property that contains another
 *   Immutable Properties key"). The text names an Object. **The same for a
 *   Track**: the draft is silent there, and a second level would be one
 *   that §12.7 does not have searched.
 * - A Key-Value-Pair inside that runs past the end of the value: malformed
 *   (§12.7, "A Key-Value-Pair cannot be parsed").
 *
 * The inner pairs are a sequence of their own (Figure 2): the first Delta
 * Type counts from 0, not from 0xB. `moqtail` 0.15.0 reads it so too
 * (`src/model/property/object_property.rs`,
 * `test_immutable_properties_independent_of_outer_prev_type`, read on
 * 2026-10-24).
 *
 * Throws what decoding a sequence throws, which this does not turn into a
 * malformed Track: the `MoqtError` of a PROTOCOL_VIOLATION of §1.4.3 (a Type
 * over 2^64 - 1, a Length over 2^16 - 1), and `TooManyEntriesError` past the
 * local limit of one sequence, which each Immutable Properties has for
 * itself. WHY NOT malformed for the first: §1.4.3 says "MUST close the
 * session" of every Key-Value-Pair, and §12.7 makes no exception of the
 * inner ones.
 *
 * `moqtail` 0.15.0 takes Immutable Properties inside Immutable Properties
 * for a PROTOCOL_VIOLATION; §12.7 has the Track malformed, which this
 * follows.
 */
export function propertiesToSearch(
	properties: readonly KeyValuePair[],
	of: "Object" | "Track",
): PropertiesToSearch {
	const mutable: KeyValuePair[] = [];
	const values: Uint8Array[] = [];
	for (const property of properties) {
		// An odd Type always carries bytes (§1.4.3); the check is for the type.
		if (property.type === IMMUTABLE_PROPERTIES) {
			if (typeof property.value !== "bigint") values.push(property.value);
		} else {
			mutable.push(property);
		}
	}
	if (of === "Object" && values.length > 1) {
		return { malformed: "more than one Immutable Properties on an Object" };
	}
	const searched = mutable;
	for (const value of values) {
		let inner: KeyValuePair[];
		try {
			inner = decodeKeyValuePairs(createByteReader(value));
		} catch (error) {
			if (error instanceof ShortBufferError) {
				return {
					malformed:
						"a Key-Value-Pair inside Immutable Properties cannot be parsed",
				};
			}
			throw error;
		}
		if (inner.some(({ type }) => type === IMMUTABLE_PROPERTIES)) {
			return {
				malformed: "Immutable Properties inside Immutable Properties",
			};
		}
		searched.push(...inner);
	}
	return { properties: searched };
}

/**
 * Why the Track Properties of a SUBSCRIBE_OK or FETCH_OK keep this endpoint
 * from processing the track, if they do: Immutable Properties that make it
 * malformed (§12.7), or a Mandatory Track Property, outside or inside them
 * (§2.5.1). None is implemented, so each one is one this endpoint "does not
 * understand". Malformed is decided first: what is inside is not known then.
 */
export function trackPropertiesRefusal(
	trackProperties: readonly KeyValuePair[],
): "malformed-track" | "unsupported-track-property" | undefined {
	const reading = propertiesToSearch(trackProperties, "Track");
	if ("malformed" in reading) return "malformed-track";
	if (reading.properties.some(({ type }) => isMandatoryTrackProperty(type))) {
		return "unsupported-track-property";
	}
	return undefined;
}
