// The namespace the publishing page announces: the pages read it through
// `shared.ts`, and `serve.ts` has the transcriber listen under it. WHY a
// file of its own: `shared.ts` reads what `serve.ts` writes into the pages,
// and does not load outside one.

export const NAMESPACE = ["xmoq", "demo"];
