// The only runtime global this package uses beyond ES2022.
// tsconfig.core.json type-checks it with no DOM library and no @types
// package, so anything not declared here fails to compile there.
//
// WHY excluded from tsconfig.json: the DOM library and @types/bun declare the
// same names with their own types, and a second declaration is an error.

declare class TextDecoder {
	constructor(
		label?: string,
		options?: { fatal?: boolean; ignoreBOM?: boolean },
	);
	decode(input?: Uint8Array): string;
}
