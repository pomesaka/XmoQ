# A percent outside a variable reference in a catalog

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the MSF catalog; a known gap of `packages/msf/README.md` ("A value with `%` is written as it is").

## Objective

MSF-01 §5.4.1: "Variable names MUST consist of alphanumeric characters, hyphens, and underscores", and "The percent character (%) MUST NOT appear in catalog field values except as part of a variable reference. Literal percent characters are not permitted." Neither reader (`readCatalog` of `@xmoq/msf`, `read_catalog` of `crates/msf`) nor the writer (`encodeCatalog`) looked at it. Now both readers turn such a catalog down as invalid, and the writer does not write one (a `RangeError`, as for every catalog its reader turns down). A reference is read as it is: nothing is put in its place.

## Expected

**Written after the runs**, which were made before this record was begun. The rows of the table were written after `@xmoq/msf`'s reader was changed and before `crates/msf`'s.

- The six new rows of `fixtures/msf-01/catalogs.tsv` (`percent-*`) are read by both readers as the table says: two catalogs (a reference in a field that is not read, two references next to each other) and four invalid (`100%`, `50% off` in a field that is not read, `%a.b%`, `%%`).
- No catalog of the interop runs has a `%`: the runs that read and write catalogs, with `moqlivemock` among them, pass as before.

## Environment

macOS 14.4, arm64; the working copy on `main` at `urtqumlp`; `mise run check`; `bun interop/matrix.ts --only "catalog:"` and `--only "watch:"`, in the working copy. The full interop run was not made: another session was measuring on the same machine.

## Result

- As expected. `mise run check` passed: the table's 75 rows in `@xmoq/msf`, the six `percent-*` rows in `crates/msf`, and the writer's refusal of `100%` (`the catalog: "100%" has a "%" outside a variable reference (§5.4.1)`).
- `--only "catalog:"`: 4 of 4 pairings passed, among them a page that reads the catalog of `moqlivemock`'s publisher (13 tracks) and `moqlivemock`'s subscriber that reads a page's catalog. `--only "watch:"`: 2 of 2, the terminal viewer reading a page's catalog and `moqlivemock`'s.
- **Not run**: a catalog with a variable from another implementation; `moqlivemock` v0.16.1 sends none. What a relay or a viewer would put in place of a variable (§5.4.2) is not done.
