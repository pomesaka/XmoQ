# Catalogs of draft-ietf-moq-msf-01

What `@xmoq/msf` and `crates/msf` read a catalog as. Both read `catalog-moqlivemock.json` and every row of `catalogs.tsv`.

## `catalog-moqlivemock.json`: what another implementation sent

The payload of the one Object of the catalog track of `mlmpub`, the publisher of [`Eyevinn/moqlivemock`](https://github.com/Eyevinn/moqlivemock) v0.16.1 (commit `91d903e9933a452f8baaa860f9fcd20cf195e077`), 2,715 bytes, as they came. Taken on 2026-10-11:

```sh
git worktree add --detach <dir> v0.16.1 && cd <dir>       # a tree with no change of ours
go build -o <bin>/mlmpub ./cmd/mlmpub                     # Go 1.26.0
(cd cmd/mlmpub && <bin>/mlmpub -addr 127.0.0.1:47200 -sideport 47201)
xmoq-moqt-client --mode subscribe --objects 1 --url https://127.0.0.1:47200/moq \
    --cert-sha256 "$(curl -s http://127.0.0.1:47201/fingerprint)" \
    --namespace mlm/msf/clear --track catalog
```

The own Rust client of `main` at `6ad5240c` printed `OBJECT group=0 object=0 status=Normal properties=none payload_len=2715 payload_hex=...`; the file is those bytes (SHA-256 `5440467e492ab7b18b336979f6af7fd071066b070e81ca3e5ab6b8b68ddf3b8f`). It has no newline at its end, and must not get one.

It decodes to: `"version": "draft-01"`, `"generatedAt": 1791685571315`, and 13 tracks, all `"packaging": "loc"` and `"isLive": true`.

No track has `namespace`, `initRef` or `timescale`, and the catalog has no `initDataList`.

## `catalogs.tsv`: documents written by hand

**Nobody's implementation wrote these documents.** They were written by hand from the text of the draft, one for each thing the check of `@xmoq/msf` looks at, and the person who read the draft wrote down what each is read as. They are no check against another implementation; they keep two implementations of ours from reading one document in two ways.

80 rows, tab-separated, a newline after each:

| Column | |
|---|---|
| 1 | A name, unique in the table |
| 2 | What the document is read as: `catalog`, `delta-update`, `unsupported-version` or `invalid` |
| 3 | The document: the bytes of the payload, as UTF-8. No row has a tab or a newline in it |

**Not every row follows from the draft.** The rows about `null`, a number too large for a double (`1e999`), half of a surrogate pair, a key that comes twice (`version-twice`), `percent-around-no-name`, `samplerate-that-is-no-whole-number` and `byte-order-mark` (a document that begins with the bytes `ef bb bf`) are choices of `@xmoq/msf` where the draft, and JSON, leave one, written down so that a second implementation of ours makes the same one; the next section has each, with its reason. A parser that is not `JSON.parse` can differ on each of them.

Where each outcome comes from in the draft is in the next section. The rows `an-array`, `a-string`, `no-json` and `null` are no JSON object, and the row `no-json` is no JSON at all.

## How the draft is read

What a reader does with each kind of document, where a draft says it and where it leaves a choice. "Here" is `@xmoq/msf`.

| Point | What is done | From |
|---|---|---|
| The value of `version` | `"draft-01"` is written, and nothing else is read: another value is `unsupported-version` | §5.1.1: "follow the convention of specifying the version as "draft-XX"", and "A subscriber MUST NOT attempt to parse a catalog version which it does not understand." **Every example of a whole catalog in the draft (§5.6; the two of a delta update, §5.6.4 and §5.6.5, have no version) has `"version": "1"`**, which is not read. Provisional |
| A field with "Required: Yes" that is missing or of another JSON type (`version`, `tracks`, and `name`, `packaging`, `isLive` of a track) | The catalog is `invalid` | §5: "At a minimum, a catalog MUST provide all mandatory fields." The examples are "non-normative" (§5.6) and are not followed where they differ: the timeline tracks of §5.6.9, and the event timeline track of §5.6.14, have no `isLive`, and are `invalid` here |
| `codec` and `bitrate` of a track | Required where `packaging` is `"loc"`, optional elsewhere | §5.2.18 asks for a codec on "tracks which have an inherent codec associated with them (e.g., audio and video tracks)" and §5.2.22 for a bitrate on "audio and video tracks". No field says which tracks those are; a LOC track holds an encoded audio or video sample an Object (§4.1). The LOC video track of the example in §5.6.14 has neither, and is `invalid` here |
| `renderGroup` and `altGroup` | Read and written where they are there, as numbers; nothing is done with them here | §5.2.11 and §5.2.12. Tracks of one `altGroup` are "alternate versions of one-another" of which "a subscriber typically subscribes to one"; choosing is the caller's. That alternates are time-aligned (§4.2, MUST) is the publisher's to keep and is not checked |
| `samplerate` and `channelConfig` | Read and written where they are there, a number and a string; **asked for on no track** | §5.2.28, "The number of audio frame samples per second", and §5.2.29, "A string specifying the audio channel configuration", ask for them on "tracks for which audio codecs are specified". Telling an audio codec string from a video one takes the WebCodecs registry, so a track without them is not turned down. What the string of `channelConfig` holds the draft does not say; its examples and `moqlivemock` have the number of channels, `"2"`. A `samplerate` that is no whole number is kept as it is, here and in `crates/msf`: §5.2.28 has "JSON Type: Number" and no more |
| A field `@xmoq/msf` reads that has another JSON type than the draft gives it (`generatedAt`, `width`, ...), **`null` among them** | `invalid` | The draft gives each field a JSON type and says nothing of a value of another one, nor of `null`. `null` is never read as "the field is absent": a serializer that writes `null` for an empty field exists, and so does a parser that reads `null` as absent (`Option` of Rust's serde), so two readers could take one document in two ways |
| A `%` in a string value outside a variable reference (`%name%`, the name of ASCII letters, digits, `-` and `_`, at least one), in any field, those that are not read among them | `invalid`, read and written alike, and in `crates/msf`; a reference is read as it is, and nothing is put in its place | §5.4.1: "Variable names MUST consist of alphanumeric characters, hyphens, and underscores", and "The percent character (%) MUST NOT appear in catalog field values except as part of a variable reference. Literal percent characters are not permitted." WHY ASCII and at least one character: the draft says no more of "alphanumeric", and "consist of" names no empty name (`fixtures/msf-01/catalogs.tsv`, the rows `percent-*`) |
| A number that is too large for a double (`1e999`), anywhere in the document, in a field that is not read as well | `invalid`, and in `crates/msf` | RFC 8259 §9: "An implementation may set limits on the range and precision of numbers", and §6 has such numbers as ones that do not interoperate. `serde_json` refuses the whole document, and so does `@xmoq/json`, which `@xmoq/msf` reads a catalog with (`JSON.parse` would read the number as Infinity). WHY anywhere and not only in a field that is read: the two readers then read every document alike |
| A payload of more than `DEFAULT_MAX_CATALOG_LENGTH`, or of more than the caller says | `invalid`, and not parsed. A number of the caller that is no length (NaN, Infinity, a negative one, one that is no integer) is a `RangeError` | The draft sets no size; RFC 8259 §9: "An implementation may set limits on the size of texts that it accepts." The payload is parsed whole, so the reader sets one; **the number is provisional** (`DEFAULT_MAX_CATALOG_LENGTH` has the reasoning) |
| A byte order mark before the document | `invalid` | RFC 8259 §8.1: "Implementations MUST NOT add a byte order mark (U+FEFF) to the beginning of a networked-transmitted JSON text", and "implementations that parse JSON texts MAY ignore the presence of a byte order mark rather than treating it as an error." Ignoring is a choice, and parsers differ (`serde_json` of Rust refuses one): refused, so that a Rust reader of ours gives the same answer to the same bytes |
| A string or a key with half of a surrogate pair (`"\ud800"`), anywhere in the document, in a field that is not read as well | `invalid`, and in `crates/msf` | JSON can spell it, and RFC 8259 §8.2 has "the behavior of software that receives JSON texts containing such values is unpredictable". It cannot be written as UTF-8, so a track of that name would be subscribed to under another name; `serde_json` and `@xmoq/json` refuse the whole document. WHY anywhere: as for a number too large for a double |
| A key that comes twice in one object | Not noticed: the later one is read, and in `crates/msf` | RFC 8259 §4: "The names within an object SHOULD be unique", and what a parser does with a second one differs. `@xmoq/json` and `serde_json` (its `Value`) keep the later one and do not say so |
| Arrays and objects nested 128 deep or more | `invalid`, and in `crates/msf` | RFC 8259 §9: "An implementation may set limits on the maximum depth of nesting." `serde_json` reads 127 and refuses 128, and `@xmoq/json` does the same |
| A field `@xmoq/msf` does not read | Ignored, and not written | §5: "A parser MUST ignore fields it does not understand." |
| Two tracks of one name | `invalid` when both have the same `namespace`, or neither has one | §5.2.3: "track names MUST be unique per namespace". A track without `namespace` is in the namespace of the catalog track (§5.2.2), which the check does not know: one track with that namespace spelled out and one without are not found |
| `isComplete` that is `false` | `invalid` | §5.1.3: "This field MUST NOT be included if it is FALSE." That is a rule for the one who writes; the draft does not say what a reader does. It is applied to what is read as well: a document that breaks a MUST is not guessed at |
| `deltaUpdate` | Absent: a whole catalog. An array with at least one entry, in a document with neither `tracks` nor `version`: `delta-update`. Anything else (`null`, `false`, a string, an empty array, a document that also has `tracks` or `version`): `invalid` | §5.1.6: "JSON Type: Array", "If this field is present, the catalog represents a delta (or partial) update ... If this field is absent, the catalog is independent." §5.3: "A delta update MUST include the Delta Update ... field with at least one operation. It MUST NOT contain an instance of a Tracks ... field or an MSF version ... field." A document that is neither clearly the one nor the other is read as neither. The operations are not looked at |
| A document with `deltaUpdate` that also has `tracks` or `version` | `invalid`, as above. **Not seen from another implementation**: the one publisher that was run, `mlmpub` of moqlivemock, never sends a delta update, so this rule was checked against the text alone. **A delta update made with the catalog type of moqlivemock v0.16.1 would be `invalid` here, by a reading of its source that was not run**: its `version` field is written also when it is empty (`internal/catalog.go` has no `omitempty` on it), and a document with `"version":""` and `deltaUpdate` is `invalid` (the row `delta-update-with-an-empty-version`) | §5.3, whose rules are for "constructing and processing" a delta update |
| A payload that begins `1f 8b` | `invalid`, with a reason that says it looks like GZIP | RFC 1952 §2.3.1. Whether a payload is compressed is said by a property (§12.1) whose number the draft does not give; the first bytes are only a hint for a log |
| `encryptionScheme` of a track | Read, and handed on with the track. A catalog that has one is not written | §4.3.1: "subscribers MUST decrypt the object payload using the specified scheme before processing." Nothing here decrypts, so whoever uses the catalog has to leave such a track alone. §5.2.39 asks for a cipher suite with it, which is not written |
| The string of `namespace` as the tuple of MoQ Transport | Split at "/", empty fields left out (`namespaceTuple`) | **The draft has no rule**; §11.1.2 has an encoding for URLs only. Its examples have "conference.example.com/conference123/alice", and `moqlivemock` splits at "/" and drops empty fields (`internal/namespace.go`, v0.16.1). A namespace with a field that has a "/" in it, or that is no UTF-8, cannot be named in a catalog this way; a track in such a namespace has to be in that of the catalog track, without `namespace`. **The result is not checked against MoQ Transport**: it can have more than the 32 fields, or more than the 4,096 bytes of a Full Track Name, that draft 18 §2.4.1 allows, and `subscribe` of `@xmoq/moqt` then rejects. `""` and `"/"` are a namespace of no field, which draft 18 allows, and not "no namespace": a track that has `"namespace": ""` is not in the namespace of the catalog track (§5.2.2 has a track inherit when the namespace "is not declared", which is read as: the key is absent) |
