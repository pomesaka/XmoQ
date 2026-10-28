// The namespaces and tracks of a room's garden, as `fixtures/pet-tracks/names.tsv`
// pins them for `xmoq-pet-tracks` and this package: the garden publishes the
// track `garden` of `xmoq/pet/<room>`; an actor announces
// `xmoq/pet/<room>/<name>` and publishes the track `acted` there; the actor
// named `pet` also publishes `state`, its body.

/** The fields every namespace of the garden begins with. */
export const PREFIX = ["xmoq", "pet"] as const;

/** The name of the actor that is the pet. */
export const PET_NAME = "pet";

/** The track of a room: the garden as it is now, a garden a Group. */
export const GARDEN = "garden";

/** The track of an actor: what it does to the garden, an action a Group. */
export const ACTED = "acted";

/** The track of the pet alone: where it is and what it does, a state a Group. */
export const STATE = "state";

/** A track of the garden, as a namespace and a track's name are read. */
export type GardenTrack =
	| { track: typeof GARDEN; room: string }
	| { track: typeof ACTED; room: string; name: string }
	| { track: typeof STATE; room: string };

const encoder = new TextEncoder();
// WHY `fatal`: a field that is not UTF-8 names no room and no actor, where
// a decoder that is not fatal would read it as another name.
const decoder = new TextDecoder("utf-8", { fatal: true });

/** `xmoq/pet/<room>`: the namespace of the track `garden` of a room. */
export function roomNamespace(room: string): Uint8Array[] {
	return [...PREFIX, room].map((field) => encoder.encode(field));
}

/** `xmoq/pet/<room>/<name>`: what an actor announces, as the fields of a Track Namespace; the pet's with `PET_NAME`. */
export function actorNamespace(room: string, name: string): Uint8Array[] {
	return [...PREFIX, room, name].map((field) => encoder.encode(field));
}

/** The fields after `xmoq/pet`, as text; undefined for another prefix, a field not UTF-8 or an empty one. */
function fieldsAfterPrefix(namespace: Uint8Array[]): string[] | undefined {
	const fields: string[] = [];
	for (const field of namespace) {
		let text: string;
		try {
			text = decoder.decode(field);
		} catch {
			return undefined;
		}
		if (text === "") return undefined;
		fields.push(text);
	}
	const [first, second, ...rest] = fields;
	return first === PREFIX[0] && second === PREFIX[1] ? rest : undefined;
}

/** The actor whose namespace `namespace` is, the pet among them; undefined where it is nobody's. */
export function actorOf(
	namespace: Uint8Array[],
): { room: string; name: string } | undefined {
	const [room, name, ...rest] = fieldsAfterPrefix(namespace) ?? [];
	if (room === undefined || name === undefined || rest.length > 0) {
		return undefined;
	}
	return { room, name };
}

/** The track of the garden that `trackName` of `namespace` is; undefined where it is none. */
export function trackOf(
	namespace: Uint8Array[],
	trackName: Uint8Array,
): GardenTrack | undefined {
	let track: string;
	try {
		track = decoder.decode(trackName);
	} catch {
		return undefined;
	}
	const [room, name, ...rest] = fieldsAfterPrefix(namespace) ?? [];
	if (room === undefined || rest.length > 0) return undefined;
	if (name === undefined) {
		return track === GARDEN ? { track, room } : undefined;
	}
	if (track === ACTED) return { track, room, name };
	if (track === STATE && name === PET_NAME) return { track, room };
	return undefined;
}
