// Spike: the page of codes.ts. Publishes a track with @xmoq/moqt and lets the driver
// reset a subgroup stream and close the session with a code.
import { connect } from "../../packages/moqt/src/index.ts";
const utf8 = new TextEncoder();
(globalThis as any).probe = async (url: string, hashHex: string) => {
  const hash = new Uint8Array(hashHex.match(/../g)!.map((h) => parseInt(h, 16)));
  const moq = await connect(url, { serverCertificateHashes: [{ algorithm: "sha-256", value: hash }] });
  const ns = [utf8.encode("xmoq"), utf8.encode("probe")];
  await moq.publishNamespace(ns);
  const track = await moq.publish({ trackNamespace: ns, trackName: utf8.encode("object"), trackAlias: 1n, parameters: { forward: true } });
  (globalThis as any).resetWith = async (code: number) => {
    const sub = await track.openSubgroup({ groupId: 0n, subgroupId: { mode: "Explicit", value: 0n }, publisherPriority: 0, properties: false, endOfGroup: false });
    await sub.write({ objectId: 0n, payload: utf8.encode("x") });
    await new Promise((r) => setTimeout(r, 300));
    await sub.reset(code);
    return "reset";
  };
  (globalThis as any).closeWith = (code: number, reason: string) => { moq.close(code, reason); return moq.closed.then((c) => JSON.stringify(c)); };
  return "published";
};
