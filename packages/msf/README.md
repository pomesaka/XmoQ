# @xmoq/msf

The MOQT Streaming Format, written from the text of `draft-ietf-moq-msf-01`: the catalog (§5) read and written as bytes, followed over a connection (`followCatalog`) and published over one (`createCatalogPublisher`). `src/core` is JSON and bytes, and its entry `@xmoq/msf/catalog` needs no types of a browser. Section numbers below are of that draft.

What a document is read as, and why, is shared with `crates/msf`: the table and the reasons are in [`fixtures/msf-01/`](../../fixtures/msf-01/README.md). How the catalog track is followed and published is at `catalogStateAfter`, `followCatalog`, `createCatalogPublisher`, `nextPublication` and `answerCatalogFetch`.

Not implemented: delta updates (told apart, not applied), compression (§12.1), timelines, encryption, authorization, URLs, log and metrics tracks, tracks the subscriber publishes (`publishTracks`, §5.1.5), initialization data (`initDataList`, §5.1.7; `initRef`), variable substitution (§5.4). LOC, the packaging of the media Objects, is not part of this package.

## Known gaps

- **`readCatalog` sees one payload, so the rules that hold between catalogs are not checked when a catalog is read**: `isLive` (§5.2.7), `isComplete` (§5.1.3) and the attributes of a track (§5.3). A publisher made with `createCatalogPublisher` keeps them for what it sends (`nextPublication`).
- **Nothing decrypts.** `encryptionScheme` of a track is read and handed on, and §4.3.1 has "subscribers MUST decrypt the object payload using the specified scheme before processing": whoever uses a catalog has to leave a track that has one alone. The other fields of encryption (`cipherSuite`, the keys) are not read.
- **An audio track without `samplerate` and `channelConfig` is read and written**, though §5.2.28 and §5.2.29 have a MUST for them: nothing here tells an audio track from another. The caller that writes an audio track gives both.
- **A publisher that is started again can begin below where it was** (§6.1, MUST). Its first Group is the time, and nothing of an earlier run is kept: when the clock went back between two runs, or the run before published more than one catalog a millisecond for as long as it ran, nothing here can tell.
- **The filter of a SUBSCRIBE for the catalog track is not looked at**: every subscription gets every catalog from the next on. That leaves the draft where the filter is AbsoluteStart or AbsoluteRange with a start that is still to come, and after the end of an AbsoluteRange (transport §5.1.2: "A publisher MUST NOT send objects from outside the requested range"). The own relay puts no filter on the subscription it makes.
- **What the draft asks of a publisher and nothing here checks or does**:
  - §5: "A catalog object SHOULD be published only when the availability of tracks changes, or after a period of time has passed such that the catalog object might fall out of cache in a delivery network." `update` with the catalog that is there publishes a new Group, and nothing publishes one again after a time.
  - §5.1.2, `generatedAt`: "This field SHOULD NOT be included if the isLive field is false." It is written whenever it is given.
  - §6.1: "If a publisher maintains state across a restart and knows the previous Group ID, it SHOULD signal the gap using the MOQT Prior Group ID Gap Extension header." The catalog Objects have no properties.
  - §11.3: that the last catalog of a broadcast has `isComplete` and no tracks is the caller's to write, and "Send a SUBSCRIBE_DONE ... for all active tracks using status code 0x2 Track Ended" is the application's for its media tracks.
  - §11.2: "An MSF publisher MUST publish a catalog track object before publishing any media track objects." The publisher of the catalog does not see the media tracks; the application keeps it.
- **A catalog that could not be written to a subscriber is not sent to it again.** Where the subscriber is a relay, its viewers are told of the Group before, a Joining FETCH from there gets an answer without the catalog, and they have none until the next update. Not run.
- **The catalog is there only while its publisher is.** The own relay keeps the latest Group of a track while the track has a publisher, and nothing after: when the publisher has gone, a viewer gets no catalog. And the relay has one subscription to the publisher for all its viewers, so a viewer whose FETCH fails gets nothing until the next update, as the catalog that is there is not sent on a subscription.
- `followCatalog` does not look at the track properties of the SUBSCRIBE_OK or at the properties of an Object, where compression is said (§12.1).
- **Delta updates are not applied.** A publisher that sends one (§5.3) cannot be followed past it; a reader has to wait for the next Group, whose first Object is a whole catalog (§5).
- **Compressed catalogs are not read**, which §12.1 asks for: "All MSF implementations MUST support both uncompressed payloads (value 0 or property absent) and GZIP compressed payloads (value 1)." The property that says a payload is compressed has no number in the draft (§14, `MSF_COMPRESSION`, "TBD"), so no two implementations can agree on one from the text. A compressed payload is `invalid` here.
- Where `initDataList` stands in the document (§5.1.7: "MUST be located after the tracks array") is not looked at; the field is not read.
- The JSON type of a field is all that is checked of its value: a `bitrate` of -1 and a `name` that is empty are read. An integer beyond 2^53 loses its last digits, as in every number of JSON read as a double.
