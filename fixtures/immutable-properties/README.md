# Immutable Properties

What a receiver makes of the Properties of an Object or a Track that hold Immutable Properties (Property Type 0xB, draft 18 §12.7): the properties it searches for the value of a property, or why the Track is not processed. Both implementations read this table, so that the receiver in each takes the same Properties for malformed.

## Format

`properties.tsv`, one row per case, six columns separated by tabs, no header. Every row ends with one line feed, the last one too; there are no empty rows. Hexadecimal digits are lower case.

| Column | Content |
|---|---|
| 1 | Name of the row; unique together with column 2 |
| 2 | `object`: the bytes are the Object Properties of one Object, the field after Object Properties Length (§11.2.1.2). `track`: they are the Track Properties at the end of a SUBSCRIBE_OK (§10.8), the remaining bytes of the message |
| 3 | For `object`, the Group ID and Object ID of the Object as `group/object`. `-` for `track` |
| 4 | The Properties, in hexadecimal digits |
| 5 | `ok`: the Properties are accepted. `malformed-track`: the Track is malformed (§2.4.2); the Object's subscription or FETCH is cancelled, or the SUBSCRIBE_OK's subscription. `protocol-violation`: the session is closed with `PROTOCOL_VIOLATION`. `unsupported-track`: the track is not processed because of a Mandatory Track Property this endpoint does not understand (§2.5.1); `track` rows only |
| 6 | For `ok`, the properties searched for the value of a property, in order: the ones outside Immutable Properties without it, in the order they came, then the ones inside each Immutable Properties. Each is `type=value`, the Type in hexadecimal digits and the value in decimal where the Type is even, in hexadecimal digits where it is odd; separated by commas. `none` for no property. `-` for any other outcome |

## What the rows hold a reader to

- The pairs inside Immutable Properties are a sequence of their own (Figure 2): the first Delta Type counts from 0, not from 0xB (`inner-types-count-from-zero`).
- What is checked of an Object is looked for inside as well (§12.7: "processors MUST search both"). A Prior Object ID Gap or Prior Group ID Gap inside that is larger than the ID it counts back from, one outside and one inside (two instances, §12.9, §12.8), and a Mandatory Track Property inside (§2.5.1) make the Track malformed. For a Track: a DYNAMIC_GROUPS above 1 inside closes the session (§12.6), and a Mandatory Track Property inside keeps the track from being processed.
- §12.7 calls malformed: Immutable Properties inside Immutable Properties, and a Key-Value-Pair inside that cannot be parsed (here, one that runs past the end of the value). **The draft has the first for an Object; the table has it for a Track as well.**
- An Object with two Immutable Properties is malformed: §12.7 has "An Object MUST NOT contain more than one instance of this property" without a consequence, and §2.4.2 has a Track that does not conform to MOQT constraints malformed. A Track may have two, and the contents of both are searched: the draft does not forbid it.
- A Delta Type inside that takes the Type over 2^64 - 1 closes the session with `PROTOCOL_VIOLATION`, as §1.4.3 has for every Key-Value-Pair.
- Malformed is decided before a Mandatory Track Property is looked for (`malformed-before-mandatory`).

What a row does not cover: the conditions of §12.8 and §12.9 that compare one Object with another, and a reader that is a relay. The reasons are in [the implementation notes](../../docs/moqt-implementation-notes.md), the row on Immutable Properties.

## Source

**Every byte was written by hand from the text of §1.4.3 and §12.7 of `draft-ietf-moq-transport-18`, and column 5 is the reading of the implementation notes, not an observation.** No capture of another implementation carries Immutable Properties: `moqtail` 0.15.0 encodes and decodes them (`src/model/property/object_property.rs`, read on 2026-10-24); no run of its relay or test client was made to send one. It reads two of the rows otherwise: Immutable Properties inside Immutable Properties are a `ProtocolViolation` there, and its check of the gaps (`validate_prior_gaps`) does not look inside.

The Types in the rows: 0x04 MAX_CACHE_DURATION, 0x10 and 0x11 Types no property of the draft has, 0x30 DYNAMIC_GROUPS, 0x3C Prior Group ID Gap, 0x3E Prior Object ID Gap, 0x4000 a Mandatory Track Property (`c0 40 00` in three bytes), 0x0B Immutable Properties.
