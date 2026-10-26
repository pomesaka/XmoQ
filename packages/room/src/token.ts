// The token the relay checks (`crates/relay/src/auth.rs`):
// `v1|<namespace, its fields joined with />|<pub or sub>|<expiry, seconds since 1970>|<mac, hex>`,
// the mac HMAC-SHA-256 with the key the room shares with the relay, over the
// text before the last `|`. `crates/room-token` makes and reads the same:
// both read `fixtures/room-token/tokens.tsv`. With WebCrypto, which the
// Worker and Bun both have.

/** How long a token is good for, in seconds. */
export const TOKEN_SECONDS = 600;

export type Role = "pub" | "sub";

/**
 * The key of the setting `AUTH_KEY_HEX`: none where it is not set, and its
 * bytes where it is hexadecimal of one byte at least. Throws a `RangeError`
 * for one that is set and is not, as the relay stops for such an
 * `--auth-key-hex` (`key` of `crates/relay/src/main.rs`). WHY NOT none for
 * it: a room whose key was mistyped would give no token, and say nothing
 * of it, beside a relay that asks for one.
 */
export function keyOfSetting(
	setting: string | undefined,
): Uint8Array<ArrayBuffer> | undefined {
	if (setting === undefined) return undefined;
	if (!/^([0-9a-fA-F]{2})+$/.test(setting)) {
		throw new RangeError(
			"AUTH_KEY_HEX is hex of one byte at least, where it is set",
		);
	}
	return Uint8Array.from(setting.match(/../g) ?? [], (byte) =>
		Number.parseInt(byte, 16),
	);
}

/** The text of the token that allows `role` in `namespace` until `expiry`, with `key`. */
export async function tokenText(
	key: Uint8Array<ArrayBuffer>,
	namespace: readonly string[],
	role: Role,
	expiry: number,
): Promise<string> {
	const hmac = await crypto.subtle.importKey(
		"raw",
		key,
		{ name: "HMAC", hash: "SHA-256" },
		false,
		["sign"],
	);
	const signed = `v1|${namespace.join("/")}|${role}|${expiry}`;
	const mac = new Uint8Array(
		await crypto.subtle.sign("HMAC", hmac, new TextEncoder().encode(signed)),
	);
	return `${signed}|${Array.from(mac, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}
