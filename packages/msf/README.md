# @xmoq/msf

The MOQT Streaming Format, written from the text of `draft-ietf-moq-msf-01`. Section numbers below are of that draft.

Status: partly implemented. The catalog (§5) is read and written as bytes; nothing here opens a connection yet, and no page publishes or reads a catalog.

## Scope

Implemented: `readCatalog`, which says what the payload of a catalog Object is, and `encodeCatalog`, which makes the payload of an independent catalog. Both apply the same check.

Not implemented: publishing the catalog track and reading it over a connection (next), delta updates (told apart, not applied), compression (§12.1), timelines, encryption, authorization, URLs, log and metrics tracks, tracks the subscriber publishes (`publishTracks`, §5.1.5), initialization data (`initDataList`, §5.1.7; `initRef`), variable substitution (§5.4). LOC, the packaging of the media Objects, is not part of this package.

## Layer rules

`src/core` is JSON and bytes: it uses no I/O and does not import `@xmoq/moqt`. `tsconfig.core.json` type-checks it without the DOM library, and `noRestrictedImports` in the root `biome.jsonc` keeps the import out. What uses a `MoqtConnection` will live outside `src/core`. `@xmoq/moqt` does not depend on this package (`scripts/ci/check-layers.sh`).

## How the draft is read

| Point | What is done | From |
|---|---|---|
| The value of `version` | `"draft-01"` is written, and nothing else is read: another value is `unsupported-version` | §5.1.1: "follow the convention of specifying the version as "draft-XX"", and "A subscriber MUST NOT attempt to parse a catalog version which it does not understand." **Every example of a whole catalog in the draft (§5.6; the two of a delta update, §5.6.4 and §5.6.5, have no version) has `"version": "1"`**, which is not read. Provisional |
| A field with "Required: Yes" that is missing or of another JSON type (`version`, `tracks`, and `name`, `packaging`, `isLive` of a track) | The catalog is `invalid` | §5: "At a minimum, a catalog MUST provide all mandatory fields." The examples are "non-normative" (§5.6) and are not followed where they differ: the timeline tracks of §5.6.9 have no `isLive`, and are `invalid` here |
| `codec` and `bitrate` of a track | Required where `packaging` is `"loc"`, optional elsewhere | §5.2.18 asks for a codec on "tracks which have an inherent codec associated with them (e.g., audio and video tracks)" and §5.2.22 for a bitrate on "audio and video tracks". No field says which tracks those are; a LOC track holds an encoded audio or video sample an Object (§4.1). The LOC video track of the example in §5.6.14 has neither, and is `invalid` here |
| `samplerate` and `channelConfig` | Not looked at | §5.2.28 and §5.2.29 ask for them on "tracks for which audio codecs are specified". Telling an audio codec string from a video one takes the WebCodecs registry |
| A field this package reads that has another JSON type than the draft gives it (`generatedAt`, `width`, ...), **`null` among them** | `invalid` | The draft gives each field a JSON type and says nothing of a value of another one, nor of `null`. `null` is never read as "the field is absent": a serializer that writes `null` for an empty field exists, and so does a parser that reads `null` as absent (`Option` of Rust's serde), so two readers could take one document in two ways |
| A number that is too large for a double (`1e999`) | `invalid`, as no number | RFC 8259 §9: "An implementation may set limits on the range and precision of numbers." `JSON.parse` reads such a number as Infinity, which could not be written back |
| A payload of more than 1 MiB, or of more than the caller says | `invalid`, and not parsed | The draft sets no size; RFC 8259 §9: "An implementation may set limits on the size of texts that it accepts." The payload is parsed whole, so the reader sets one; **the number is provisional** (`DEFAULT_MAX_CATALOG_LENGTH` has the reasoning) |
| A byte order mark before the document | `invalid` | RFC 8259 §8.1: "Implementations MUST NOT add a byte order mark (U+FEFF) to the beginning of a networked-transmitted JSON text", and "implementations that parse JSON texts MAY ignore the presence of a byte order mark rather than treating it as an error." Ignoring is a choice, and parsers differ (`serde_json` of Rust refuses one): refused, so that a Rust reader of ours gives the same answer to the same bytes |
| A string with half of a surrogate pair (`"\ud800"`), in a field that is read | `invalid`, as no string | JSON can spell it (RFC 8259 §8.2 calls the behaviour of software that receives one "unpredictable"). It cannot be written as UTF-8, so a track of that name would be subscribed to under another name; `serde_json` refuses it |
| A key that comes twice in one object | Not noticed: the later one is read | RFC 8259 §4: "The names within an object SHOULD be unique", and what a parser does with a second one differs. `JSON.parse` keeps the later one and does not say so. **A reader in Rust has to be made to do the same, or both to refuse** |
| A field this package does not read | Ignored, and not written | §5: "A parser MUST ignore fields it does not understand." |
| Two tracks of one name | `invalid` when both have the same `namespace`, or neither has one | §5.2.3: "track names MUST be unique per namespace". A track without `namespace` is in the namespace of the catalog track (§5.2.2), which the check does not know: one track with that namespace spelled out and one without are not found |
| `isComplete` that is `false` | `invalid` | §5.1.3: "This field MUST NOT be included if it is FALSE." That is a rule for the one who writes; the draft does not say what a reader does. It is applied to what is read as well: a document that breaks a MUST is not guessed at |
| `deltaUpdate` | Absent: a whole catalog. An array with at least one entry, in a document with neither `tracks` nor `version`: `delta-update`. Anything else (`null`, `false`, a string, an empty array, a document that also has `tracks` or `version`): `invalid` | §5.1.6: "JSON Type: Array", "If this field is present, the catalog represents a delta (or partial) update ... If this field is absent, the catalog is independent." §5.3: "A delta update MUST include the Delta Update ... field with at least one operation. It MUST NOT contain an instance of a Tracks ... field or an MSF version ... field." A document that is neither clearly the one nor the other is read as neither. The operations are not looked at |

## Known gaps

- **`readCatalog` sees one payload, so the rules that hold between catalogs are not checked here.** They belong to the part that reads or publishes the catalog track, which does not exist yet:
  - the track is named `"catalog"` (§5);
  - Object 0 of a Group is a whole catalog and every later Object a delta update, and "Any catalog updates that precede the first Object of the latest Group MUST be ignored" (§5);
  - `isLive`: "A True value MUST never follow a False value" (§5.2.7);
  - `isComplete`: "This field MUST NOT be removed from a catalog once it has been added" (§5.1.3);
  - the attributes of a track "MUST NOT be modified after being declared" (§5.3);
  - a track without `namespace` is in the namespace of the catalog track (§5.2.2), which is not resolved.
- **A track that is encrypted cannot be told from one that is not**: `encryptionScheme` is not read, and §4.3.1 has "subscribers MUST decrypt the object payload using the specified scheme before processing." Before a catalog of another publisher is used to choose a track, the field has to be read or such a track left out.
- **A catalog with an audio track cannot be written as the draft asks**: `samplerate` and `channelConfig` (§5.2.28, §5.2.29) are not among the fields that are written. Needed when audio is added.

- **Delta updates are not applied.** A publisher that sends one (§5.3) cannot be followed past it; a reader has to wait for the next Group, whose first Object is a whole catalog (§5).
- **Compressed catalogs are not read**, which §12.1 asks for: "All MSF implementations MUST support both uncompressed payloads (value 0 or property absent) and GZIP compressed payloads (value 1)." The property that says a payload is compressed has no number in the draft (§14, the table of MSF properties: `MSF_COMPRESSION`, "TBD"), so no two implementations can agree on one from the text. A compressed payload is `invalid` here.
- Where `initDataList` stands in the document (§5.1.7: "MUST be located after the tracks array") is not looked at; the field is not read.
- The JSON type of a field is all that is checked of its value: a `bitrate` of -1 and a `name` that is empty are read.
- `encodeCatalog` does not limit the length of what it writes, and lets the `TypeError` of `JSON.stringify` through for a value that is no `Catalog` by its type (one that contains itself, a BigInt).

## Checked against

`fixtures/msf-01/` has the catalog `moqlivemock`'s publisher sent, which is read without a complaint, and a table of documents written by hand with what each is read as.
