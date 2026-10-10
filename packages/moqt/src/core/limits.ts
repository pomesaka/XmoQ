import type { SubgroupStreamLimits } from "./subgroup.ts";

/**
 * The limits an application can set. The fixed limits of the implementation
 * (how many Track Aliases are remembered, how many Key-Value-Pairs a message
 * may hold) are constants next to the code they bound.
 */
export interface SessionLimits extends SubgroupStreamLimits {
	/**
	 * Bytes held for subgroup streams that arrived before the SUBSCRIBE_OK
	 * carrying their Track Alias (§11.4.2 allows buffering them "for a brief
	 * period"). Each held Object counts its payload, its properties and a fixed
	 * overhead, so Objects without a payload are bounded too.
	 */
	maxHeldSubgroupBytes: number;
}

/** Provisional values: the draft sets no maximum and the project has not chosen any. */
const DEFAULT_SESSION_LIMITS: SessionLimits = {
	maxObjectPayloadLength: 16 * 1024 * 1024,
	maxObjectPropertiesLength: 64 * 1024,
	maxHeldSubgroupBytes: 1024 * 1024,
};

/** The limits to run with: the defaults, with what the application set on top. */
export function resolveLimits(
	limits: Partial<SessionLimits> = {},
): SessionLimits {
	return {
		maxObjectPayloadLength:
			limits.maxObjectPayloadLength ??
			DEFAULT_SESSION_LIMITS.maxObjectPayloadLength,
		maxObjectPropertiesLength:
			limits.maxObjectPropertiesLength ??
			DEFAULT_SESSION_LIMITS.maxObjectPropertiesLength,
		maxHeldSubgroupBytes:
			limits.maxHeldSubgroupBytes ??
			DEFAULT_SESSION_LIMITS.maxHeldSubgroupBytes,
	};
}
