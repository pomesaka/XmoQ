# Bytes for draft 18 that a peer's library wrote, and no wire carried

Messages another implementation's library serialized when it was asked to in a script, for encodings none of its clients send. **They were never on a wire**: bytes that were are in [`fixtures/moqt-18/`](../moqt-18/README.md), and bytes written by hand from the draft in [`fixtures/moqt-18-by-hand/`](../moqt-18-by-hand/README.md). The three are kept apart because a test of each implementation holds that every file of a directory is what its README says.

Both implementations read every file here and check that they decode it to the value below and encode that value to the same bytes.

Variable-length integers are those of §1.4.1: `80 fa` is 250 and `81 f4` is 500.

| File | Written by | Decodes to |
|---|---|---|
| `subscribe-delivery-timeouts-moqtail-ts` | moqtail npm 0.13.0, `Subscribe.newLatestObject(0n, FullTrackName.tryNew("xmoq/phase0", "object"), [new ObjectDeliveryTimeout(250n), new SubgroupDeliveryTimeout(500n)]).serialize()`, run with Bun on 2026-10-12 | SUBSCRIBE, Request ID 0, (`xmoq`, `phase0`) / `object`, OBJECT_DELIVERY_TIMEOUT = 250, SUBGROUP_DELIVERY_TIMEOUT = 500, SUBSCRIPTION_FILTER = Largest Object |
| `subscribe-subgroup-delivery-timeout-moqtail-ts` | The same with `[new SubgroupDeliveryTimeout(500n)]` | The same without OBJECT_DELIVERY_TIMEOUT |
| `request-update-subgroup-delivery-timeout-moqtail-ts` | moqtail npm 0.13.0, `new RequestUpdate(2n, [new SubgroupDeliveryTimeout(250n)]).serialize()`, run with Bun on 2026-10-12 | REQUEST_UPDATE, Request ID 2, SUBGROUP_DELIVERY_TIMEOUT = 250 |
| `request-update-no-subgroup-delivery-timeout-moqtail-ts` | The same with `new SubgroupDeliveryTimeout(0n)` | REQUEST_UPDATE, Request ID 2, SUBGROUP_DELIVERY_TIMEOUT = 0 (§8: no timeout) |
| `goaway-new-session-uri-moqtail-ts` | moqtail npm 0.13.0, `new GoAway("https://relay.example:4443/", 5000n, 3n).serialize()`, run with Bun on 2026-10-24 | GOAWAY, New Session URI `https://relay.example:4443/` (27 bytes), Timeout 5000, Request ID 3 |
| `goaway-same-uri-moqtail-ts` | The same with `new GoAway(undefined, 0n, 1n)` | GOAWAY, an empty New Session URI (the current URI is reused), Timeout 0, Request ID 1 |
| `goaway-request-stream-moqtail-ts` | The same with `new GoAway("https://relay.example:4443/", 2000n)`, which leaves the Request ID out, as on a request stream | GOAWAY, New Session URI `https://relay.example:4443/`, Timeout 2000, no Request ID |

The parameters come out in ascending order of their types whatever order the library is given them in (seen with the two swapped), as §10.2 asks.
