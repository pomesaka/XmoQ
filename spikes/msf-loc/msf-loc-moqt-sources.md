# What "fully following" LOC-04 and MSF-01 obliges for XmoQ Phase 1

Read on 2026-10-10. Sources (all plain text, read locally):

| Short name | File | Date in the draft |
|---|---|---|
| MSF-01 | `scratchpad/formats/msf-01.txt` | 2 June 2026 |
| LOC-04 | `scratchpad/formats/loc-04.txt` | 20 July 2026 |
| LOC-03, -02, -01, -00 | `scratchpad/formats/loc-0N.txt` (fetched today from `https://www.ietf.org/archive/id/draft-ietf-moq-loc-0N.txt`, all HTTP 200) | see A |
| MOQT-18 | `scratchpad/draft/draft-ietf-moq-transport-18.txt` | May 2026 (12 May 2026 per the reference entries that cite it) |
| MOQT-19 | `scratchpad/formats/draft-ietf-moq-transport-19.txt` (fetched today, HTTP 200) | July 2026 (6 July 2026 per LOC-04's reference entry) |

How much was read: MSF-01 and LOC-04 completely. LOC-03 by a full `diff` against LOC-04. LOC-02/-01/-00 only for date, references, property IDs and terms (not line by line). MOQT-18: §1.4, §2.1 to §2.5.1, §3.3.2 to §3.4, §5.1 (state, filters, joining), §5.2, §8, §9, §10 (table, FILL_TIMEOUT to FORWARD, REQUEST_ERROR codes, SUBSCRIBE, SUBSCRIBE_OK, PUBLISH_DONE, FETCH, FETCH_OK), §11.1 to §11.4.4, §12, §15.7, §15.8, §15.10, Appendix A.1. MOQT-19: §1.4, §12.1 to §12.7, §15.8, §15.9, Appendix A.1, the message table by `rg`, and a `diff` of the FETCH, Object, Subgroup and Fetch Header sections against MOQT-18. Sections of MOQT-18/19 not listed here were not read.

Quotes are verbatim; line breaks inside quotes are removed.

---

## A. Versions

| Draft | Date | MOQT it references | Evidence (reference entry, verbatim) |
|---|---|---|---|
| MSF-01 | 2 June 2026 | **draft 18** (normative) | §15.1 `[MoQTransport]`: "Media over QUIC Transport", Work in Progress, Internet-Draft, draft-ietf-moq-transport-18, 12 May 2026" |
| LOC-04 | 20 July 2026 | **draft 19** (normative) | §7.1 `[MoQTransport]`: "Internet-Draft, draft-ietf-moq-transport-19, 6 July 2026" |
| LOC-03 | 6 July 2026 | **draft 18** (normative) | §7.1: "Draft, draft-ietf-moq-transport-18, 12 May 2026" |
| LOC-02 | 15 March 2026 | draft 17 | §7.1: "Draft, draft-ietf-moq-transport-17, 2 March 2026" |
| LOC-01 | 7 July 2025 | draft 12 | "Draft, draft-ietf-moq-transport-12, 23 June 2025" |
| LOC-00 | 30 March 2025 | draft 10 | "Internet-Draft, draft-ietf-moq-transport-10, 3 March 2025" |

Which LOC does MSF-01 reference: **LOC-02**, normatively. MSF-01 §15.1: "[LOC] Zanaty, M., Nandakumar, S., and P. Thatcher, "Low Overhead Media Container", Work in Progress, Internet-Draft, draft-ietf-moq-loc-02, 15 March 2026, <https://datatracker.ietf.org/doc/html/draft-ietf-moq-loc-02>."

Is there a LOC revision that references MOQT 18, 17 or 16: **yes for 18 (LOC-03) and 17 (LOC-02); none for 16.**

Consequences:

- The combination the owner named (MSF-01 + LOC-04 + MOQT-18) is not a combination any of the three drafts names. MSF-01 points to MOQT-18 and LOC-02. LOC-04 points to MOQT-19 and (informatively) MSF-01: LOC-04 §7.2 "[MoQCatalog] ... draft-ietf-moq-msf-01, 2 June 2026".
- LOC-03 is the LOC revision written against MOQT-18. `diff loc-03.txt loc-04.txt` shows the body text is the same except: Timestamp ID (see B), the IANA table (2 rows in -03, 6 rows in -04), removal of -03 §6.2 "MoQ Streaming Format Registry", the "(IANA, please assign ...)" notes, and the references (MOQT 18 to 19, SecureObjects -00 to -01). No MUST/SHOULD sentence differs.
- Property IDs are **not stable across LOC revisions** (table in B.3). MSF-01's normative LOC (-02) uses different numbers from LOC-04.

---

## B. Can LOC-04 be carried on MOQT draft 18?

Short answer: **yes on the wire; the encodings LOC-04 relies on are the same in MOQT-18 and MOQT-19.** The only mismatch is the numeric IDs, where LOC-04 disagrees with the provisional table that both MOQT-18 and MOQT-19 print.

### B.1 Item by item

| What LOC-04 uses | LOC-04 (section, quote) | MOQT-18 (section, quote) | MOQT-19 (section, quote) | Conclusion |
|---|---|---|---|---|
| Term "Properties" / "Object Properties" | §2.2 "composed of a MOQ Object Header, with optional Properties, and a Payload ... populate the MOQ Object Properties with the LOC Public Properties" | §2.5 "Tracks and Objects can have additional relay-visible fields, known as Properties"; §11.2.1.2 "Object Properties" | same sections, same text | **Works on draft 18 unchanged.** Same name. The rename happened before 18: MOQT-19 App. A.3 (Since -16) "Rename Extension Headers to Properties (#1504)" |
| "Extension Headers" | not used in LOC-04 (LOC-01 used "LOC Header Extensions") | not the term of draft 18 | not the term | Not applicable. MSF-01 still uses the old term in two places (§4.3.4 "immutable header extensions", §6.1 "MOQT Prior Group ID Gap Extension header") |
| `vi64` | §2.3 "ID: Identifier assigned by the registry (vi64)" | §1.4.1 "x (vi64): Indicates that x holds an integer value using the variable-length encoding as described above."; "uses the number of leading 1 bits of the first byte to indicate the length of the encoding in bytes"; "Integers are encoded in 1 to 9 bytes" | §1.4.1 same sentences, same Table 1, same Table 2 | **Works on draft 18 unchanged.** `vi64` is already draft 18's own name; the tables are identical. It is not the QUIC RFC 9000 varint (leading-1-bits length, 1 to 9 bytes) |
| Even/odd rule | §2.3 "Length: Length of metadata Value in bytes (vi64 if ID is odd, omitted if ID is even)"; "Value: Value of metadata (vi64 if ID is even, Length bytes if ID is odd)" | §1.4.3 "Length: Only present when Type is odd."; "Value: A single varint encoded value when Type is even, otherwise a sequence of Length bytes." | §1.4.3 identical | **Works on draft 18 unchanged** |
| Key-Value-Pair wire form | not restated in LOC | §1.4.3 `Key-Value-Pair { Delta Type (vi64), [Length (vi64),] Value (..) }`; "Key-Value-Pairs encode a Type value as a delta from the previous Type value, or from 0 if there is no previous Type value."; "The maximum length of a value is 2^16-1 bytes." | §1.4.3 identical figure and text | **Same in 18 and 19.** Types are delta-encoded, so properties are written in ascending Type order; parity is of the Type, not of the delta. A Video Config value is limited to 65535 bytes |
| Object Properties container | §2.2 figure "MOQ Header Properties" | §11.2.1.2 `Object Properties { Properties Length (vi64), Properties (..), }`; §11.4.2 "The PROPERTIES bit (0x01) indicates when the Properties field is present in all Objects in this Subgroup"; §11.4.4 flag 0x20 "Properties field is present" | same (diff shows only a sentence reordering in §11.4.2) | **Works on draft 18 unchanged** |
| Track-scope properties (LOC-04 table: TIMESCALE, VIDEO_CONFIG, AUDIO_CONFIG "Track, Object") | §6.1 Table 1 | §2.5 "Track Properties always appear as the final field in the messages that carry them"; SUBSCRIBE_OK §10.8 and FETCH_OK §10.13 have "Track Properties (..)"; §2.5.1 names "PUBLISH, SUBSCRIBE_OK, or FETCH_OK" | same | **Works on draft 18 unchanged** (the carrier exists). LOC-04 does not say when a property goes on the Track and when on the Object: see "Unclear" |
| "Immutable Properties" | §3.1.2 "SHOULD be encoded as Immutable Properties as defined in [MoQTransport]"; §5.2 "IMMUTABLE_PROPERTIES mechanism" | §12.7 "Immutable Properties (Property Type 0xB) is a Track or Object Property that contains a sequence of Key-Value-Pairs"; §15.8 "0x0B IMMUTABLE_PROPERTIES Track, Object" | §12.7 identical | **Works on draft 18 unchanged.** LOC-04 §3.1.2 "This specification does not define any Immutable Properties" |
| "Private Properties" | §3.1.3 "[SecureObjects] defines a Private properties mechanism (type 0xA)" | not defined; appears only in the §12.7 figure ("Private Properties / App Payload" inside "Object Payload") | same figure | **Does not exist in MOQT 18 or 19 as a mechanism**; it is defined by SecureObjects. Not used in Phase 1 (no encryption) |
| PRIOR_GROUP_ID_GAP / PRIOR_OBJECT_ID_GAP | §5.4 "[MoQTransport] defines PRIOR_GROUP_ID_GAP and PRIOR_OBJECT_ID_GAP properties" | §12.8 "Prior Group ID Gap (Property Type 0x3C)"; §12.9 "Prior Object ID Gap (Property Type 0x3E)" | same numbers in §15.8 | **Works on draft 18 unchanged** |
| IANA registry LOC adds to | §6.1 "registers the following entries in the MOQ Properties registry established by [MoQTransport]" | §2.5 "Property types are registered in the IANA table 'MOQ Properties'."; §15.8 | §15.8 | **Exists in draft 18 under the same name.** Contents differ, see B.2 |
| Secure Objects Key ID (type 0x2), cipher suite 0x0004 | §3.1.1, §3.1.4 | not in MOQT (0x02 in MOQT-18 is OBJECT_DELIVERY_TIMEOUT, Track scope) | 0x02 is OBJECT_DELIVERY_TIMEOUT, "Track, Object" | Not used in Phase 1. Not examined further |

### B.2 The numeric IDs

IDs LOC-04 defines (§2.3.1 to §2.3.3, §6.1 Table 1): TIMESTAMP 0x10 (Object), TIMESCALE 0x08 (Track, Object), VIDEO_FRAME_MARKING 0x09 (Object), AUDIO_LEVEL 0x0C (Object), VIDEO_CONFIG 0x0D (Track, Object), AUDIO_CONFIG 0x0F (Track, Object).

Property types MOQT-18 defines itself (§15.8 Table 14): 0x02 OBJECT_DELIVERY_TIMEOUT (Track), 0x04 MAX_CACHE_DURATION (Track), 0x06 SUBGROUP_DELIVERY_TIMEOUT (Track), 0x0B IMMUTABLE_PROPERTIES (Track, Object), 0x0E DEFAULT_PUBLISHER_PRIORITY (Track), 0x22 DEFAULT_PUBLISHER_GROUP_ORDER (Track), 0x30 DYNAMIC_GROUPS (Track), 0x3C PRIOR_GROUP_ID_GAP (Object), 0x3E PRIOR_OBJECT_ID_GAP (Object), "0x7f * N + 0x9D Reserved for greasing". Ranges: §2.5.1 "Property types in the range 0x4000-0x7FFF are designated as Mandatory Track Properties."

**Collision check: none of LOC-04's six IDs (0x08, 0x09, 0x0C, 0x0D, 0x0F, 0x10) is a property type defined by MOQT-18 Table 14.** (0x08, 0x09 and 0x10 are also Message Parameter types in §15.7, EXPIRES, LARGEST_OBJECT and FORWARD, but that is a different number space.)

Disagreement with the provisional table. MOQT-18 §15.8 Table 15, "provisional registrations for other active drafts in the moq wg. These entries share the same Property Type space as the table above.":

| Name | MOQT-18 Table 15 | MOQT-19 Table 15 | LOC-04 | Same? |
|---|---|---|---|---|
| TIMESTAMP | 0x06, Object | 0x06, Object | **0x10**, Object | **differs** |
| TIMESCALE | 0x08, Track, Object | 0x08, Track, Object | 0x08, Track, Object | same |
| VIDEO_FRAME_MARKING | 0x0A, Object (even: a varint value) | 0x0A, Object | **0x09**, Object (odd: length-prefixed; LOC-04 §2.3.2.2 "encoded with a length prefix") | **differs, and the encoding class differs** |
| AUDIO_LEVEL | 0x0C, Object | 0x0C, Object | 0x0C, Object | same |
| VIDEO_CONFIG | 0x0D, Object | 0x0D, Object | 0x0D, **Track, Object** | number same, scope differs |
| AUDIO_CONFIG | absent | absent | 0x0F, Track, Object | not in either MOQT table |

So MOQT-19, which LOC-04 references, prints a provisional table that LOC-04 does not match either. The provisional table itself reuses 0x06 for TIMESTAMP while Table 14 assigns 0x06 to SUBGROUP_DELIVERY_TIMEOUT. In MOQT-18 the two have different scopes (Track vs Object). In MOQT-19 the timeout becomes "Track, Object" (§12.1 "SUBGROUP_DELIVERY_TIMEOUT (Property Type 0x06) is a Track and Object Property"; App. A.1 "Delivery timeouts are both Track and Object Properties (#1476)"), so 0x06 on an Object is ambiguous there. That LOC-04 moved TIMESTAMP to 0x10 for this reason is my inference; no draft says so.

Unknown-property handling in MOQT-18 that makes any of these numbers safe through a relay: §2.5 "If a Relay does not support a Property, it MUST NOT be modified, MUST be forwarded, and MUST be cached with the Track or Object, unless it is a Mandatory Track Property". §15.8 "Endpoints MUST ignore unknown Property types, skipping them using the length field." (MOQT-19 rewords: "skipping them according to the Key-Value-Pair encoding; odd types use their length field, even types are skipped by parsing a varint value.")

A relay that *implements* MOQT-18's SUBGROUP_DELIVERY_TIMEOUT must not interpret an Object-scope 0x06: irrelevant for LOC-04 (0x10), relevant if anyone sends LOC-02/-03 numbering.

Conclusion for IDs: **works on draft 18 unchanged on the wire; "does not exist in draft 18" as a registration** (LOC-04's 0x10 and 0x09 and 0x0F are in no MOQT table). Interop with another implementation depends on which LOC revision that implementation follows.

### B.3 Property IDs across LOC revisions

| Property | LOC-00 / -01 (term: "LOC Header Extensions", "varint") | LOC-02 (MOQT-17) | LOC-03 (MOQT-18) | LOC-04 (MOQT-19) |
|---|---|---|---|---|
| Timestamp (-01: capture timestamp) | 2 | 0x06 | body §2.3.1.1 "ID: 0x0A"; IANA table §6.1 "0x06 TIMESTAMP" (**the draft contradicts itself**) | 0x10 |
| Timescale | absent | 0x08 | 0x08 | 0x08 |
| Video Config | 13 | 13 | 0x0D | 0x0D |
| Video Frame Marking | 4 | 4 ("Length: Varies (1-4 bytes)" although 4 is even) | 0x09 | 0x09 |
| Audio Level | 6 | 6 (same number as Timestamp 0x06 in the same draft) | 0x0C | 0x0C |
| Audio Config | absent | absent | 0x0F | 0x0F |

(LOC-00 IDs were not extracted; the row label "LOC-00 / -01" reflects LOC-01 only: "ID: 2", "ID: 13", "ID: 4", "ID: 6".)

Note that LOC-03's body value 0x0A for Timestamp is the number MOQT-18's provisional table gives to VIDEO_FRAME_MARKING.

### B.4 Differences between MOQT-18 and MOQT-19 (only what the two texts show)

| Area | MOQT-18 | MOQT-19 | Evidence |
|---|---|---|---|
| Control message type numbers | SETUP 0x2F00, GOAWAY 0x10, SUBSCRIBE 0x3, SUBSCRIBE_OK 0x4, PUBLISH 0x1D, PUBLISH_OK 0x1E, PUBLISH_DONE 0xB, FETCH 0x16, FETCH_OK 0x18, TRACK_STATUS 0xD, PUBLISH_NAMESPACE 0x6, SUBSCRIBE_NAMESPACE 0x50, SUBSCRIBE_TRACKS 0x51, NAMESPACE 0x8, NAMESPACE_DONE 0xE, REQUEST_UPDATE 0x2, REQUEST_OK 0x7, REQUEST_ERROR 0x5 | same numbers | §10 Table 5 in 18; §10 table in 19 (lines 3216 to 3260) |
| 0xF | PUBLISH_BLOCKED (§10.20) | PUBLISH_SKIPPED (§10.20) | 19 App. A.1 "Rename PUBLISH_BLOCKED to PUBLISH_SKIPPED (#1779)" |
| Subscription filters | one "SUBSCRIPTION FILTER Parameter" 0x21 (§10.2.9); §5.1.2 "Subscription Filters" | 0x21 is "LOCATION_FILTER" (§10.2.9); new §5.1.3 "Range Filters", §5.1.4 "Combining Filters"; new parameters §10.2.10 SUBGROUP FILTER, §10.2.11 OBJECTID FILTER, §10.2.12 PRIORITY FILTER, §10.2.13 OBJECT PROPERTY FILTER, §10.2.14 TRACK PROPERTY FILTER | TOCs; 19 App. A.1 "Add Range Filters that can filter Objects from Subscriptions and SUBSCRIBE_TRACKS (#1765)" |
| Subscriptions per Track | §5.1 "an endpoint can have at most one subscription to a Track acting as the publisher and at most one acting as a subscriber" | changed | 19 App. A.1 "Allow multiple concurrent subscriptions per Track (#1775)" (the new §5.1 text was not read) |
| GOAWAY | has Request ID | removed | 19 App. A.1 "Remove Request ID from GOAWAY (#1623)" |
| Setup | none | new option and error | 19 App. A.1 "Add MAX_REQUEST_UPDATES Setup Option and TOO_MANY_REQUEST_UPDATES error (#1613)" |
| Request stream closure | §3.3.2 Request Cancellation and Rejection | new §3.3.2 "Graceful Request Stream Closure"; cancellation is §3.3.3 | TOCs; 19 App. A.1 "Define FIN vs. RST/STOP_SENDING semantics on request streams (#1698)" |
| Delivery timeout properties 0x02, 0x06 | Track scope | "Track, Object" | §12.1, §12.2, §15.8 of each |
| Object Status | §11.2.1.1 "Any object with a status code other than zero MUST have an empty payload." | "An Object MUST have an empty payload unless its Object Status value is registered as permitting a payload in the Object Status registry (Section 15.9)."; new §15.9 registry | diff |
| FETCH, FETCH_OK, FETCH_HEADER, Fetch Object Fields, End of Range | as in E | **same wire format and same range rules**; one sentence gains "unless filters were requested" | diff of §10.12 to §10.13 and §11.4.4: no other change |
| Subgroup header / Subgroup Object Fields | §11.4.2 | same; type notation "(i)" becomes "(vi64)"; "next Object" rule gains filter exceptions | diff |
| Relay reordering | §9.4 "A relay MUST NOT reorder or drop objects received on a multi-object stream when forwarding to subscribers, unless it has application specific information." | exception removed | 19 App. A.1 "Remove relay exception for reordering or dropping objects (#1762)" (new text not read) |
| Reserved application range (1 byte) | §2.5 and §12 say "0x38 to 0x3F", §15.8 says "0x78 to 0x7F" (the draft contradicts itself; 0x3C and 0x3E are defined in §12.8, §12.9) | §2.5, §12 and §15.8 all say 0x78 to 0x7F | `rg` |
| vi64, Key-Value-Pair, Location | §1.4.1 to §1.4.3 | identical | read both |

No message type was added, removed or renumbered between 18 and 19 that I can show; one was renamed.

---

## C. Map of MSF-01

Classification for Phase 1 (one publisher, one LOC video track, one catalog track; no audio, encryption, timelines, multiple qualities, logs, metrics, authorization):

- **(a)** unavoidable
- **(b)** does not apply; the unused feature is named
- **(c)** needs a decision

Each normative sentence has a number `[n]` used in the count.

### C.1 Section by section

| § | Specifies | MUST / MUST NOT / REQUIRED sentences (verbatim) | Class |
|---|---|---|---|
| 1 Introduction | Scope statement; "This document describes version 1 of the streaming format." | none | - |
| 2 Conventions | BCP 14; "uses the conventions detailed in Section 1.3 of [RFC9000] when describing the binary encoding" | none | - |
| 3 Scope | Features in scope (informative list) | none. "An MOQT relay can process the announcement as well as cache and propagate the tracks" (lower case) | - |
| 4 Media packaging | "MSF delivers LOC [LOC] packaged media bitstreams." | none | - |
| 4.1 LOC packaging | One chunk per Object; GOP per Group | [1] "Samples that belong to the same Group of Pictures (GOP) MUST be placed within the same MOQT Group." [2] "When LOC packaging is used for a track, the catalog packaging attribute (Section 5.2.4) MUST be present and it MUST be populated with a value of "loc"." | (a), (a) |
| 4.2 Time-alignment | Alternate-group alignment | [3] "Tracks advertised in the catalog as belonging to a common alternate group MUST be time-aligned." | (b) alternate groups |
| 4.3 Content protection | Secure Objects | none | (b) |
| 4.3.1 Encryption scheme signaling | catalog fields | [4] "When the encryptionScheme field is present in a track definition, subscribers MUST decrypt the object payload using the specified scheme before processing." | (b) encryption |
| 4.3.2 Key management | out of scope | [5] "Publishers and subscribers MUST use the same key management system and agree on the keyId scope semantics for interoperable operation." | (b) encryption |
| 4.3.3 Recommended scheme | moq-secure-objects | [6] "Implementations supporting content encryption MUST implement the "moq-secure-objects" scheme" [7] "The cipherSuite field MUST be present and set to a supported cipher suite value" [8] "The keyId field MUST be present to identify the key material" [9] "The trackBaseKey field MUST be present to provide the base key material" | (b) encryption ×4 |
| 4.3.4 Encrypted object structure | what stays plaintext | none | (b) |
| 5 Catalog | The catalog track, its objects, how to read it | [10] "The catalog track MUST have a case-sensitive Track Name of "catalog"." [11] "The first catalog object published within a new group MUST be independent and MUST provide a complete catalog that does not require any prior catalog object for interpretation." [12] "Any catalog updates that precede the first Object of the latest Group MUST be ignored." [13] "Each catalog update MUST be mapped to an MOQT Object." [14] "All catalog updates, both independent and delta, MUST be mapped to MOQT sub-group 0." [15] "The first Object (with Object ID 0) in any Group in a catalog track MUST hold an independent copy of the catalog." [16] "All subsequent Objects within that Group (i.e Objects IDs >= 1) MUST hold a delta update." [17] "As soon as an independent update is produced, it MUST be placed at the start of a new Group." [18] "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining FETCH (offset = 0) in order to obtain the latest complete catalog along with all subsequent catalog objects, including delta updates, that follow." [19] "At a minimum, a catalog MUST provide all mandatory fields." [20] "Custom field names MUST NOT collide with field names described in this draft." [21] "A parser MUST ignore fields it does not understand." | [10] to [13], [15], [17], [19] to [21] (a). [14] (c): conflicts with §6 [69]. [16] (c): whether the player implements delta processing although the publisher never sends one. [18] (a): forces FETCH in the relay or the publisher, see E |
| 5.1 Root Catalog Fields | Table 1 | none | - |
| 5.1.1 MSF version | `version`, "Required: Yes", String | [22] "A subscriber MUST NOT attempt to parse a catalog version which it does not understand." | (a). Value: (c), see C.3 |
| 5.1.2 Generated at | `generatedAt`, optional, ms since epoch | none (SHOULD NOT if isLive false) | optional |
| 5.1.3 Is Complete | `isComplete`, optional | [23] "This field MUST NOT be included if it is FALSE." [24] "This field MUST NOT be removed from a catalog once it has been added." | [23] (a), met by omission. [24] (c): only when the end of broadcast is signaled (§11.3) |
| 5.1.4 Tracks | `tracks`, "Required: Yes", Array | [25] the "Required: Yes" line | (a) |
| 5.1.5 Publish tracks | `publishTracks`, optional; tracks "to which the subscriber can publish data, such as logs, metrics, or other QoE data" | none | (b) publish tracks |
| 5.1.6 Delta update | `deltaUpdate`, optional | [26] "Each operation object MUST contain an "op" field indicating the operation type, and a "tracks" field" [27] (remove) "Each track object MUST include a Track Name Section 5.2.3 field, MAY include a Track Namespace Section 5.2.2 field, and MUST NOT hold any other fields." [28] (clone) "Each track object MUST include a Parent Name ... The cloned track inherits all attributes from the parent except the Track Name which MUST be new." | (b) delta updates (publisher side). Player side falls under [16] |
| 5.1.7 Initialization Data List | `initDataList`, optional; entries `id`, `type` ("inline": "Base64 [BASE64] encoded initialization data"), `data` | [29] "The Initialization Data List, if present, MUST be located after the tracks array in the root of the JSON catalog." | (b) unless codec config is put in the catalog: (c), see C.3 |
| 5.2 Track Object Fields | Table 3 | none | - |
| 5.2.1 Tracks object | "A track object is a JSON Object containing a collection of fields" | none | - |
| 5.2.2 Track namespace | `namespace`, optional, String | [30] "If it is not declared within a track, then each track MUST inherit the namespace of the catalog track." | (a) |
| 5.2.3 Track name | `name`, "Required: Yes", String | [31] "Within the catalog, track names MUST be unique per namespace." | (a) |
| 5.2.4 Packaging | `packaging`, "Required: Yes"; values loc, mediatimeline, eventtimeline, moqlog, moqmetrics | [32] the "Required: Yes" line | (a) |
| 5.2.5 Event timeline type | `eventType` | [33] "This field is required if the Section 5.2.4 value is "eventtimeline". This field MUST NOT be used if the packaging value is not "eventtimeline"." | (a), met by omission |
| 5.2.6 Track role | `role`, optional; reserved value "video" | none | optional |
| 5.2.7 Is Live | `isLive`, "Required: Yes", Boolean | [34] "A True value MUST never follow a False value." | (a) |
| 5.2.8 Target latency | `targetLatency`, optional, ms | [35] "If isLive is FALSE, this field MUST be ignored." [36] "All tracks belonging to the same render group MUST have identical target latencies." [37] "All tracks belonging to the same alternate group MUST have identical target latencies." [38] "This property MUST NOT be present if the buffers Section 5.2.9 property is present within a track definition." | (b) field not used; using it is optional |
| 5.2.9 Buffers | `buffers`, optional; keys target, min, max | [39] "Buffer is defined as the duration of media data that MUST be buffered before decoding commences." [40] "Unknown keys in the target buffer object MUST be ignored." [41] "If isLive is FALSE, this target buffer property MUST be ignored." [42] "All tracks belonging to the same render group MUST have identical target buffers." [43] "All tracks belonging to the same alternate group MUST have identical target buffers." [44] "This property MUST NOT be present if the target latency Section 5.2.8 property is present within a track definition." | (b) field not used |
| 5.2.10 Track label | `label` | none | optional |
| 5.2.11 Render group | `renderGroup` | none (SHOULD) | optional |
| 5.2.12 Alternate group | `altGroup` | [45] "Alternate tracks MUST have matching media time sequences." | (b) one quality |
| 5.2.13 Initialization reference | `initRef`: "A string pointing at the id field of an entry in the Initialization Data List" | none | optional |
| 5.2.14 Dependencies | `depends` | none | (b) |
| 5.2.15 Template | `template` (media timeline template) | none | (b) timelines |
| 5.2.16 Temporal ID, 5.2.17 Spatial ID | `temporalId`, `spatialId` | none | (b) layers |
| 5.2.18 Codec | `codec`, "Required: Conditional" | [46] "For LOC packaged content, the string codec registrations are defined in Sect 3 and Section 4 of [WEBCODECS-CODEC-REGISTRY]. This property MUST be specified for tracks which have an inherent codec associated with them (e.g., audio and video tracks)." | (a) |
| 5.2.19 Mimetype | `mimeType` | none | optional |
| 5.2.20 Framerate | `framerate` | none | optional |
| 5.2.21 Timescale | `timescale`: "The number of time units that pass per second." | none | optional; relation to LOC Timescale unstated |
| 5.2.22 Maximum Bitrate | `bitrate`, "Required: Conditional" | [47] "This property MUST be specified for audio and video tracks." | (a) |
| 5.2.23 Average Bitrate, 5.2.24 Maximum GOP Duration, 5.2.25 Maximum Group Duration | `avgBitrate`, `maxGopDuration`, `maxGroupDuration` | none | optional |
| 5.2.26 Width, 5.2.27 Height | `width`, `height` | none ("This property SHOULD accompany tracks which have a visual representation.") | SHOULD |
| 5.2.28 Audio sample rate | `samplerate` | [48] "This property MUST accompany tracks for which audio codecs are specified." | (b) no audio (Phase 2: a) |
| 5.2.29 Channel configuration | `channelConfig`, String | [49] "This property MUST accompany tracks for which audio codecs are specified." | (b) no audio (Phase 2: a) |
| 5.2.30 Display width, 5.2.31 Display height | `displayWidth`, `displayHeight` | none | optional |
| 5.2.32 Language | `lang` | [50] "The string MUST be one of the standard Tags for Identifying Languages as defined by [LANG]." | (b) field not used |
| 5.2.33 Parent name | `parentName` | [51] "This field MUST only be included inside a clone operation in a delta update" | (b) delta |
| 5.2.34 Parent namespace | `parentNamespace` | [52] same wording | (b) delta |
| 5.2.35 Track duration | `trackDuration` | [53] "This field MUST NOT be included if the isLive Section 5.2.7 field value is true." | (a), met by omission |
| 5.2.36 Connection URI | `connectionUri` (publish tracks) | [54] "When specified, the subscriber MUST establish a new MOQT connection to this URI for publishing the track data." [55] "The URI MUST be a valid MOQT endpoint URI" | (b) publish tracks |
| 5.2.37 Token | `token` | none | (b) |
| 5.2.38 Encryption scheme | `encryptionScheme`; "If this field is absent, the track content is unencrypted." | none | (b) |
| 5.2.39 Cipher suite | `cipherSuite` | [56] "This field MUST be present when encryptionScheme is specified." [57] "Implementations MUST support "aes-128-gcm-sha256"." | (b) encryption. [57] has no condition in its own sentence; read in context of the field |
| 5.2.40 Key ID | `keyId` | none | (b) |
| 5.2.41 Track Base Key | `trackBaseKey` | [58] "Publishers and subscribers MUST use matching trackBaseKey values for successful decryption." | (b) encryption |
| 5.2.42 Authorization Info | `authInfo` | [59] "Custom scheme names MUST use a unique naming convention" | (b) authorization |
| 5.2.43 Token Delivery via URI | example | none | (b) |
| 5.2.44 Accessibility | `accessibility` | none ("A required 'scheme' field", "A required 'value' field", lower case) | (b) |
| 5.3 Delta updates | Rules for deltas | [60] "A delta update MUST include the Delta Update Section 5.1.6 field with at least one operation." [61] "It MUST NOT contain an instance of a Tracks Section 5.1.4 field or an MSF version Section 5.1.1 field." [62] "The tuple of Track Namespace and Track Name defines a fixed set of Track attributes which MUST NOT be modified after being declared. To modify any attribute, a new track with a different Namespace\|Name tuple is created by Adding or Cloning and then the old track is removed." | [60], [61] (b) delta. [62] **(c)**: it is stated under delta rules but is worded for any track. If the browser encoder changes resolution, codec string or bitrate mid-broadcast, the declared `width`/`height`/`codec`/`bitrate` may no longer hold |
| 5.4 Variable Substitution | `%name%` variables from the URI fragment | none (MAY) | (b) |
| 5.4.1 Variable Syntax | | [63] "Variable names MUST consist of alphanumeric characters, hyphens, and underscores." [64] "The percent character (%) MUST NOT appear in catalog field values except as part of a variable reference." [65] "Variable values MUST consist only of alphanumeric characters, hyphens, underscores, and the at sign (@)." [66] "Special characters including commas, semicolons, quotes, ampersands, and other punctuation MUST NOT appear in variable values." | [64] (a): unconditional, applies to every catalog (no "%" in any value). Others (b) |
| 5.4.2 Variable Resolution | | [67] "Query parameters ... MUST NOT be used for variable substitution." | (b) |
| 5.5 Catalog Compression | "catalog objects MAY be compressed using the MSF_COMPRESSION property" | none | (b), but see §12 |
| 5.6, 5.6.1 to 5.6.16 Catalog Examples | "non-normative JSON examples". 5.6.13 is the termination catalog | [68] in 5.6.16: "The subscriber MUST replace %resourceId% with their actual resource identifier" | (b) publish tracks |
| 6 Media transmission | Objects to streams | [69] "Irrespective of the Section 4 in place, each MOQT Object MUST be mapped to a new MOQT Stream." | **(a) and (c)**: see C.3 "Group/Object/Subgroup" |
| 6.1 Group numbering | | [70] "Group IDs for a track MUST be unique and MUST increase monotonically." [71] "When a publisher restarts (e.g., after connectivity loss or encoder restart), it MUST ensure the new starting Group ID is greater than any previously published Group ID for that track." (Also: "each subsequent Group ID SHOULD increase by 1"; "One approach is to use the current wall clock time in milliseconds since the Unix epoch as the starting Group ID."; "it SHOULD signal the gap using the MOQT Prior Group ID Gap Extension header") | [70] (a). [71] (c): what the browser publisher does on reload (starting Group ID) |
| 6.2 Object Numbering | | [72] "Object ID MUST be zero for the first Object within a Group and then MUST increase monotonically by one within that Group." | (a) |
| 7 Media Timeline track | "Media timeline tracks are optional." | none | (b) |
| 7.1, 7.1.1 payload | JSON records | [73] "This value MUST match the media presentation timestamp ... of the first media sample in the referenced Object." | (b) timelines |
| 7.2 Catalog requirements | | [74] "A media timeline track MUST carry a 'type' identifier in the Catalog with a value of "mediatimeline"." [75] "A media timeline track MUST carry a 'depends' attribute" [76] "The mime-type of a media timeline track MUST be specified as "application/json"." | (b) |
| 7.3 Updating | | [77] "The publisher MUST publish an independent media timeline in the first MOQT Object of each MOQT Group of a media timeline track." [78] "An independent media timeline object MUST contain all media timeline records accumulated" | (b) |
| 7.4 Template | | [79] "For content with variable segment durations, the explicit media timeline track (Section 7.1) MUST be used instead." | (b) |
| 7.4.1 Template Format | | [80] "All six values are mandatory and MUST appear in the specified order." | (b) |
| 7.4.2 Template Immutability | | [81] "Publishers MUST NOT change the template values for a track after the first Object has been published." | (b) |
| 8 Event Timeline track | "Event timeline tracks are optional." | [82] "each event record MUST contain a reference to one of Media PTS, wallclock time or MOQT Location" | (b) |
| 8.1 Data format | | [83] "An index reference, which MUST be either 't' for wallclock time, 'l' for Location or 'm' for Media PTS." | (b) |
| 8.2 Catalog requirements | | [84] "An event timeline track MUST carry: ..." | (b) |
| 8.3 Updating | | [85] "The publisher MUST publish an independent event timeline in the first MOQT Object of each MOQT Group" [86] "An independent event timeline object MUST contain all event timeline records" | (b) |
| 8.4, 8.4.1, 8.4.2 examples | | none | (b) |
| 9, 9.1 Log track | | [87] "Implementations MUST follow the object payload format specified in Section 4 of [MOQLOG]." | (b) logs |
| 9.2 | contains "TODO: Finalize on track naming" | [88] "Log tracks MUST use the namespace and track name format defined in Section 3 of [MOQLOG]." | (b) |
| 9.3 | Group/Object IDs for logs | none | (b) |
| 9.4 | | [89] "A log track MUST be declared in the publishTracks array of the catalog with: ..." | (b) |
| 10, 10.1 Metrics track | | [90] "Implementations MUST follow the object payload format specified in Section 3 of [MOQMETRICS]." | (b) metrics |
| 10.2 | contains "TODO: Finalize the track naming." | [91] "Metrics tracks MUST use the namespace and track name format defined in Section 3 of [MOQMETRICS]." | (b) |
| 10.3 | | none | (b) |
| 10.4 | | [92] "A metrics track MUST be declared in the publishTracks array of the catalog with: ..." | (b) |
| 10.5 Well-known event timeline types | | none | (b) |
| 11 Workflow | | none | - |
| 11.1 URL construction | `moqt://authority/path?query#msf:<ns>--<name>&params` | [93] "If the query is absent, the ? separator MUST be omitted." [94] ABNF comment "MUST NOT contain '&' or '?'"; "'?' MUST be percent-encoded as %3F within this component" [95] "The track-identifier MUST be formatted as an MSF namespace-name string (see Section 11.1.2)." | (c): whether Phase 1 uses MSF URLs at all. MSF does not require a player to be started from an MSF URL |
| 11.1.1 Reserved fragment parameters | wallclock-range, mediatime-range, location-range, c4m, connection | [96] "The "." dot and "-" dash separators MUST be omitted when the second value is omitted." [97] ""q" indicates that a Native QUIC connection MUST be used." [98] ""wt" indicates that a WebTransport connection MUST be used." [99] "the client MUST process the union of those ranges" | (b) not used |
| 11.1.2 Namespace-Name String Encoding | "-" between namespace fields, "--" before the name | [100] "All other byte values (including hyphens and periods used as data) MUST be percent-encoded using a period (.) followed by two lowercase hexadecimal digits" | (c) with [93] to [95] |
| 11.1.3 Example URLs | | none | - |
| 11.2 Initiating a broadcast | | [101] "An MSF publisher MUST publish a catalog track object before publishing any media track objects." | (a) |
| 11.3 Ending a live broadcast | steps, see C.3 | none ("can deliver a deterministic signal ... by taking the following steps") | optional; (c) whether to do it |
| 11.4, 11.4.1, 11.4.2 Authorization | | none | (b) |
| 11.4.3 Presenting Authorization | | [102] "When a token is associated with a track, it MUST be included in ALL control messages that accept the AUTHORIZATION TOKEN parameter" | (b) |
| 11.4.4 Authorization failures | | none | (b) |
| 12 MSF Properties | "MSF defines MOQT Track Properties and Object Properties" | none | - |
| 12.1 Compression Signaling | GZIP for catalogs and timelines | [103] "Publishers MUST use one of the following approaches" [104] "A publisher MUST NOT use both mechanisms on the same track." [105] "If the track property is set, then every object in the track is compressed and the object property MUST NOT be present." [106] "If compression varies between objects, then the track property MUST NOT be set and the object property MUST be used on each compressed object." [107] "All MSF implementations MUST support both uncompressed payloads (value 0 or property absent) and GZIP compressed payloads (value 1)." | [103] to [106] (b) publisher does not compress. [107] **(c)**: unconditional, but the Property ID is "TBD" (§14.3), so a player cannot recognise the property |
| 12.1.1 Track Property | | [108] "Publishers MUST include the MSF_COMPRESSION track property in the PUBLISH message (publisher-initiated flow) or SUBSCRIBE_OK (subscriber-initiated flow)." [109] "Subscribers MUST check for this property in the corresponding message before processing the track payload." [110] "If the property is absent, and no per-object compression is signaled, the subscriber MUST treat the payload as uncompressed." [111] "If the property is present with a value the subscriber does not support, the subscriber MUST NOT attempt to process the payload" | [108] (b). [110] (a). [109], [111] (c) with [107] |
| 12.1.2 Object Property | | [112] "Publishers MUST include the MSF_COMPRESSION object property on each compressed object." [113] "Subscribers MUST check for this property on each received object before processing its payload." [114] "If the property is absent on an object, the subscriber MUST treat that object's payload as uncompressed." [115] "the subscriber MUST NOT attempt to process that object" | [112] (b). [114] (a). [113], [115] (c) with [107] |
| 13 Security Considerations | "ToDo" | none | - |
| 14, 14.1 to 14.4 IANA | "msf" fragment type; event timeline types; MSF_COMPRESSION "Property ID: TBD"; compression algorithm values 0 None, 1 GZIP | none | - |
| 15 References | LOC-02, MOQT-18 normative | - | - |

### C.2 Count

By this numbering (one number per normative sentence or bullet; two "Required: Yes" lines without a MUST are counted as [25] and [32]):

| Class | Numbers | Count |
|---|---|---|
| (a) unavoidable | 1, 2, 10, 11, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 25, 30, 31, 32, 33, 34, 46, 47, 53, 64, 70, 72, 101, 110, 114 | **29** |
| (c) needs a decision | 14, 16, 24, 62, 69, 71, 93, 94, 95, 100, 107, 109, 111, 113, 115 | **15** |
| (b) feature not used | the remaining 71 | **71** |
| total | | 115 |

Of the 29 (a), five are met by leaving a field out ([20], [23], [33], [53], [64]).

Decisions behind the 15 (c), grouped:

1. **Stream mapping** ([14], [69]): one stream per Object, versus catalog Objects all in Subgroup 0.
2. **Delta processing in the player** ([16]): implement add/remove/clone although the XmoQ publisher never sends a delta.
3. **Attribute immutability** ([62]): fix the encoder settings for the life of a track, or rename the track when they change.
4. **Publisher restart** ([71]): how the browser picks a starting Group ID after a reload.
5. **End of broadcast** ([24], §11.3): whether to publish the `isComplete` catalog.
6. **MSF URLs** ([93], [94], [95], [100]): whether to accept `#msf:` URLs.
7. **MSF_COMPRESSION** ([107], [109], [111], [113], [115]): the ID is TBD.

### C.3 Answers to the specific questions

**Smallest catalog JSON for one H.264 LOC video track**

```json
{
  "version": "draft-01",
  "tracks": [
    {
      "name": "video",
      "packaging": "loc",
      "isLive": true,
      "codec": "avc1.42E01F",
      "bitrate": 1000000
    }
  ]
}
```

| Field | Required by |
|---|---|
| `version` | §5.1.1 "Required: Yes JSON Type: String". Value: §5.1.1 "For usage against IETF Internet-Draft releases, follow the convention of specifying the version as "draft-XX". For example "draft-03" refers to the -03 draft release." Every example in §5.6 writes `"version": "1"` instead; see "Unclear" |
| `tracks` | §5.1.4 "Required: Yes JSON Type: Array" |
| `name` | §5.2.3 "Required: Yes"; unique per namespace ([31]). The string "video" is my choice; MSF fixes only the catalog's name |
| `packaging: "loc"` | §5.2.4 "Required: Yes"; §4.1 "MUST be populated with a value of "loc"" |
| `isLive` | §5.2.7 "Required: Yes JSON Type: Boolean" |
| `codec` | §5.2.18 "MUST be specified for tracks which have an inherent codec associated with them (e.g., audio and video tracks)"; string from the WebCodecs Codec Registry. The value above is a placeholder; for VP9 it would be a `vp09...` string from the same registry |
| `bitrate` | §5.2.22 "This property MUST be specified for audio and video tracks." (maximum bitrate, bits per second; the number above is a placeholder) |

Not required but SHOULD: `width`, `height` (§5.2.26, §5.2.27). Optional and commonly present in the examples: `role: "video"`, `framerate`, `renderGroup`, `generatedAt`, `targetLatency`. `namespace` may be omitted ([30]). Note that example §5.6.14 shows a `"role": "video"` LOC track with neither `codec` nor `bitrate`; the examples are declared non-normative and that one contradicts [46] and [47].

**Catalog track name and namespace.** Name: [10] "MUST have a case-sensitive Track Name of "catalog"". Namespace: MSF sets no rule for the catalog's own namespace; media tracks inherit it when they omit `namespace` ([30]). §11.1.3 shows `...#msf:customer-livestream-123--catalog` as "Namespace: ('customer', 'livestream', '123')", "Track Name: 'catalog'".

**How a subscriber obtains the catalog.** [18]: "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining FETCH (offset = 0) in order to obtain the latest complete catalog along with all subsequent catalog objects, including delta updates, that follow." MSF does not say which of MOQT-18's two Joining Fetch types is meant, and MOQT-18 has no field named "offset" (the field is "Joining Start", §10.12.2). Only a **Relative Joining Fetch (Fetch Type 0x2) with Joining Start = 0** yields "the latest complete catalog": its Start Location is "{Joining Location.Group - Joining Start, 0}" (MOQT-18 §10.12.2.1). An Absolute Joining Fetch (0x3) with Joining Start = 0 starts at Group 0, the whole history. MSF also does not say which Subscription Filter the SUBSCRIBE carries. Details in E.3.

**Do media tracks also need FETCH.** MSF-01 has no sentence that requires FETCH, a Joining FETCH, or any particular filter for media tracks. The only FETCH requirement is [18], for the catalog. §11.1 says only "The client uses this identifier to initiate a SUBSCRIBE or FETCH command once the transport session is established."

**Group/Object/Subgroup mapping for LOC media.**

- Object: §4.1 "each EncodedAudioChunk or EncodedVideoChunk sample is placed in a separate MOQT Object".
- Group: [1] "Samples that belong to the same Group of Pictures (GOP) MUST be placed within the same MOQT Group." It does not say that a Group holds exactly one GOP, nor that a Group starts with a key frame. MOQT-18 §2.3 adds "Groups SHOULD be independently useful, so Objects within a Group SHOULD NOT depend on Objects in other Groups."
- Group IDs: [70], [71]; "each subsequent Group ID SHOULD increase by 1".
- Object IDs: [72], start at 0, step 1.
- Subgroup: MSF-01 says nothing about Subgroup IDs for media. For streams: [69] "each MOQT Object MUST be mapped to a new MOQT Stream".
- [69] against MOQT-18: §2.2 "Objects from the same Subgroup MUST NOT be sent on different streams, unless one of the streams was reset prematurely, or upstream conditions have forced objects from a Subgroup to be sent out of Object ID order." So one stream per Object means one Subgroup per Object. MOQT-18 offers an encoding for that (§11.4.2 SUBGROUP_ID_MODE 0b01: "the Subgroup ID is the Object ID of the first Object transmitted in this Subgroup").
- [69] against MSF's own [14]: catalog Objects of one Group (independent + deltas) "MUST be mapped to MOQT sub-group 0", which under MOQT-18 §2.2 puts them on one stream, while [69] says "Irrespective of the Section 4 in place" every Object gets a new stream. The two cannot both hold for a catalog Group with more than one Object. With a single static catalog Object per Group there is no conflict.
- [69] against LOC-04 §4.2 (example, not normative): "one unidirectional QUIC stream is setup to deliver all the encoded video chunks within a MOQT group".

**What goes in Object 0.** Catalog: [15] an independent catalog. Media: MSF does not say in so many words; [1] + [72] imply the first sample of the Group. LOC-04 §4.2 (example): "The first encoded video frame, MOQT Object with ObjectID 0, shall be the Independent (IDR) frame" (lower-case "shall").

**Timestamps / timescale.** MSF-01 requires no timestamp on media Objects. `timescale` is an optional catalog field (§5.2.21 "The number of time units that pass per second."); its relation to LOC's Timescale property is not stated. `targetLatency` (§5.2.8) is defined against "when content was encoded" in wallclock time, but no sentence obliges the publisher to carry that time. Timestamp MUSTs exist only for the optional media timeline ([73]).

**Initialization data for LOC.** MSF-01 has no field named `initData`. It has `initDataList` at the root (§5.1.7, entries `id`, `type: "inline"`, `data` = "Base64 [BASE64] encoded initialization data") and `initRef` on a track (§5.2.13). Both are optional. MSF-01 never says what the "initialization data" is for a LOC track, and no example in §5.6 uses `initDataList` or `initRef`. LOC-04 gives two places: in the payload (§2.1.1 "Parameter sets can be sent in the bitstream payload before key frames") or in the Video Config property (§2.1.2 "Parameter sets can be sent in headers before key frames, as described in the Video Config LOC Property"; §2.3.2.1 "Video codec configuration "extradata" ... which maps to the WebCodecs VideoDecoderConfig description property"). So: catalog, property, or payload are each permitted; none is mandated; whether catalog `initDataList` data and the Video Config property are the same bytes is not stated.

**What the receiver must do on a new catalog object.** [12] ignore updates older than Object 0 of the latest Group; [21] ignore unknown fields; [22] do not parse an unknown `version`; apply a delta's operations in order (§5.3 "Operations are applied sequentially in the order they appear in the deltaUpdate array"); [110]/[114] treat as uncompressed when no MSF_COMPRESSION property is present. MSF-01 does not say what a player does with its existing subscriptions when a track disappears from the catalog.

**Track naming rules.** Only: catalog is "catalog" ([10]); names unique per namespace ([31]); `namespace` and `name` are JSON Strings. How a JSON String `namespace` maps to MOQT's tuple of byte fields is not stated in §5.2.2 (the examples write "conference.example.com/conference123/alice"); §11.1.2 defines a hyphen form, but for URLs. Log and metrics names are fixed by other drafts ([88], [91]).

**Relays, caching, cache duration, max-age, delivery timeout.** MSF-01 places **no requirement** on relays or on MOQT properties/parameters such as MAX_CACHE_DURATION, the delivery timeouts, priorities or group order. `rg -i "timeout|max.?age|MAX_CACHE"` finds nothing. The only cache-related sentences are §3 "An MOQT relay can process the announcement as well as cache and propagate the tracks" and §5 "A catalog object SHOULD be published only when the availability of tracks changes, or after a period of time has passed such that the catalog object might fall out of cache in a delivery network." The introduction claims MSF "specifies prioritization strategies for real-time", but the document has no section that does.

**Timelines.** None is mandatory. §7 "Media timeline tracks are optional." §8 "Event timeline tracks are optional." MSF-01 has no "SAP timeline"; "SAP-types for media segments" appears only as an example of event timeline content (§8).

**Publish tracks.** §5.1.5: optional array of tracks "to which the subscriber can publish data, such as logs, metrics, or other QoE data". Not needed.

**Termination.** §11.3, no MUST: "an original publisher can deliver a deterministic signal to all subscribers that the broadcast is complete by taking the following steps: Send a SUBSCRIBE_DONE (See MOQT Sect 8.1.2) message for all active tracks using status code 0x2 Track Ended. ... If the live stream is being terminated permanently without conversion to VOD, then publish an independent catalog update which signals isComplete Section 5.1.3 as TRUE and which contains an empty Tracks Section 5.1.4 field." Example §5.6.13: `{"version": "1", "generatedAt": ..., "isComplete": true, "tracks": []}`. In MOQT-18 the message is PUBLISH_DONE (0xB, §10.11) with "TRACK_ENDED (0x2): The track is no longer being published."; "SUBSCRIBE_DONE" and "Sect 8.1.2" do not exist in MOQT-18. The order of the steps (PUBLISH_DONE on the catalog track before or after the final catalog Object) is not stated.

**MOQT parameters or filters for live-edge join.** None required by MSF-01 beyond [18] for the catalog. No mention of Next Group Start, Largest Object, NEW_GROUP_REQUEST, GROUP_ORDER, SUBSCRIBER_PRIORITY, or datagrams.

Stale cross-references in MSF-01 to MOQT-18 (for the reader of both): §5.2.2/§5.2.3 "See section 2.3 of [MoQTransport]" (track naming is §2.4.1 in MOQT-18; §2.3 is Groups); §11.3 "SUBSCRIBE_DONE (See MOQT Sect 8.1.2)"; §14.1 "[MoQTransport] Sect 14.3" (MOQT URI Fragment Types is §15.3); §14.3 ""Track Properties" registry established by [MoQTransport] (Section 14.4)" (MOQT-18 has one "MOQ Properties" registry, §15.8).

---

## D. Map of LOC-04

### D.1 Every MUST

LOC-04 contains exactly two BCP 14 "MUST" sentences, both about encryption:

| § | Quote | Applies to an unencrypted video track? |
|---|---|---|
| 3.1.1 | "The Secure Object Key ID property (type 0x2 in [SecureObjects]) MUST be included as an immutable property to identify the keying material used for encryption." | no |
| 3.1.4 | "Implementations using LOC with Secure Objects MUST support the AES_128_GCM_SHA256_128 cipher suite (0x0004)." | no |

**No MUST of LOC-04 applies to one unencrypted video track.** There is no "REQUIRED" or upper-case "SHALL". §2.3 says "Each property must specify the following information in the IANA registry" (lower case, about registrations).

SHOULD / MAY that do apply:

| § | Quote |
|---|---|
| 2.1.3 | "A 4-byte length prefix can be sent before each NAL Unit, similar to "canonical" ("avc" or "hevc") formats. A length value of 1 SHOULD be interpreted as a start code rather than a length. The length is in network byte order, i.e. big endian, and SHOULD be 4 bytes long to disambiguate from start code prefixes. A length prefix less than 4 bytes long, which is uncommon, MAY be specified in the Video Config" |
| 2.1.4 | "A 4-byte start code can be sent before each NAL Unit, similar to "annexB" formats. The start code value is 1 in network byte order, i.e. big endian, and SHOULD be 4 bytes long to disambiguate from length prefixes. A 3-byte start code, which is uncommon, MAY be used if the track never uses length prefixes or any Video Config" |
| 3.1.2 | "LOC Properties that should be immutable but visible to relays SHOULD be encoded as Immutable Properties" (in the encryption section) |

### D.2 One EncodedVideoChunk to one Object

- Payload: §2 "The "internal data" in these chunks is used directly in this specification as the "LOC Payload" bitstream."; §2.2 "The LOC Payload is the "internal data" of an EncodedAudioChunk or EncodedVideoChunk."
- Layout: §2.2 "populate the MOQ Object Properties with the LOC Public Properties, and populate the MOQ Object Payload with LOC Private Properties followed by the LOC Payload"; figure: "LOC Private Properties + LOC Payload = all MOQ Object Payload".
- For Phase 1 (no Secure Objects, hence no Private Properties): Object Payload = the chunk's bytes; Object Properties = whichever LOC properties the publisher chooses.
- That one chunk is one Object is said by MSF-01 §4.1 ("placed in a separate MOQT Object") and by LOC-04's examples (§4.2 "each encoded video chunk is considered as input to MOQT Object payload"); LOC-04's normative part does not state it as a rule.

### D.3 Mandatory versus optional properties for video

All optional. §2.2 "The LOC Public and Private Properties carry optional metadata related to the Payload"; §2.3 "carry optional metadata for the corresponding LOC Payload".

| Property | ID, encoding | Mandatory? | Quote |
|---|---|---|---|
| Timestamp | 0x10, vi64 value | no | §2.3.1.1 "Timestamp of the encoded media frame encoded as vi64. The unit of the timestamp is determined by the Timescale property ... If no timescale property is present, the timestamp is interpreted as wall-clock time in microseconds since the Unix epoch." |
| Timescale | 0x08, vi64 value; scope Track, Object | no | §2.3.1.2 "The number of Timestamp units per second ... When this property is present, the Timestamp represents media time rather than wall-clock time. The epoch or anchor point for the timestamp is application-defined. If this property is not present, timestamps default to microseconds since Unix epoch." |
| Video Config | 0x0D, length-prefixed bytes; scope Track, Object | no | §2.1.2 "Parameter sets can be sent in headers before key frames, as described in the Video Config LOC Property" ("can", not "MUST"); §2.3.2.1 as quoted in C.3 |
| Video Frame Marking | 0x09, length 1 to 4 bytes | no | §2.3.2.2 "Flags for video frames which are independent, discardable, or base layer sync points, as well as temporal and spatial layer identification, as defined in [RFC9626], encoded with a length prefix." |

LOC-04 does not say on which Objects Video Config appears (every key frame, first key frame, only on change), nor whether it is to be sent as a Track Property, an Object Property, or both.

### D.4 H.264 `avc` versus Annex B

Both are allowed; nothing is required. §2.1: "the LOC Payload can use either the "canonical" format ("avc" or "hevc") often used in storage containers like MP4 / ISO BMFF, or the "annexB" format used in some non-MP4 applications. These formats differ in how they carry initialization and configuration information called parameter sets as well as how parts of a video frame are delimited with length prefixes or start codes." §2.1.1: "Parameter sets can be sent in the bitstream payload before key frames, similar to "annexB" formats. Newer "canonical" formats such as "avc3" and "hev1" codec strings also support parameter sets in the bitstream payload or outside it in "extradata" metadata headers." §2.1.2: "similar to the original "canonical" formats such as "avc1" and "hvc1" codec strings."

No signal tells the receiver which framing a track uses. The drafts offer only the SHOULD of §2.1.3 (a 4-byte value of 1 is a start code), the codec string in the catalog (`avc1` vs `avc3`, by the "similar to" sentences above), and the presence or absence of a Video Config.

### D.5 Groups, Subgroups, Object IDs

Only in §4 "Examples", and with lower-case verbs. §1.2: "Track, Group, Subgroup, Object ... are defined in [MoQTransport] and used here to refer to those aspects of the MOQT Object Model." §4.2: "The GroupID is incremented by 1 at IDR Frame boundaries. The ObjectID is increment by 1 for each encoded video frame, starting at 0 and resetting to 0 at the start of a new group. The first encoded video frame, MOQT Object with ObjectID 0, shall be the Independent (IDR) frame and the rest of the encoded video frames corresponds to dependent (delta) frames, organized in the decode order. When mapping to QUIC for sending, one unidirectional QUIC stream is setup to deliver all the encoded video chunks within a MOQT group."

By LOC-04 §1.1 ("when, and only when, they appear in all capitals"), "shall" here is not a BCP 14 requirement. **LOC-04 has no normative Group/Subgroup/Object ID rule; MSF-01 supplies the normative ones** ([1], [69], [70], [71], [72]), and its stream rule [69] differs from LOC-04's example.

LOC-04 §4.1 to §4.6 list catalog fields in a form that is not MSF JSON (for example "samplerate: 480000" for 48 kHz in §4.1); they are illustrations.

---

## E. MOQT draft 18: FETCH and the relay cache

### E.1 Names, numbers, streams (as draft 18 names them)

| Thing | Value | Section, quote |
|---|---|---|
| FETCH | control message type **0x16**, "Request, First" | §10 Table 5; §10.12 "A subscriber sends FETCH as the first message on a new bidi stream to a publisher to request a range of already published objects within a track." |
| Fetch Types | 0x1 Standalone Fetch, 0x2 Relative Joining Fetch, 0x3 Absolute Joining Fetch | §10.12 Table 6; "An endpoint that receives a Fetch Type other than 0x1, 0x2 or 0x3 MUST close the session with a PROTOCOL_VIOLATION." |
| FETCH message | `Type (vi64) = 0x16, Length (16), Request ID (vi64), Fetch Type (vi64), [Standalone (Standalone Fetch),] [Joining (Joining Fetch),] Number of Parameters (vi64), Parameters (..) ...` | §10.12.3 Figure 15 |
| Standalone Fetch | `Track Namespace (..), Track Name Length (vi64), Track Name (..), Start Location (Location), End Location (Location)`; "End Location: The end Location, plus 1. A Location.Object value of 0 means the entire group is requested." | §10.12.1 |
| Joining Fetch | `Joining Request ID (vi64), Joining Start (vi64)` | §10.12.2 |
| FETCH_OK | control message type **0x18**, on the same bidi stream: `Type (vi64) = 0x18, Length (16), End Of Track (8), End Location (Location), Number of Parameters (vi64), Parameters (..) ..., Track Properties (..)` | §10.13 Figure 16; "A publisher sends a FETCH_OK as the first message on the bidi stream in response to a successful fetch." |
| Failure | REQUEST_ERROR, type **0x5** | §10.12.3 "A publisher responds to a FETCH request with either a FETCH_OK or a REQUEST_ERROR message." |
| Data | one new unidirectional stream, stream type **0x05 FETCH_HEADER**: `FETCH_HEADER { Type (vi64) = 0x5, Request ID (vi64) }` | §3.4 Table 3; §11.4.4 Figure 26; §10.12.3 "The Objects in the response are delivered on a single unidirectional stream." |
| FETCH_CANCEL | **does not exist in draft 18.** Cancellation is by stream: §5.2 "the subscriber wishing to cancel the FETCH MAY send STOP_SENDING for the data stream as well as the bidi request stream. It MUST send STOP_SENDING for the bidi request stream."; §3.3.2 "Implementations SHOULD cancel requests by abruptly terminating any directions of a stream that are still open by resetting or sending STOP_SENDING." | |
| Parameters on FETCH | AUTHORIZATION_TOKEN (not read in detail), FILL_TIMEOUT 0x0A (§10.2.5 "MAY appear in a FETCH message"), SUBSCRIBER_PRIORITY 0x20 (§10.2.7), GROUP_ORDER 0x22 (§10.2.8 "If omitted from FETCH, the receiver uses Ascending (0x1).") | |

Fetch Object serialization (§11.4.4 Figure 27): `{ Serialization Flags (vi64), [Group ID Delta (vi64),] [Subgroup ID (vi64),] [Object ID Delta (vi64),] [Publisher Priority (8),] [Properties (..),] Object Payload Length (vi64), [Object Payload (..),] }`.

- Flags (§11.4.4.1): bits 0x03 Subgroup ID mode (0 zero, 1 same as prior, 2 prior plus one, 3 field present); 0x04 Object ID Delta present; 0x08 Group ID Delta present; 0x10 Priority present; 0x20 Properties present; 0x40 Datagram.
- "The first Object MUST include a Group ID Delta and Object ID Delta, and these values are the absolute Group ID and Object ID. If the first Object in the FETCH response uses a flag that references fields in the prior Object, the Subscriber MUST close the session with a PROTOCOL_VIOLATION."
- "If the Group Order is Ascending, the Group ID is the prior Object's Group ID plus the Group ID Delta + 1." "If the computed Group ID would be less than 0 or greater than 2^64-1, the Subscriber MUST close the Session with error 'PROTOCOL_VIOLATION'." Same for Object ID.
- "When encoding an Object with a Forwarding Preference of "Datagram" ... The publisher MUST SET bit 0x40 to '1'. When 0x40 is set, it SHOULD set the two least significant bits to zero and the subscriber MUST ignore the bits."
- No Object Status in FETCH: §11.2.1.1 "only present in objects that are delivered via a SUBSCRIPTION, and is absent in Objects delivered via a FETCH".
- Special values (§11.4.4 Table 7): **0x8C End of Non-Existent Range**, **0x10C End of Unknown Range**; "Any other value is a PROTOCOL_VIOLATION." §11.4.4.2: "the Group ID and Object ID fields are present. Subgroup ID, Priority and Properties are not present. All Objects with Locations between the last serialized Object, if any, and this Location, inclusive, either do not exist (when Serialization Flags is 0x8C) or are unknown (0x10C). A publisher SHOULD NOT use End of Non-Existent Range in a FETCH response except to split a range of Objects that will not be serialized into those that are known not to exist and those with unknown status."

Errors and resets:

| Name | Where | Number | Quote |
|---|---|---|---|
| INVALID_RANGE | REQUEST_ERROR | 0x11 | §10.6.2 "In response to SUBSCRIBE or FETCH, specified Filter or range of Locations cannot be satisfied." |
| INVALID_JOINING_REQUEST_ID | REQUEST_ERROR | 0x32 | "In response to a Joining FETCH, the referenced Request ID is not an Established Subscription." |
| DOES_NOT_EXIST | REQUEST_ERROR | 0x10 | "The track or namespace is not available at the publisher." |
| NOT_SUPPORTED | REQUEST_ERROR | 0x3 | "The endpoint does not support the type of request." |
| MALFORMED_TRACK | REQUEST_ERROR | 0x12 | "In response to a FETCH, a relay publisher detected the track was malformed" |
| TIMEOUT, UNAUTHORIZED, INTERNAL_ERROR, EXCESSIVE_LOAD, GOING_AWAY, REDIRECT, UNSUPPORTED_EXTENSION | REQUEST_ERROR | 0x2, 0x1, 0x0, 0x9, 0x6, 0x34, 0x33 | §15.10.2 |
| UNKNOWN_OBJECT_STATUS | stream reset code | 0x6 | §3.3.3 "In response to a FETCH, the publisher is unable to determine the status of the next Object in the requested range." |
| NO_OBJECTS | **does not exist in draft 18** | | App. A (older entry) "Remove NO_OBJECTS fetch error code (#1303)" |

### E.2 Every MUST about FETCH and caching

| § | Quote |
|---|---|
| 5.2 | "The publisher MUST send exactly one FETCH_OK or REQUEST_ERROR in response to a FETCH." |
| 5.2 | "It MUST send STOP_SENDING for the bidi request stream." (subscriber cancelling) |
| 5.2 | "The Publisher can destroy fetch state as soon as it has received a STOP_SENDING. It MUST reset the bidi request stream and unidirectional data stream associated with the FETCH." |
| 5.1 | "A publisher MUST save the Largest Location communicated in SUBSCRIBE_OK, PUBLISH or REQUEST_UPDATE_OK that changes the Forward State from 0 to 1. This value is called the Joining Location and can be used in a Joining FETCH (see Section 10.12.2) while the subscription is in the Established state." |
| 10.12.2 | "A Joining Fetch is only permitted when the associated subscription has Forward State 1; otherwise the publisher MUST respond with a REQUEST_ERROR with error code INVALID_RANGE." |
| 10.12.2 | "A publisher MUST process any pending REQUEST_UPDATE messages for the associated subscription before evaluating the current request." |
| 10.12.2 | "If no Objects have been published for the track the publisher MUST respond with a REQUEST_ERROR with error code INVALID_RANGE." |
| 10.12.2 | "If a publisher receives a Joining Fetch with a Request ID that does not correspond to a subscription in the same session in the Established or Pending (subscriber) states, it MUST return a REQUEST_ERROR with error code INVALID_JOINING_REQUEST_ID." |
| 10.12.3 | "If it encounters an object in the requested range that is not cached and has unknown status, the relay MUST pause subsequent delivery until it has confirmed the object's status upstream." |
| 10.12.3 | "End Location MUST specify the same or a larger Location than Start Location for Standalone and Absolute Joining Fetches." |
| 10.12.3 | "If no Objects have been published for the track or Start Location is greater than the Largest Object (Section 10.7) the publisher MUST return REQUEST_ERROR with error code INVALID_RANGE." |
| 10.12.3 | "A publisher MUST send fetched groups in the requested group order, either ascending or descending. Within each group, objects are sent in Object ID order; subgroup ID is not used for ordering." |
| 10.13 | "A publisher MAY send Objects in response to a FETCH before the FETCH_OK message is sent, but the FETCH_OK MUST NOT be sent until the End Location is known." |
| 10.13 | "If End Location is smaller than the Start Location in the corresponding FETCH the receiver MUST close the session with a PROTOCOL_VIOLATION." |
| 9.4 | "If a relay does not have sufficient information to send a FETCH_OK immediately in response to a FETCH, it MUST withhold sending FETCH_OK until it does." |
| 9.4 | "Relays MUST follow the constraints on LARGEST_OBJECT defined in Section 10.2.11." |
| 10.2.11 | "If Objects have been published on this Track the Publisher MUST include this parameter."; "A relay MUST set LARGEST_OBJECT to the largest of the following: 1. Any LARGEST_OBJECT value received from the upstream publisher in SUBSCRIBE_OK, PUBLISH, or REQUEST_UPDATE_OK 2. The largest Location of an Object received on an upstream subscription" |
| 10.2.5 | (FILL_TIMEOUT = 0) "the relay MUST NOT wait for upstream delivery and MUST report any unavailable Objects as Unknown gaps" |
| 9.1 | "An endpoint that receives a duplicate Object with a different Forwarding Preference, Subgroup ID, Priority or Payload MUST treat the track as Malformed." |
| 9.1 | "A cache MUST store all fields of an Object defined in Section 11.2.1, with the exception of any Object Properties (Section 11.2.1.2) that specify otherwise." |
| 2.5 | "If a Relay does not support a Property, it MUST NOT be modified, MUST be forwarded, and MUST be cached with the Track or Object"; "Relays MUST NOT attempt to merge sets of unknown properties received in different messages." |
| 9.6 | "A relay MUST include all Properties associated with a Track when sending any PUBLISH, SUBSCRIBE_OK, TRACK_STATUS_OK, or FETCH_OK, unless allowed by the property's specification" |
| 9.7 | "A relay MUST NOT modify Object fields when forwarding, except for Object Properties as specified in Section 2.5. A relay MUST treat the object payload as opaque. A relay MUST NOT combine, split, or otherwise modify object payloads." |
| 12.3 | "If present, the relay MUST NOT start forwarding any individual Object received through this subscription or fetch after the specified number of milliseconds has elapsed since the beginning of the Object was received." |
| 2.4.2 | "If a relay detects a Malformed Track, it MUST immediately terminate downstream subscriptions with PUBLISH_DONE and reset any fetch streams with Status Code MALFORMED_TRACK. Object(s) triggering Malformed Track status MUST NOT be cached." |
| 2.5.1 | "For FETCH_OK messages: the subscriber MUST cancel the fetch ... If the relay has already forwarded data on a fetch stream, it MUST reset the stream." (unknown Mandatory Track Property) |
| 11.4.4.1 | the three MUSTs quoted in E.1 |

What a relay is required to do, in words:

- **Must it have a cache: no.** §9.1 "Relays MAY cache Objects, but are not required to."
- **May it forward the FETCH upstream: yes.** §9.5 "When a Relay needs to make an upstream FETCH request, it determines the available publishers using the same matching rules as SUBSCRIBE. When more than one publisher is available, the Relay MAY send the FETCH to any of them." §10.2.5 "When a relay encounters Objects within the requested range that are not immediately available and have unknown status, it issues upstream FETCHes to retrieve them." For this to work with a cache-less relay, the original publisher (the browser) must itself answer FETCH.
- **What it is responsible for:** §10.12.3 "The publisher responding to a FETCH is responsible for delivering all available Objects in the requested range in the requested order". "Any gaps in the Group and Object IDs in the response stream indicate objects that do not exist." "If no Objects exist in the requested range, the publisher opens the unidirectional stream, sends the FETCH_HEADER (see Section 11.4.4) and closes the stream with a FIN."
- **If it has only part of the range:** §10.12.3 "A relay that has cached objects from the beginning of the range MAY start sending objects immediately in response to a FETCH." Then the pause MUST above. "If the upstream FETCH fails, the relay sends a REQUEST_ERROR and can reset the unidirectional stream." And: "If a Publisher receives a FETCH with a range that includes one or more Objects with unknown status (e.g. a Relay has temporarily lost contact with the Original Publisher and does not have the Object in cache), it can choose to reset the FETCH data stream with UNKNOWN_OBJECT_STATUS (Section 3.3.3), or indicate the range of unknown Objects and continue serving other known Objects." (the latter with 0x10C). It must not let a missing Object look like a gap, because a gap means "does not exist" and §2.1 says "The Object is known to not exist. This state is permanent."
- **May it refuse FETCH outright:** the error code exists (NOT_SUPPORTED 0x3, "The endpoint does not support the type of request"). No sentence in the sections read says a relay must support FETCH, and none says it may decline. MSF [18] then cannot be met by a subscriber through such a relay.
- **How long Objects are cached:** §12.3 "MAX_CACHE_DURATION (Property Type 0x04) is a Track Property. It is an integer expressing the number of milliseconds an Object can be served from a cache." "Once Objects have expired from cache, their state becomes unknown, and a relay that handles a downstream request that includes those Objects re-requests them." "If MAX_CACHE_DURATION is not sent by the publisher, the Objects can be cached until implementation constraints cause them to be evicted." No minimum retention is specified.
- **Delivery timeouts** (§8) concern subscriptions: "Each MOQT subscription has two timeout values associated with it"; §10.12.3 "The Object Forwarding Preference does not apply to fetches." Whether the timeouts apply to fetch streams is not stated in the sections read.
- **"Largest Location" / Largest Object:** §5.1.2 "The Largest Object is the Object with the largest Location (Section 1.4.2) in the Track from the perspective of the publisher processing the message. Largest Object updates when the first byte of an Object with a Location larger than the previous value is published or received through a subscription." FETCH_OK: "If the requested FETCH End Location was beyond the Largest known (possibly final) Object, End Location is {Largest.Group, Largest.Object + 1}". §10.12.3 "Objects larger than the Largest Object will not be retrieved by a FETCH."

### E.3 What a Relative Joining FETCH with Joining Start = 0 asks for

Sequence (§5.1.3): "To join a Track at a past Group, the subscriber sends a SUBSCRIBE, PUBLISH_OK or REQUEST_UPDATE with Forward State 1 followed by a Joining FETCH (see Section 10.12.2) for the intended start Group, which can be relative."

1. Subscriber opens a bidi stream and sends SUBSCRIBE (0x3) with Forward State 1 (FORWARD 0x10).
2. Publisher (relay) answers SUBSCRIBE_OK (0x4) with LARGEST_OBJECT (0x9) = L. The publisher saves L as the **Joining Location** (§5.1).
3. Subscriber opens another bidi stream and sends FETCH (0x16), Fetch Type 0x2, Joining Request ID = the SUBSCRIBE's Request ID, Joining Start = 0. It need not wait for step 2: §10.12.2 "if that subscription has not yet been established, the Publisher receiving the Joining Fetch buffers the pending Joining Fetch until either the Subscription is established or the request times out."
4. Range (§10.12.2.1): "For a Relative Joining Fetch, the publisher sets the Start Location to {Joining Location.Group - Joining Start, 0}." "The publisher receiving a Joining Fetch sets the End Location to {Joining Location.Group, Joining Location.Object + 1}"; "the last Object included in the Joining FETCH response is the Object at the Joining Location."

So with Joining Start = 0 the request is: **Objects {L.Group, 0} through {L.Group, L.Object} inclusive, where L is the Largest Location the publisher reported for that subscription.** One Group only: the Group that held the largest Object when the subscription was established, from its Object 0 up to that Object. It is relative to the subscription's Joining Location, not to the live edge at the time the FETCH arrives.

Purpose (§10.12.2.1): "so the Objects retrieved by the FETCH and SUBSCRIBE are contiguous and non-overlapping". That holds if the subscription starts at {L.Group, L.Object + 1}, which is the Largest Object filter (§5.1.2 "Largest Object (0x2): The filter Start Location is {Largest Object.Group, Largest Object.Object + 1}"). With Next Group Start (0x1, "{Largest Object.Group + 1, 0}") the remainder of Group L.Group is delivered by neither. Draft 18 §10.12.2 does not name the filter the SUBSCRIBE must use, and neither does MSF.

If nothing has been published yet: REQUEST_ERROR INVALID_RANGE (§10.12.2). The subscription itself stays; §5.1.2 "If no content has been delivered yet, the filter Start Location is {0, 0}."

Because Largest Object advances on "the first byte" of an Object, the Joining Location can name an Object the relay has not finished receiving.

### E.4 Minimum a relay must keep to answer that Joining FETCH

Draft 18 does not state a minimum; this is derived from E.2 and E.3.

**(i) Catalog track, latest Group = one independent catalog Object plus deltas.** The range is {G, 0} to {G, L.Object}. To answer from its own memory the relay keeps, for the Group with the largest Group ID seen, every Object from Object 0 to the largest Object, with all Object fields and all properties (§9.1 "A cache MUST store all fields of an Object defined in Section 11.2.1"; §2.5 unknown properties "MUST be cached with the Track or Object"). For a static catalog that is one Object. When a new Group begins, the old Group is no longer needed for joiners whose SUBSCRIBE_OK reports the new Group, but a subscriber whose Joining Location was taken in the old Group may send its FETCH afterwards; how long to keep the old Group for that case is not specified. If the relay lacks any Object in the range it cannot simply omit it; it has to fetch upstream, mark 0x10C, or reset with UNKNOWN_OBJECT_STATUS. Alternative with no relay cache: forward the FETCH to the publisher, which then must keep the catalog Group itself.

**(ii) Video track, one Group = one GOP.** MSF does not require a Joining FETCH for media, so the minimum demanded by MSF is zero. If the player does use Relative Joining Start = 0 on the video track, the range is the current GOP from its Object 0 (the key frame) to the Largest Object: the relay keeps the whole in-progress GOP (all frames, payloads and LOC properties), bounded by GOP duration times bitrate, and the same late-FETCH question about the previous Group applies. If the player instead subscribes with Next Group Start (§5.1.3 "To join a Track at the next Group, the subscriber sends a SUBSCRIBE with Filter Type Next Group Start"), the relay keeps nothing for joining and the viewer waits for the next GOP.

In a FETCH response the stream mapping of MSF [69] does not apply: all Objects of the range go on one FETCH stream, with Properties carried per Object (flag 0x20).

---

## Unclear in the drafts

1. **Which LOC.** MSF-01 normatively references LOC-02 (MOQT-17); LOC-04 references MOQT-19. Property IDs differ between LOC-02, -03 and -04 (B.3). "MSF-01 + LOC-04 on MOQT-18" is a combination no draft defines.
2. **LOC property IDs versus MOQT's provisional table.** MOQT-18 and MOQT-19 both list TIMESTAMP 0x06 and VIDEO_FRAME_MARKING 0x0A; LOC-04 says 0x10 and 0x09. LOC-03 says 0x0A in the body and 0x06 in its IANA table for Timestamp.
3. **MSF [69] "each MOQT Object MUST be mapped to a new MOQT Stream"** versus MSF [14] (all catalog Objects in sub-group 0), versus MOQT-18 §2.2 (same Subgroup, same stream), versus LOC-04 §4.2 (one stream per Group, example). MSF gives no Subgroup ID rule for media.
4. **"Joining FETCH (offset = 0)"**: Relative or Absolute is not stated; "offset" is not a draft 18 field name. Which Subscription Filter accompanies it is not stated.
5. **`version` value**: §5.1.1 says "draft-XX"; all examples use "1"; §1 says "This document describes version 1 of the streaming format."
6. **Initialization data for LOC**: what `initDataList` data contains for a LOC track, and its relation to the LOC Video Config property, are not stated. No example uses it.
7. **Video Config placement**: Track Property, Object Property or both; on which Objects. LOC-04 gives it "Track, Object" scope and says only "can be sent in headers before key frames".
8. **avc versus Annex B signalling**: no explicit indicator.
9. **Private Properties without encryption**: LOC-04 §2.2 puts "LOC Private Properties followed by the LOC Payload" in the Object Payload but gives no way to tell whether Private Properties are present when Secure Objects is not used. I assume none are present; the draft does not say so.
10. **`namespace` String to MOQT namespace tuple** mapping in the catalog (§5.2.2) is not defined.
11. **MSF_COMPRESSION**: [107] obliges all implementations to support GZIP, but the Property ID is "TBD"; the object-property registration is not even listed with an ID.
12. **[62] attribute immutability** is written under delta-update rules; whether it binds a publisher that never sends deltas is a matter of reading.
13. **Whether a Group must start with a key frame / hold exactly one GOP**: MSF [1] only says a GOP stays inside one Group.
14. **End of broadcast** (§11.3): refers to SUBSCRIBE_DONE and "MOQT Sect 8.1.2", which MOQT-18 does not have; order of PUBLISH_DONE and the final catalog is not given.
15. **Catalog `timescale`** versus LOC Timescale property: relation not stated.
16. **How long a relay must keep the previous Group** for a Joining FETCH whose Joining Location is in it: not specified in MOQT-18.
17. **Whether a relay may decline FETCH** (NOT_SUPPORTED) in MOQT-18: no sentence either way in the sections read.
18. **MOQT-18 reserved application range**: §2.5/§12 say 0x38 to 0x3F, §15.8 says 0x78 to 0x7F.
19. **MOQT-18 §15.8** "skipping them using the length field" does not describe even types, which have no length field.

## Not found

- In MSF-01: any requirement on MAX_CACHE_DURATION, delivery timeouts, priorities, group order, datagrams, NEW_GROUP_REQUEST, or any subscription filter; any FETCH requirement for media tracks; a field named `initData`; a "SAP timeline"; the prioritization strategies the introduction announces; a rule for what a player does when a track is removed from the catalog; a rule for the catalog track's namespace; security considerations (§13 is "ToDo").
- In LOC-04: any MUST that applies without encryption; a normative Group/Subgroup/Object ID rule; a statement that Timestamp is mandatory; a statement of which Objects carry Video Config.
- In MOQT-18: a FETCH_CANCEL message; a NO_OBJECTS error; a "MoQ Streaming Format" registry (LOC-03 §6.2 cites one at "[MoQTransport] Sect 8"); a definition of Private Properties; a minimum cache retention.
- A LOC revision that references MOQT-16.
- Not examined: SecureObjects, MOQLOG, MOQMETRICS, C4M, the WebCodecs Codec Registry, and the parts of MOQT-18/19 not listed at the top. Statements about what WebCodecs or Chromium produce are deliberately absent.
