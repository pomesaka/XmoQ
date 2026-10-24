# Catalogs of draft-ietf-moq-msf-01

What `@xmoq/msf` reads a catalog as. `crates/msf` reads `catalog-moqlivemock.json` as well, for the fields a viewer chooses a track by, and of `catalogs.tsv` the rows on `samplerate` and `channelConfig` and the rows on `%` (§5.4.1); it does not read the rest of the table yet, and turns down less than `@xmoq/msf` does.

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

| Tracks | `role` | `codec` | Other fields |
|---|---|---|---|
| `video_400kbps_avc`, `video_600kbps_avc`, `video_900kbps_avc` | `video` | `avc3.4D401F` | `bitrate` (379454, 564523, 848032), `width` 1280, `height` 720, `framerate` 25, `renderGroup` 1, `altGroup` 1, `lang` `und` |
| `video_400kbps_hevc`, `video_600kbps_hevc`, `video_900kbps_hevc` | `video` | `hev1.1.6.L93.90` | the same fields |
| `video_400kbps_av1`, `video_600kbps_av1`, `video_900kbps_av1` | `video` | `av01.0.05M.08.0.110.02.02.02.0` | the same fields |
| `audio_monotonic_128kbps_aac`, `audio_scale_128kbps_aac` | `audio` | `mp4a.40.2` | `bitrate` 134376, `samplerate` 48000, `channelConfig` `"2"`, `renderGroup` 1, `altGroup` 2, `lang` `mon` and `sca` |
| `audio_monotonic_128kbps_opus`, `audio_scale_128kbps_opus` | `audio` | `opus` | the same, `bitrate` 135200 |

No track has `namespace`, `initRef` or `timescale`, and the catalog has no `initDataList`.

## `catalogs.tsv`: documents written by hand

**Nobody's implementation wrote these documents.** They were written by hand from the text of the draft, one for each thing the check of `@xmoq/msf` looks at, and the person who read the draft wrote down what each is read as. They are no check against another implementation; they keep two implementations of ours from reading one document in two ways.

75 rows, tab-separated, a newline after each:

| Column | |
|---|---|
| 1 | A name, unique in the table |
| 2 | What the document is read as: `catalog`, `delta-update`, `unsupported-version` or `invalid` |
| 3 | The document: the bytes of the payload, as UTF-8. No row has a tab or a newline in it |

**Not every row follows from the draft.** These are choices of `@xmoq/msf` where the draft, and JSON, leave one, written down so that a second implementation of ours makes the same one: the rows about `null`, about a number too large for a double (`1e999`), about half of a surrogate pair (refused in a field that is read; not noticed in a key or in a field that is not read), about a key that comes twice (`version-twice`: the later one is read), `percent-around-no-name` (§5.4.1 has variable names "consist of" alphanumeric characters, hyphens and underscores; a name of none is taken for no name, and "alphanumeric" for ASCII), `samplerate-that-is-no-whole-number` (kept as it is: §5.2.28 has a JSON Number and no more; both readers are tested for the value 44100.5), and `byte-order-mark`, whose document begins with the bytes `ef bb bf`. A parser that is not `JSON.parse` can differ on each of them, and the Rust reader that will read this table has to be made to agree, or the row changed for both.

Where each outcome comes from in the draft is in the [README of `@xmoq/msf`](../../packages/msf/README.md), "How the draft is read". The rows `an-array`, `a-string`, `no-json` and `null` are no JSON object, and the row `no-json` is no JSON at all.
