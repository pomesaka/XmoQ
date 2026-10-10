// Entry "@xmoq/moqt/diagnostics": the types of `ConnectOptions.diagnostics`.
// They expose the session's internal events, so they are kept apart from the
// main entry, whose types an application can rely on.

export type { SessionEvent } from "./core/session-output.ts";
export type { Diagnostics } from "./webtransport/diagnostics.ts";
