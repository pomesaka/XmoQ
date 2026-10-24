import {
	type ByteReader,
	type ByteWriter,
	createByteWriter,
	MAX_VARINT,
} from "./bytes.ts";
import { protocolViolation } from "./error.ts";
import { MAX_KEY_VALUE_PAIRS, TooManyEntriesError } from "./key-value-pair.ts";
import {
	decodeTrackNamespace,
	encodeTrackNamespace,
	type TrackNamespace,
} from "./track.ts";
import { readLength, wireEnum } from "./wire.ts";

/** Location of §1.4.2. */
export interface Location {
	group: bigint;
	object: bigint;
}

/**
 * Whether a range that starts at `start` and has the End Location `end` ends
 * before it starts. An End Location names the last Object plus 1, and with
 * Object 0 the whole Group (§10.12.1).
 */
export function endsBefore(start: Location, end: Location): boolean {
	if (end.group !== start.group) return end.group < start.group;
	return end.object !== 0n && end.object < start.object;
}

/** Subscription Filter of §5.1.2. */
export type SubscriptionFilter =
	| { filterType: "NextGroupStart" }
	| { filterType: "LargestObject" }
	| { filterType: "AbsoluteStart"; startLocation: Location }
	| {
			filterType: "AbsoluteRange";
			startLocation: Location;
			endGroupDelta: bigint;
	  };

/** GROUP_ORDER values of §10.2.8. */
export type GroupOrder = "Ascending" | "Descending";

/**
 * Message Parameters of §10.2, one optional field per Parameter Type. An
 * absent field means the parameter is not in the message; the defaults of the
 * draft are not filled in.
 */
export interface MessageParameters {
	/** 0x02, milliseconds. */
	objectDeliveryTimeout?: bigint;
	/**
	 * 0x03, one entry per instance (§10.2.2 allows repeating it). Each entry is
	 * the Token structure as bytes: it is carried, not interpreted, because
	 * authorization is an open question of the project.
	 */
	authorizationToken?: Uint8Array[];
	/** 0x04, milliseconds. */
	rendezvousTimeout?: bigint;
	/** 0x06, milliseconds. */
	subgroupDeliveryTimeout?: bigint;
	/** 0x08, milliseconds. */
	expires?: bigint;
	/** 0x09 */
	largestObject?: Location;
	/**
	 * 0x0A, milliseconds a relay may spend on getting Objects of a FETCH from
	 * upstream; 0 for none at all (§10.2.5).
	 */
	fillTimeout?: bigint;
	/** 0x10 */
	forward?: boolean;
	/** 0x20, 0 to 255. */
	subscriberPriority?: number;
	/** 0x21 */
	subscriptionFilter?: SubscriptionFilter;
	/** 0x22 */
	groupOrder?: GroupOrder;
	/** 0x32 */
	newGroupRequest?: bigint;
	/** 0x34, the new prefix of a SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS. */
	trackNamespacePrefix?: TrackNamespace;
}

/**
 * The message a parameter block belongs to. PUBLISH_OK and
 * PUBLISH_NAMESPACE_OK are REQUEST_OK messages named by the request they
 * answer (§10.5). REQUEST_OK stands for a REQUEST_OK whose request is not
 * known: the codec sees the message without its stream, so it accepts what
 * any of the named kinds accepts and the session narrows it.
 */
export type ParameterScope =
	| "SUBSCRIBE"
	| "SUBSCRIBE_OK"
	| "PUBLISH"
	| "PUBLISH_OK"
	| "PUBLISH_NAMESPACE"
	| "PUBLISH_NAMESPACE_OK"
	/** SUBSCRIBE_NAMESPACE: AUTHORIZATION_TOKEN alone (§10.2.2). */
	| "SUBSCRIBE_NAMESPACE"
	| "REQUEST_OK"
	/** REQUEST_UPDATE for a subscription (§10.9.1). */
	| "REQUEST_UPDATE"
	/**
	 * REQUEST_UPDATE for a FETCH. The codec does not know what a
	 * REQUEST_UPDATE updates and decodes every one as that of a
	 * subscription; the session checks an update of a FETCH against this.
	 */
	| "REQUEST_UPDATE_OF_FETCH"
	/**
	 * REQUEST_UPDATE for a SUBSCRIBE_NAMESPACE, checked by the session as
	 * REQUEST_UPDATE_OF_FETCH is.
	 */
	| "REQUEST_UPDATE_OF_SUBSCRIBE_NAMESPACE"
	/**
	 * A REQUEST_UPDATE whose request is not known, as the codec sees it: what
	 * the update of any request accepts. The session narrows it.
	 */
	| "ANY_REQUEST_UPDATE"
	| "FETCH"
	/** No parameter of the draft lists FETCH_OK. */
	| "FETCH_OK";

const REQUEST_OK_KINDS: readonly ParameterScope[] = [
	"PUBLISH_OK",
	"PUBLISH_NAMESPACE_OK",
];

type WriteValue = (writer: ByteWriter) => void;

interface ParameterDefinition {
	/** Name in the draft, for error messages. */
	name: string;
	type: bigint;
	/**
	 * Messages the parameter MAY appear in, from its definition in §10.2.2 to
	 * §10.2.13, limited to the messages implemented here.
	 */
	scopes: readonly ParameterScope[];
	/** Whether §10.2 allows more than one instance in a message. */
	repeatable: boolean;
	/** Reads one value into `parameters`. */
	read(reader: ByteReader, parameters: MessageParameters): void;
	/** One writer per instance in `parameters`; empty when the parameter is absent. */
	writers(parameters: MessageParameters): WriteValue[];
}

/** A writer for `value`, or none when the parameter is absent. */
function present<T>(
	value: T | undefined,
	write: (writer: ByteWriter, value: T) => void,
): WriteValue[] {
	return value === undefined ? [] : [(writer) => write(writer, value)];
}

const writeVarint = (writer: ByteWriter, value: bigint): void => {
	writer.varint(value);
};

export function decodeLocation(reader: ByteReader): Location {
	const group = reader.varint();
	const object = reader.varint();
	return { group, object };
}

export function encodeLocation(writer: ByteWriter, location: Location): void {
	writer.varint(location.group).varint(location.object);
}

const FILTER_TYPES = wireEnum<SubscriptionFilter["filterType"]>({
	NextGroupStart: 0x1n,
	LargestObject: 0x2n,
	AbsoluteStart: 0x3n,
	AbsoluteRange: 0x4n,
});

function decodeSubscriptionFilter(reader: ByteReader): SubscriptionFilter {
	const value = reader.varint();
	const filterType = FILTER_TYPES.fromWire(value);
	switch (filterType) {
		case "NextGroupStart":
		case "LargestObject":
			return { filterType };
		case "AbsoluteStart":
			return { filterType, startLocation: decodeLocation(reader) };
		case "AbsoluteRange": {
			const startLocation = decodeLocation(reader);
			const endGroupDelta = reader.varint();
			if (startLocation.group + endGroupDelta > MAX_VARINT) {
				throw protocolViolation("End Group exceeds 2^64 - 1");
			}
			return { filterType, startLocation, endGroupDelta };
		}
		case undefined:
			throw protocolViolation(`unknown Filter Type ${value}`);
	}
}

function encodeSubscriptionFilter(
	writer: ByteWriter,
	filter: SubscriptionFilter,
): void {
	writer.varint(FILTER_TYPES.toWire(filter.filterType));
	switch (filter.filterType) {
		case "NextGroupStart":
		case "LargestObject":
			break;
		case "AbsoluteStart":
			encodeLocation(writer, filter.startLocation);
			break;
		case "AbsoluteRange":
			encodeLocation(writer, filter.startLocation);
			writer.varint(filter.endGroupDelta);
			break;
	}
}

const GROUP_ORDERS = wireEnum<GroupOrder>({
	Ascending: 0x1n,
	Descending: 0x2n,
});

function decodeGroupOrder(value: number): GroupOrder {
	const groupOrder = GROUP_ORDERS.fromWire(BigInt(value));
	if (groupOrder === undefined) {
		throw protocolViolation(
			`GROUP_ORDER value ${value} is neither 0x1 nor 0x2`,
		);
	}
	return groupOrder;
}

function decodeForward(value: number): boolean {
	if (value === 0) return false;
	if (value === 1) return true;
	throw protocolViolation(`FORWARD value ${value} is neither 0 nor 1`);
}

/**
 * Every Message Parameter implemented here, in ascending Type order, which is
 * the order §10.2 requires on the wire. Adding a parameter takes one entry
 * here and one field in `MessageParameters`.

 */
const PARAMETERS: readonly ParameterDefinition[] = [
	{
		name: "OBJECT_DELIVERY_TIMEOUT",
		type: 0x02n,
		scopes: [
			"PUBLISH_OK",
			"SUBSCRIBE",
			"REQUEST_UPDATE",
			"REQUEST_UPDATE_OF_FETCH",
		],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.objectDeliveryTimeout = reader.varint();
		},
		writers: (parameters) =>
			present(parameters.objectDeliveryTimeout, writeVarint),
	},
	{
		// FIXME: the Token structure is not decoded, so the MUSTs of §10.2.2
		// (malformed Token, alias registration and lookup, the token cache limit,
		// uniqueness of repeated tokens) are not enforced. They wait for the
		// project's decision on authorization.
		name: "AUTHORIZATION_TOKEN",
		type: 0x03n,
		scopes: [
			"PUBLISH",
			"SUBSCRIBE",
			"PUBLISH_NAMESPACE",
			"REQUEST_UPDATE",
			"FETCH",
			"REQUEST_UPDATE_OF_FETCH",
			"SUBSCRIBE_NAMESPACE",
		],
		repeatable: true,
		read: (reader, parameters) => {
			const length = readLength(
				reader,
				reader.remaining,
				"AUTHORIZATION_TOKEN length",
			);
			parameters.authorizationToken = [
				...(parameters.authorizationToken ?? []),
				reader.take(length).slice(),
			];
		},
		writers: (parameters) =>
			(parameters.authorizationToken ?? []).map(
				(token) => (writer: ByteWriter) => {
					writer.varint(token.length).bytes(token);
				},
			),
	},
	{
		// WHY a varint: §10.2.6 gives the meaning (milliseconds) and no
		// encoding. moqtail 0.15.0 writes it as a varint
		// (libs/moqtail-rs/src/model/parameter/message_parameter.rs at tag
		// relay@0.15.0, lines 510-511, read on 2026-10-10). No peer has sent this
		// parameter in a run, so no captured bytes back this choice.
		name: "RENDEZVOUS_TIMEOUT",
		type: 0x04n,
		scopes: ["SUBSCRIBE"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.rendezvousTimeout = reader.varint();
		},
		writers: (parameters) => present(parameters.rendezvousTimeout, writeVarint),
	},
	{
		name: "SUBGROUP_DELIVERY_TIMEOUT",
		type: 0x06n,
		scopes: [
			"PUBLISH_OK",
			"SUBSCRIBE",
			"REQUEST_UPDATE",
			"REQUEST_UPDATE_OF_FETCH",
		],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.subgroupDeliveryTimeout = reader.varint();
		},
		writers: (parameters) =>
			present(parameters.subgroupDeliveryTimeout, writeVarint),
	},
	{
		name: "EXPIRES",
		type: 0x08n,
		scopes: ["SUBSCRIBE_OK", "PUBLISH", "PUBLISH_OK"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.expires = reader.varint();
		},
		writers: (parameters) => present(parameters.expires, writeVarint),
	},
	{
		name: "LARGEST_OBJECT",
		type: 0x09n,
		scopes: ["SUBSCRIBE_OK", "PUBLISH"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.largestObject = decodeLocation(reader);
		},
		writers: (parameters) => present(parameters.largestObject, encodeLocation),
	},
	{
		// WHY a varint: §10.2.5 gives the meaning (milliseconds) and no
		// encoding, like §10.2.6 for RENDEZVOUS_TIMEOUT above. The Rust
		// implementation and the moqtail crate 0.15.0 read it so
		// (`FillTimeout` in `src/model/parameter/message_parameter.rs` of the
		// crate, as the Rust implementation recorded on 2026-10-11).
		name: "FILL_TIMEOUT",
		type: 0x0an,
		scopes: ["FETCH"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.fillTimeout = reader.varint();
		},
		writers: (parameters) => present(parameters.fillTimeout, writeVarint),
	},
	{
		name: "FORWARD",
		type: 0x10n,
		scopes: ["SUBSCRIBE", "PUBLISH", "PUBLISH_OK", "REQUEST_UPDATE"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.forward = decodeForward(reader.uint8());
		},
		writers: (parameters) =>
			present(parameters.forward, (writer, forward) => {
				writer.uint8(forward ? 1 : 0);
			}),
	},
	{
		// §10.2.7 has the update of a FETCH as well; the REQUEST_UPDATE decoded
		// here is the one to a subscription.
		name: "SUBSCRIBER_PRIORITY",
		type: 0x20n,
		scopes: [
			"SUBSCRIBE",
			"PUBLISH_OK",
			"REQUEST_UPDATE",
			"FETCH",
			"REQUEST_UPDATE_OF_FETCH",
		],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.subscriberPriority = reader.uint8();
		},
		writers: (parameters) =>
			present(parameters.subscriberPriority, (writer, priority) => {
				writer.uint8(priority);
			}),
	},
	{
		name: "SUBSCRIPTION_FILTER",
		type: 0x21n,
		scopes: ["SUBSCRIBE", "PUBLISH_OK", "REQUEST_UPDATE"],
		repeatable: false,
		read: (reader, parameters) => {
			const length = readLength(
				reader,
				reader.remaining,
				"SUBSCRIPTION_FILTER length",
			);
			const filter = reader.sub(length);
			parameters.subscriptionFilter = decodeSubscriptionFilter(filter);
			if (filter.remaining > 0) {
				throw protocolViolation(
					"SUBSCRIPTION_FILTER is longer than its Filter Type allows",
				);
			}
		},
		writers: (parameters) =>
			present(parameters.subscriptionFilter, (writer, filter) => {
				const encoded = createByteWriter();
				encodeSubscriptionFilter(encoded, filter);
				writer.varint(encoded.length).bytes(encoded.finish());
			}),
	},
	{
		name: "GROUP_ORDER",
		type: 0x22n,
		scopes: ["SUBSCRIBE", "PUBLISH_OK", "FETCH"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.groupOrder = decodeGroupOrder(reader.uint8());
		},
		writers: (parameters) =>
			present(parameters.groupOrder, (writer, groupOrder) => {
				writer.uint8(Number(GROUP_ORDERS.toWire(groupOrder)));
			}),
	},
	{
		name: "NEW_GROUP_REQUEST",
		type: 0x32n,
		scopes: ["PUBLISH_OK", "SUBSCRIBE", "REQUEST_UPDATE"],
		repeatable: false,
		read: (reader, parameters) => {
			parameters.newGroupRequest = reader.varint();
		},
		writers: (parameters) => present(parameters.newGroupRequest, writeVarint),
	},
	{
		name: "TRACK_NAMESPACE_PREFIX",
		type: 0x34n,
		// §10.2.14: "MAY appear in REQUEST_UPDATE for a SUBSCRIBE_NAMESPACE or
		// SUBSCRIBE_TRACKS request".
		scopes: ["REQUEST_UPDATE_OF_SUBSCRIBE_NAMESPACE"],
		repeatable: false,
		// §10.2.14: "uses the Track Namespace encoding described in Section
		// 2.4.1", with no length before it, as in the Rust implementation
		// (`crates/moqt/src/params.rs`, where the moqtail crate's other
		// reading is).
		read: (reader, parameters) => {
			parameters.trackNamespacePrefix = decodeTrackNamespace(reader);
		},
		writers: (parameters) =>
			present(parameters.trackNamespacePrefix, encodeTrackNamespace),
	},
];

const PARAMETERS_BY_TYPE: ReadonlyMap<bigint, ParameterDefinition> = new Map(
	PARAMETERS.map((definition) => [definition.type, definition]),
);

const REQUEST_UPDATE_KINDS: readonly ParameterScope[] = [
	"REQUEST_UPDATE",
	"REQUEST_UPDATE_OF_FETCH",
	"REQUEST_UPDATE_OF_SUBSCRIBE_NAMESPACE",
];

function allows(
	definition: ParameterDefinition,
	scope: ParameterScope,
): boolean {
	switch (scope) {
		case "REQUEST_OK":
			return REQUEST_OK_KINDS.some((kind) => definition.scopes.includes(kind));
		case "ANY_REQUEST_UPDATE":
			return REQUEST_UPDATE_KINDS.some((kind) =>
				definition.scopes.includes(kind),
			);
		default:
			return definition.scopes.includes(scope);
	}
}

/**
 * Names of the parameters in `parameters` that §10.2.1 does not allow in
 * `scope`. No parameter is defined for PUBLISH_NAMESPACE_OK, so every
 * parameter is outside that scope.
 */
export function parametersOutsideScope(
	parameters: MessageParameters,
	scope: ParameterScope,
): string[] {
	return PARAMETERS.filter(
		(definition) =>
			definition.writers(parameters).length > 0 && !allows(definition, scope),
	).map((definition) => definition.name);
}

/**
 * Decodes "Number of Parameters" and the parameters that follow.
 *
 * §10.2: a receiver has to understand every parameter, so an unknown Type
 * cannot be skipped. §10.2.1: a parameter in a message it is not defined for
 * is a PROTOCOL_VIOLATION.
 */
export function decodeMessageParameters(
	reader: ByteReader,
	scope: ParameterScope,
): MessageParameters {
	const count = reader.varint();
	// The same bound as for Key-Value-Pairs, and for the same reason: a
	// repeated AUTHORIZATION_TOKEN can take two bytes on the wire.
	// WHY compared as soon as it is read: a count over the limit that is also
	// larger than the bytes that follow is reported as the limit, as the Rust
	// implementation does. Up to the limit, a count the message is too short
	// for fails where the missing parameter is read.
	if (count > BigInt(MAX_KEY_VALUE_PAIRS)) {
		throw new TooManyEntriesError("Message Parameters in one message");
	}
	const parameters: MessageParameters = {};
	let type = 0n;
	for (let i = 0; i < count; i++) {
		const delta = reader.varint();
		type += delta;
		if (type > MAX_VARINT) {
			throw protocolViolation("Parameter Type exceeds 2^64 - 1");
		}
		const definition = PARAMETERS_BY_TYPE.get(type);
		if (definition === undefined || !allows(definition, scope)) {
			throw protocolViolation(
				`Message Parameter 0x${type.toString(16)} is unknown or not allowed in ${scope}`,
			);
		}
		if (i > 0 && delta === 0n && !definition.repeatable) {
			throw protocolViolation(`duplicate ${definition.name} parameter`);
		}
		definition.read(reader, parameters);
	}
	return parameters;
}

/** Writes "Number of Parameters" and the parameters in ascending Type order (§10.2). */
export function encodeMessageParameters(
	writer: ByteWriter,
	parameters: MessageParameters,
	scope: ParameterScope,
): void {
	const outside = parametersOutsideScope(parameters, scope);
	if (outside.length > 0) {
		throw new RangeError(`${outside.join(", ")} not allowed in ${scope}`);
	}
	const values = PARAMETERS.flatMap((definition) =>
		definition
			.writers(parameters)
			.map((write) => ({ type: definition.type, write })),
	);
	writer.varint(values.length);
	let previous = 0n;
	for (const { type, write } of values) {
		writer.varint(type - previous);
		previous = type;
		write(writer);
	}
}
