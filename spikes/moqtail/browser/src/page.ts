// Phase 0 spike: the browser side of one MOQT draft-18 Object transfer through a
// moqtail relay. `run.mjs` drives this page through `window.xmoq`.

import {
  FilterType,
  FullTrackName,
  GroupOrder,
  LiveTrackSource,
  Location,
  MOQtailClient,
  MoqtObject,
  ObjectForwardingPreference,
  RequestError,
  Tuple,
} from 'moqtail'

type ReceivedObject = { group: string; object: string; payloadHex: string }

type XmoqPage = {
  /** `certSha256Hex` is null when Chromium trusts the relay through launch arguments. */
  connect(relayUrl: string, certSha256Hex: string | null): Promise<void>
  publishTrack(namespace: string, trackName: string): Promise<string>
  sendObject(payloadHex: string): void
  subscribe(namespace: string, trackName: string): Promise<string>
  firstObject(): Promise<ReceivedObject>
  disconnect(): Promise<void>
}

declare global {
  interface Window {
    xmoq: XmoqPage
  }
}

function fromHex(hex: string): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(hex.length / 2)
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return bytes
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('')
}

let client: MOQtailClient | null = null
let published: { fullTrackName: FullTrackName; controller: ReadableStreamDefaultController<MoqtObject> } | null = null
let received: Promise<ReceivedObject> | null = null

function connected(): MOQtailClient {
  if (!client) throw new Error('connect() has not completed')
  return client
}

window.xmoq = {
  async connect(relayUrl, certSha256Hex) {
    client = await MOQtailClient.new({
      url: relayUrl,
      // WHY serverCertificateHashes: the relay's certificate is self-signed and no
      // root store trusts it. Pinning needs no browser flag and no system CA.
      transportOptions: certSha256Hex
        ? { serverCertificateHashes: [{ algorithm: 'sha-256', value: fromHex(certSha256Hex) }] }
        : {},
      callbacks: {
        onMessageSent: msg => console.log(`control sent: ${msg.constructor.name}`),
        onMessageReceived: msg => console.log(`control received: ${msg.constructor.name}`),
        onSessionTerminated: reason => console.log(`session terminated: ${String(reason)}`),
      },
    })
    // WHY Reflect.get: TypeScript 6.0.3's DOM lib has no `WebTransport.protocol`
    // (tsc reports TS2339), although Chromium exposes it.
    const protocol: unknown = Reflect.get(client.webTransport, 'protocol')
    console.log(`connected: protocol=${String(protocol)}`)
  },

  async publishTrack(namespace, trackName) {
    const moq = connected()
    await moq.publishNamespace(Tuple.fromUtf8Path(namespace))

    const fullTrackName = FullTrackName.tryNew(namespace, trackName)
    let controller: ReadableStreamDefaultController<MoqtObject> | undefined
    const stream = new ReadableStream<MoqtObject>({
      start: c => {
        controller = c
      },
    })
    if (!controller) throw new Error('ReadableStream did not start synchronously')

    moq.addOrUpdateTrack({
      fullTrackName,
      trackSource: { live: new LiveTrackSource(stream) },
      publisherPriority: 0,
    })
    const trackAlias = moq.trackSources.get(fullTrackName.toString())?.trackAlias
    if (trackAlias === undefined) throw new Error('track alias was not assigned')

    const result = await moq.publish(fullTrackName, true, trackAlias)
    if (result instanceof RequestError) {
      throw new Error(`PUBLISH rejected: code=${result.errorCode} reason=${result.reasonPhrase.phrase}`)
    }
    published = { fullTrackName, controller }
    console.log(`track published: alias=${trackAlias}`)
    return trackAlias.toString()
  },

  sendObject(payloadHex) {
    if (!published) throw new Error('publishTrack() has not completed')
    const object = MoqtObject.newWithPayload(
      published.fullTrackName,
      new Location(0n, 0n),
      0,
      ObjectForwardingPreference.Subgroup,
      0n,
      null,
      fromHex(payloadHex),
    )
    published.controller.enqueue(object)
    console.log(`object enqueued: group=0 object=0 payload_len=${payloadHex.length / 2}`)
  },

  async subscribe(namespace, trackName) {
    const result = await connected().subscribe({
      fullTrackName: FullTrackName.tryNew(namespace, trackName),
      groupOrder: GroupOrder.Original,
      filterType: FilterType.LatestObject,
      forward: true,
      priority: 128,
    })
    if (result instanceof RequestError) {
      throw new Error(`SUBSCRIBE rejected: code=${result.errorCode} reason=${result.reasonPhrase.phrase}`)
    }

    const reader = result.stream.getReader()
    received = (async () => {
      for (;;) {
        const { done, value } = await reader.read()
        if (done) throw new Error('object stream ended before any object with a payload')
        if (!value.payload) continue
        return {
          group: value.location.group.toString(),
          object: value.location.object.toString(),
          payloadHex: toHex(value.payload),
        }
      }
    })()
    console.log(`subscribed: requestId=${result.requestId}`)
    return result.requestId.toString()
  },

  firstObject() {
    if (!received) throw new Error('subscribe() has not completed')
    return received
  },

  async disconnect() {
    await client?.disconnect()
    client = null
  },
}

console.log('page ready')
