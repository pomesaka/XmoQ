# xmoq-meeting-tracks

The namespaces of a meeting (decided on 2026-10-25, [design](../../docs/scale-and-meetings.md)), as every side of it uses them. No input or output, no dependencies, and nothing to build in C.

| Item | What it is |
|---|---|
| `MEET` | The first two fields, `xmoq/meet` |
| `prefix` | The prefix of a meeting's broadcasts, `xmoq/meet/<room>`, which a reader asks for with one SUBSCRIBE_NAMESPACE |
| `participant` | The namespace a participant publishes under, `xmoq/meet/<room>/<id>` |
| `seat_subscribe` | The namespace of a seat's token to subscribe, `xmoq/meet/<room>/*` |
| `participant_of` | The participant a namespace is the broadcast of, or none |

What they give is the shared tables `namespaces.tsv` and `participant-of.tsv` of [`fixtures/meeting-presence/`](../../fixtures/meeting-presence/README.md); `@xmoq/meeting` (`packages/meeting/src/core/names.ts`) and the room (`seatFor` of `packages/room`) read them as well.
