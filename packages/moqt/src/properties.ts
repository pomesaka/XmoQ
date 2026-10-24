// Entry "@xmoq/moqt/properties": what the properties of an Object or a
// Track hold, without I/O. WHY an entry of its own: a layer above that reads
// properties in a part without I/O (the core of @xmoq/media, checked without
// the types of the browser) cannot import the main entry, which has the
// connection.

export {
	type PropertiesToSearch,
	propertiesToSearch,
} from "./core/immutable-properties.ts";
export type { KeyValuePair } from "./core/key-value-pair.ts";
