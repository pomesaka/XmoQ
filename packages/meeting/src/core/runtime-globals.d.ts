// The only runtime globals src/core may use. tsconfig.core.json type-checks
// src/core with no DOM library and no @types package, so anything not declared
// here (streams, timers, fetch, WebTransport) fails to compile there.
//
// WHY excluded from tsconfig.json: the DOM library and @types/bun declare the
// same names with their own types, and a second declaration is an error.

declare class TextDecoder {
	decode(input?: Uint8Array): string;
}
