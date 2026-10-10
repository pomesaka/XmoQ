// Phase 0 spike driver: starts the moqtail relay, a real Chromium and the Rust
// client, moves one MOQT Object through the relay and compares the received
// payload with the sent one.
//
// Usage: node run.mjs [--direction browser-to-rust|rust-to-browser|browser-to-browser|rust-to-rust]
//                     [--relay moqtail|own] [--subscribers N]
//                     [--rust-publisher moqtail|own] [--rust-subscriber moqtail|own]
//                     [--publish-namespace] [--subscribe-before-publish]
//                     [--browser chromium|headless-shell|chrome]
//                     [--cert-trust hash|spki-flag|none] [--port N] [--verbose] [--trace]

import { execFileSync, spawn } from 'node:child_process'
import { X509Certificate, createHash, randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { build } from 'esbuild'
import { chromium } from 'playwright'

const here = dirname(fileURLToPath(import.meta.url))
const spikeRoot = join(here, '..')
const relayBin = join(spikeRoot, '.tools', 'bin', 'relay')
// Rust client binary per --rust-publisher / --rust-subscriber value. `moqtail` is
// the spike client on the moqtail crate; `own` is XmoQ's client on `crates/moqt`.
// Both take the same arguments and print the same marker lines.
const RUST_CLIENTS = {
  moqtail: join(spikeRoot, 'rust-client', 'target', 'debug', 'xmoq-moqtail-spike'),
  own: join(spikeRoot, '..', '..', 'target', 'debug', 'xmoq-moqt-client'),
}
const certDir = join(spikeRoot, 'cert')
const certFile = join(certDir, 'cert.pem')
const keyFile = join(certDir, 'key.pem')

// Relay per --relay value: its binary, its arguments, and the line that says it
// is listening, whose first group names the implementation. `moqtail` is the
// moqtail relay; `own` is XmoQ's relay in `crates/relay`.
const RELAYS = {
  moqtail: {
    bin: relayBin,
    args: port => [
      '--port', port, '--host', '127.0.0.1', '--cert-file', certFile, '--key-file', keyFile,
      '--log-folder', join(spikeRoot, '.logs'),
    ],
    ready: /(moqtail\/\S+) is running on/,
  },
  own: {
    // XMOQ_RELAY_BIN: another build of the own relay, as spikes/relay-core-diff makes one.
    bin: process.env.XMOQ_RELAY_BIN ?? join(spikeRoot, '..', '..', 'target', 'debug', 'xmoq-relay'),
    args: port => ['--port', port, '--cert-file', certFile, '--key-file', keyFile],
    ready: /^LISTENING (\S+) port=/,
  },
}

const NAMESPACE = 'xmoq/phase0'
const TRACK = 'object'
const STEP_TIMEOUT_MS = 20_000

// Playwright launch options per --browser value. `chromium` is the full browser in
// new headless mode; `headless-shell` is Playwright's default headless binary.
const BROWSERS = {
  chromium: { channel: 'chromium' },
  'headless-shell': {},
  chrome: { channel: 'chrome' },
}

// Which side runs in the browser, per --direction value.
const DIRECTIONS = {
  'browser-to-rust': { publisherInBrowser: true, subscriberInBrowser: false },
  'rust-to-browser': { publisherInBrowser: false, subscriberInBrowser: true },
  'browser-to-browser': { publisherInBrowser: true, subscriberInBrowser: true },
  'rust-to-rust': { publisherInBrowser: false, subscriberInBrowser: false },
}

// `none` is the negative control: the self-signed certificate is not trusted at all.
const CERT_TRUST_MODES = ['hash', 'spki-flag', 'none']

const { values: options } = parseArgs({
  options: {
    direction: { type: 'string', default: 'browser-to-rust' },
    relay: { type: 'string', default: 'moqtail' },
    // How many subscribers of the kind --direction names. All have to receive
    // the Object.
    subscribers: { type: 'string', default: '1' },
    'rust-publisher': { type: 'string', default: 'moqtail' },
    'rust-subscriber': { type: 'string', default: 'moqtail' },
    // The publisher sends PUBLISH_NAMESPACE and no PUBLISH, and publishes the
    // track when the relay subscribes to it (draft 18, §9.5). For the `own`
    // Rust publisher and for the page (moqtail-ts).
    'publish-namespace': { type: 'boolean', default: false },
    // The subscribers ask for the track after the `own` Rust publisher
    // announced its namespace and before its PUBLISH: the relay's SUBSCRIBE to
    // the publisher is unanswered when the PUBLISH arrives (draft 18, §5.1).
    // The publisher holds its PUBLISH and its answer back to make that order.
    'subscribe-before-publish': { type: 'boolean', default: false },
    // How the publisher's Subgroup Headers give the Subgroup ID (draft 18,
    // §11.4.2): zero, first-object or explicit. The `own` Rust publisher has
    // all three; the page (moqtail-ts) has zero and first-object.
    'subgroup-id': { type: 'string', default: 'zero' },
    // A second Rust publisher of the same track, moqtail or own, that
    // publishes after the first and sends the same Object (Group 0, the same
    // payload). Needs Rust subscribers. The run then also checks that every
    // subscriber got Group 0 once, that the subscription goes on when the
    // first publisher has left, and, with an `own` second publisher, that
    // its next Object (Group 1) arrives.
    'second-rust-publisher': { type: 'string' },
    // The Group ID of the first Object of the `own` Rust publisher.
    'first-group': { type: 'string', default: '0' },
    // Bytes of padding after the payload's text. With a second publisher, a
    // payload that takes a while to write lets the relay's STOP_SENDING for
    // the second copy arrive while that publisher is still writing it.
    'payload-padding': { type: 'string', default: '0' },
    // A scenario of its own, with own Rust clients and the own relay: a
    // publisher announces the namespace and answers the relay's SUBSCRIBE
    // only after 4 s; a subscriber subscribes, so that the relay asks that
    // publisher; a second publisher sends PUBLISH for the track meanwhile.
    // The subscriber has to be answered then, get what the second publisher
    // sends (Group 0), and, once the first has answered, what that one sends
    // (Group 1).
    'publish-while-asked': { type: 'boolean', default: false },
    // After the first Object, the `own` Rust publisher ends its subscription
    // with PUBLISH_DONE and publishes the track anew at once, under the next
    // Track Alias (`other`) or the same one (`same`), and then sends its next
    // Object (Group 1). The subscriber, an `own` Rust one, has to stay
    // subscribed and get that Object.
    republish: { type: 'string' },
    // Makes the `own` client print the raw bytes of every stream.
    trace: { type: 'boolean', default: false },
    // After the transfer, ends the Rust publisher and waits until the `own`
    // subscriber reports that PUBLISH_DONE ended its subscription.
    'await-publish-done': { type: 'boolean', default: false },
    browser: { type: 'string', default: 'chromium' },
    'cert-trust': { type: 'string', default: 'hash' },
    port: { type: 'string', default: '44318' },
    verbose: { type: 'boolean', default: false },
  },
})

function log(source, line) {
  console.log(`[${source}] ${line}`)
}

function withTimeout(promise, what) {
  let timer
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timed out waiting for ${what}`)), STEP_TIMEOUT_MS)
  })
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer))
}

/**
 * Chromium accepts a certificate through `serverCertificateHashes` only if it is
 * ECDSA P-256 and valid for less than two weeks, so the certificate is regenerated
 * whenever it is missing or about to expire.
 */
function ensureCertificate() {
  const oneDayMs = 24 * 60 * 60 * 1000
  const usable =
    existsSync(certFile) &&
    existsSync(keyFile) &&
    new Date(new X509Certificate(readFileSync(certFile)).validTo).getTime() - Date.now() > oneDayMs
  if (!usable) {
    mkdirSync(certDir, { recursive: true })
    // WHY two steps instead of `req -newkey ec`: with LibreSSL 3.3.6 (macOS
    // /usr/bin/openssl) the one-step form wrote a key that the relay rejected with
    // "failed to parse private key as RSA, ECDSA, or EdDSA".
    const quiet = { stdio: ['ignore', 'ignore', 'pipe'] }
    execFileSync('openssl', ['ecparam', '-name', 'prime256v1', '-genkey', '-noout', '-out', keyFile], quiet)
    execFileSync(
      'openssl',
      [
        'req', '-x509', '-new', '-key', keyFile, '-out', certFile, '-days', '13', '-subj', '/CN=localhost',
        '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1',
      ],
      quiet,
    )
    log('driver', `generated ${certFile}`)
  }
  const certificate = new X509Certificate(readFileSync(certFile))
  return {
    sha256Hex: createHash('sha256').update(certificate.raw).digest('hex'),
    spkiSha256Base64: createHash('sha256')
      .update(certificate.publicKey.export({ type: 'spki', format: 'der' }))
      .digest('base64'),
    validTo: certificate.validTo,
  }
}

/** Spawns a child, echoes its output, and lets callers wait for a matching line. */
function spawnLogged(source, command, args, env = {}) {
  const child = spawn(command, args, { env: { ...process.env, ...env }, stdio: ['pipe', 'pipe', 'pipe'] })
  const lines = []
  const waiters = []
  const exited = new Promise(resolve => child.once('exit', code => resolve(code)))

  const onLine = line => {
    // Strip ANSI colors that `tracing` adds, so the patterns below can match.
    const clean = line.replace(/\u001b\[[0-9;]*m/g, '')
    lines.push(clean)
    log(source, clean)
    for (const waiter of [...waiters]) {
      const match = clean.match(waiter.pattern)
      if (match) {
        waiters.splice(waiters.indexOf(waiter), 1)
        waiter.resolve(match)
      }
    }
  }
  for (const stream of [child.stdout, child.stderr]) {
    let buffered = ''
    stream.setEncoding('utf8')
    stream.on('data', chunk => {
      buffered += chunk
      const parts = buffered.split('\n')
      buffered = parts.pop() ?? ''
      parts.filter(Boolean).forEach(onLine)
    })
  }

  return {
    child,
    exited,
    /** How many lines printed so far match. */
    count(pattern) {
      return lines.filter(line => pattern.test(line)).length
    },
    waitForLine(pattern) {
      for (const line of lines) {
        const match = line.match(pattern)
        if (match) return Promise.resolve(match)
      }
      const matched = new Promise(resolve => waiters.push({ pattern, resolve }))
      const died = exited.then(code => {
        throw new Error(`${source} exited with code ${code} before printing ${pattern}`)
      })
      return withTimeout(Promise.race([matched, died]), `${source} to print ${pattern}`)
    },
  }
}

async function servePage() {
  const outfile = join(here, 'dist', 'page.js')
  await build({
    entryPoints: [join(here, 'src', 'page.ts')],
    bundle: true,
    format: 'esm',
    target: 'es2022',
    // The page logs control messages by class name; without this esbuild renames
    // colliding classes (`Setup` became `Setup2`).
    keepNames: true,
    outfile,
    logLevel: 'warning',
  })
  const files = {
    '/': { type: 'text/html', body: readFileSync(join(here, 'index.html')) },
    '/page.js': { type: 'text/javascript', body: readFileSync(outfile) },
  }
  const server = createServer((request, response) => {
    const file = files[request.url ?? '']
    if (!file) {
      response.writeHead(404).end()
      return
    }
    response.writeHead(200, { 'content-type': file.type }).end(file.body)
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  // WHY localhost and not 127.0.0.1 in the URL: both are secure contexts for
  // WebTransport, and `localhost` is what a developer would type.
  return { server, url: `http://localhost:${server.address().port}/` }
}

async function main() {
  const direction = DIRECTIONS[options.direction]
  if (!direction) throw new Error(`unknown --direction ${options.direction}`)
  const launchOptions = BROWSERS[options.browser]
  if (!launchOptions) throw new Error(`unknown --browser ${options.browser}`)
  const certTrust = options['cert-trust']
  if (!CERT_TRUST_MODES.includes(certTrust)) throw new Error(`unknown --cert-trust ${certTrust}`)
  const rustPublisher = direction.publisherInBrowser ? null : options['rust-publisher']
  const rustSubscriber = direction.subscriberInBrowser ? null : options['rust-subscriber']
  for (const implementation of [rustPublisher, rustSubscriber]) {
    if (implementation !== null && !RUST_CLIENTS[implementation]) {
      throw new Error(`unknown Rust client ${implementation}`)
    }
  }
  const relayKind = RELAYS[options.relay]
  if (!relayKind) throw new Error(`unknown --relay ${options.relay}`)
  const subscriberCount = Number.parseInt(options.subscribers, 10)
  if (!(subscriberCount >= 1)) throw new Error(`--subscribers must be at least 1`)
  const binaries = [relayKind.bin, ...[rustPublisher, rustSubscriber].filter(Boolean).map(name => RUST_CLIENTS[name])]
  for (const binary of binaries) {
    if (!existsSync(binary)) throw new Error(`${binary} is missing; see README.md for the build steps`)
  }

  const certificate = ensureCertificate()
  log('driver', `certificate sha256=${certificate.sha256Hex} validTo=${certificate.validTo}`)
  const relayAuthority = `127.0.0.1:${options.port}`
  const relayUrl = `https://${relayAuthority}`
  const payload = Buffer.concat([
    Buffer.from(`xmoq-phase0 ${new Date().toISOString()} ${randomBytes(8).toString('hex')}`),
    Buffer.alloc(Number.parseInt(options['payload-padding'], 10), 0x2e),
  ])
  const payloadHex = payload.toString('hex')

  // `hash` pins the certificate from the page (serverCertificateHashes).
  // `spki-flag` leaves the page untouched and makes Chromium accept the key through
  // launch arguments; both arguments are needed, the second forces HTTP/3 to the relay.
  const browserArgs =
    certTrust === 'spki-flag'
      ? [
          `--ignore-certificate-errors-spki-list=${certificate.spkiSha256Base64}`,
          `--origin-to-force-quic-on=${relayAuthority}`,
        ]
      : []
  const pageCertHash = certTrust === 'hash' ? certificate.sha256Hex : null
  const awaitPublishDone = options['await-publish-done']
  if (awaitPublishDone && (direction.publisherInBrowser || direction.subscriberInBrowser || rustSubscriber !== 'own')) {
    throw new Error('--await-publish-done needs --direction rust-to-rust and --rust-subscriber own')
  }

  const cleanups = []
  let result
  try {
    const relay = spawnLogged(
      'relay',
      relayKind.bin,
      relayKind.args(options.port),
      options.verbose ? { RUST_LOG: 'debug' } : {},
    )
    cleanups.push(async () => {
      relay.child.kill('SIGINT')
      await relay.exited
    })
    const [, relayVersion] = await relay.waitForLine(relayKind.ready)

    // rust-to-rust needs no page, so the browser is started on first use.
    let browserSession
    const startBrowser = async () => {
      const { server, url: pageUrl } = await servePage()
      cleanups.push(() => new Promise(resolve => server.close(resolve)))
      const browser = await chromium.launch({ headless: true, args: browserArgs, ...launchOptions })
      cleanups.push(() => browser.close())
      log('driver', `browser=${options.browser} version=${browser.version()} args=${JSON.stringify(browserArgs)}`)
      return { browser, pageUrl }
    }

    const openConnectedPage = async label => {
      browserSession ??= await startBrowser()
      const { browser, pageUrl } = browserSession
      const context = await browser.newContext()
      const page = await context.newPage()
      page.on('console', message => log(label, message.text()))
      page.on('pageerror', error => log(label, `pageerror: ${error.message}`))
      await page.goto(pageUrl)
      await page.waitForFunction(() => typeof window.xmoq === 'object')
      await withTimeout(
        page.evaluate(([url, hash]) => window.xmoq.connect(url, hash), [relayUrl, pageCertHash]),
        `${label} to connect`,
      )
      return page
    }
    const spawnRustClient = (label, implementation, mode, extraArgs = []) => {
      const traceArgs = options.trace && implementation === 'own' ? ['--trace'] : []
      const client = spawnLogged(label, RUST_CLIENTS[implementation], [
        '--mode', mode, '--url', relayUrl, '--cert-sha256', certificate.sha256Hex,
        '--namespace', NAMESPACE, '--track', TRACK, ...extraArgs, ...traceArgs,
      ])
      cleanups.push(async () => {
        // Closing stdin tells a publisher to end its subscription and leave. It
        // gets a moment to do so before it is killed, so PUBLISH_DONE reaches the relay.
        client.child.stdin.end()
        const grace = new Promise(resolve => setTimeout(resolve, 3_000))
        await Promise.race([client.exited, grace])
        client.child.kill('SIGTERM')
        await client.exited
      })
      return client
    }

    if (options['publish-while-asked']) {
      if (options.relay !== 'own' || direction.publisherInBrowser || direction.subscriberInBrowser) {
        throw new Error('--publish-while-asked needs --relay own and --direction rust-to-rust')
      }
      const asked = spawnRustClient('rust-pub', 'own', 'publish-namespace', [
        '--payload-hex', payloadHex, '--serve-delay-ms', '4000', '--first-group', '1',
      ])
      await asked.waitForLine(/^NAMESPACE_PUBLISHED/)
      const subscriber = spawnRustClient('rust-sub', 'own', 'subscribe', ['--objects', '2'])
      // The relay asks the publisher that announced the namespace.
      await relay.waitForLine(/^OPEN_REQUEST /)
      const answeredBeforeThePublish = subscriber.count(/^SUBSCRIBED /) > 0
      const other = spawnRustClient('rust-pub2', 'own', 'publish', ['--payload-hex', payloadHex])
      await other.waitForLine(/^PUBLISHED /)
      // The PUBLISH gave the track its source: the subscriber is answered,
      // although the publisher that was asked has not answered yet.
      await subscriber.waitForLine(/^SUBSCRIBED /)
      const askedHadAnswered = asked.count(/^SERVING /) > 0
      other.child.stdin.write('send\n')
      await other.waitForLine(/^SENT group=0 /)
      await subscriber.waitForLine(/^OBJECT group=0 /)
      // The publisher that was asked answers now, and is a second source.
      await asked.waitForLine(/^SERVING /)
      asked.child.stdin.write('send\n')
      await asked.waitForLine(/^SENT group=1 /)
      await subscriber.waitForLine(/^OBJECT group=1 /)
      result = {
        scenario: 'publish-while-asked',
        match: !answeredBeforeThePublish && !askedHadAnswered,
        answeredBeforeThePublish,
        askedHadAnsweredWhenTheSubscriberWasAnswered: askedHadAnswered,
        group0: subscriber.count(/^OBJECT group=0 /),
        group1: subscriber.count(/^OBJECT group=1 /),
        relayVersion,
      }
      log('driver', `RESULT ${JSON.stringify(result)}`)
      if (!result.match) throw new Error('the subscriber was answered at another time than expected')
      return
    }
    const byNamespace = options['publish-namespace']
    const subscribeFirst = options['subscribe-before-publish']
    if (subscribeFirst && (direction.publisherInBrowser || rustPublisher !== 'own' || byNamespace || options.relay !== 'own')) {
      throw new Error('--subscribe-before-publish needs --relay own, --rust-publisher own and no --publish-namespace')
    }
    // Subscriber pages connect before the publisher starts when they have to
    // subscribe inside the publisher's delay: launching the browser takes long.
    const subscriberPages = []
    if (subscribeFirst && direction.subscriberInBrowser) {
      for (let index = 0; index < subscriberCount; index++) {
        subscriberPages.push(await openConnectedPage(`browser-sub${subscriberCount === 1 ? '' : `-${index + 1}`}`))
      }
    }

    // The publisher offers the track first: the relay rejects a SUBSCRIBE for a
    // track nobody has published or announced the namespace of.
    let sendObject
    let endPublisher
    let secondPublisher = null
    let firstPublisher = null
    const subscriberClients = []
    let published = async () => {}
    if (direction.publisherInBrowser) {
      const page = await openConnectedPage('browser-pub')
      await withTimeout(
        byNamespace
          ? page.evaluate(([ns, track]) => window.xmoq.announceTrack(ns, track), [NAMESPACE, TRACK])
          : page.evaluate(([ns, track]) => window.xmoq.publishTrack(ns, track), [NAMESPACE, TRACK]),
        'browser-pub to offer the track',
      )
      const subgroupInHeader = options['subgroup-id'] !== 'first-object'
      sendObject = () => page.evaluate(([hex, inHeader]) => window.xmoq.sendObject(hex, inHeader), [payloadHex, subgroupInHeader])
    } else {
      if (byNamespace && rustPublisher !== 'own') throw new Error('--publish-namespace needs --rust-publisher own')
      const mode = byNamespace ? 'publish-namespace' : 'publish'
      // 3 s for the subscribers to ask, and an answer to the relay's SUBSCRIBE
      // that would come after the PUBLISH. Both are below the 5 s the relay
      // waits for an answer.
      const delays = subscribeFirst ? ['--publish-delay-ms', '3000', '--answer-delay-ms', '4000'] : []
      const subgroupId = options['subgroup-id'] === 'zero' ? [] : ['--subgroup-id', options['subgroup-id']]
      if (subgroupId.length > 0 && rustPublisher !== 'own') throw new Error('--subgroup-id needs --rust-publisher own')
      const firstGroup = options['first-group'] === '0' ? [] : ['--first-group', options['first-group']]
      if (firstGroup.length > 0 && rustPublisher !== 'own') throw new Error('--first-group needs --rust-publisher own')
      const client = spawnRustClient('rust-pub', rustPublisher, mode, ['--payload-hex', payloadHex, ...delays, ...subgroupId, ...firstGroup])
      firstPublisher = client
      await client.waitForLine(byNamespace || subscribeFirst ? /^NAMESPACE_PUBLISHED/ : /^PUBLISHED /)
      if (subscribeFirst) published = () => client.waitForLine(/^PUBLISHED /)
      sendObject = async () => {
        // The publisher has a subscriber only once the relay's SUBSCRIBE
        // reached it.
        if (byNamespace) await client.waitForLine(/^SERVING /)
        client.child.stdin.write('send\n')
        await client.waitForLine(/^SENT /)
      }
      endPublisher = () => client.child.stdin.end()
      if (options['second-rust-publisher']) {
        if (byNamespace || subscribeFirst || direction.subscriberInBrowser || options.relay !== 'own') {
          throw new Error('--second-rust-publisher needs --relay own, Rust subscribers and a plain PUBLISH')
        }
        // PUBLISHED is printed after PUBLISH_OK: the relay took the second
        // publisher of the track.
        secondPublisher = spawnRustClient('rust-pub2', options['second-rust-publisher'], 'publish', ['--payload-hex', payloadHex])
        await secondPublisher.waitForLine(/^PUBLISHED /)
        const sendFirst = sendObject
        sendObject = async () => {
          await sendFirst()
          secondPublisher.child.stdin.write('send\n')
          await secondPublisher.waitForLine(/^SENT group=0 /)
        }
      }
    }

    // The subscribers are live before the Object is sent: the subscription uses
    // the LatestObject filter, which does not replay what was published earlier.
    const receivers = []
    const subscriptionEnds = []
    // With --subscribe-before-publish the answers come only after the PUBLISH.
    const subscribing = []
    const subscribed = async answer => {
      // Marked as handled: a failure elsewhere must not leave it unhandled.
      answer.catch(() => {})
      if (subscribeFirst) subscribing.push(answer)
      else await answer
    }
    for (let index = 0; index < subscriberCount; index++) {
      const label = subscriberCount === 1 ? '' : `-${index + 1}`
      if (direction.subscriberInBrowser) {
        const page = subscriberPages[index] ?? (await openConnectedPage(`browser-sub${label}`))
        await subscribed(
          withTimeout(
            page.evaluate(([ns, track]) => window.xmoq.subscribe(ns, track), [NAMESPACE, TRACK]),
            'browser-sub to subscribe',
          ),
        )
        receivers.push(async () => {
          const object = await withTimeout(page.evaluate(() => window.xmoq.firstObject()), 'browser-sub to receive')
          return object.payloadHex
        })
      } else {
        // Asking for a second Object keeps the subscriber waiting after the first,
        // so it is still there when the publisher ends the track.
        // With two publishers the own subscriber is kept waiting as well: it
        // is to see what comes after the first Object, or that nothing does.
        // (The moqtail client has no such flag; it reads until it is ended.)
        const more = awaitPublishDone || ((options['second-rust-publisher'] || options.republish) && rustSubscriber === 'own')
        const client = spawnRustClient(`rust-sub${label}`, rustSubscriber, 'subscribe', more ? ['--objects', '2'] : [])
        subscriberClients.push(client)
        await subscribed(client.waitForLine(/^SUBSCRIBED /))
        subscriptionEnds.push(() => client.waitForLine(/^SUBSCRIPTION_ENDED (.*)$/))
        receivers.push(async () => {
          const [, hex] = await client.waitForLine(/^OBJECT .*payload_hex=([0-9a-f]*)$/)
          return hex
        })
      }
    }

    if (subscribeFirst) {
      // The order this run is about, as the relay saw it: its SUBSCRIBE went
      // out, and gave way when the PUBLISH came.
      await relay.waitForLine(/^OPEN_REQUEST /)
      await published()
      await relay.waitForLine(/^CANCEL_OWN_REQUEST /)
      await Promise.all(subscribing)
    }

    await sendObject()
    const receivedHexes = await Promise.all(receivers.map(receive => receive()))
    const receivedHex = receivedHexes[0]
    let twoPublishers = null
    if (secondPublisher) {
      // The first publisher leaves with PUBLISH_DONE. The relay has to keep
      // the track, which has another source.
      endPublisher()
      // The first publisher is the relay's first session, session 0. The
      // relay finishes its PUBLISH stream once the stream it counted has
      // ended there; a publisher that closes without waiting just goes.
      await relay.waitForLine(/^(FINISH_REQUEST|SESSION_CLOSED) session=0\b/)
      const nextFromOwn = options['second-rust-publisher'] === 'own'
      if (nextFromOwn) {
        // The own client numbers its Groups itself: the next one is Group 1.
        secondPublisher.child.stdin.write('send\n')
        await secondPublisher.waitForLine(/^SENT group=1 /)
        // The moqtail subscriber of this spike leaves after its first
        // Object, so only an own subscriber can show the next one.
        if (rustSubscriber === 'own') {
          await Promise.all(subscriberClients.map(client => client.waitForLine(/^OBJECT group=1 /)))
        }
      } else {
        // The moqtail client sends one Object. Give a second copy of Group 0
        // or an end of the subscription the time to show up.
        await new Promise(resolve => setTimeout(resolve, 1500))
      }
      twoPublishers = {
        second: options['second-rust-publisher'],
        group0PerSubscriber: subscriberClients.map(client => client.count(/^OBJECT group=0 /)),
        endedWhileOnePublisherWasLeft: subscriberClients.some(client => client.count(/^SUBSCRIPTION_ENDED /) > 0),
        nextGroupArrived: nextFromOwn && rustSubscriber === 'own',
        // What the moqtail subscriber shows is its first Object only.
        subscriberStays: rustSubscriber === 'own',
      }
    }
    let republished = null
    if (options.republish) {
      if (rustPublisher !== 'own' || rustSubscriber !== 'own' || byNamespace || !['same', 'other'].includes(options.republish)) {
        throw new Error('--republish same|other needs own Rust clients and a plain PUBLISH')
      }
      firstPublisher.child.stdin.write(options.republish === 'same' ? 'republish-same-alias\n' : 'republish\n')
      // Printed once the new PUBLISH has its PUBLISH_OK and the stream of
      // the first one has ended, or 3 s have passed.
      const [, firstEnded] = await firstPublisher.waitForLine(/^PUBLISH_DONE_PEER_ENDED how=(.*)$/)
      firstPublisher.child.stdin.write('send\n')
      await firstPublisher.waitForLine(/^SENT group=1 /)
      await Promise.all(subscriberClients.map(client => client.waitForLine(/^OBJECT group=1 /)))
      republished = {
        alias: options.republish,
        publishOks: firstPublisher.count(/^PUBLISHED /),
        firstSubscriptionEnded: firstEnded,
        subscriptionEndedForASubscriber: subscriberClients.some(client => client.count(/^SUBSCRIPTION_ENDED /) > 0),
      }
    }
    let subscriptionEnd = null
    if (awaitPublishDone) {
      endPublisher()
      const ends = await Promise.all(subscriptionEnds.map(wait => wait()))
      subscriptionEnd = ends.map(([, end]) => end).join(' | ')
    }

    result = {
      direction: options.direction,
      match:
        receivedHexes.every(hex => hex === payloadHex) &&
        (twoPublishers === null ||
          (twoPublishers.group0PerSubscriber.every(count => count === 1) && !twoPublishers.endedWhileOnePublisherWasLeft)) &&
        (republished === null || (republished.publishOks === 2 && !republished.subscriptionEndedForASubscriber)),
      twoPublishers,
      republished,
      relay: options.relay,
      subscribers: subscriberCount,
      sentText: payload.toString(),
      receivedText: Buffer.from(receivedHex, 'hex').toString(),
      sentHex: payloadHex,
      receivedHex,
      relayVersion,
      rustPublisher,
      publishNamespace: options['publish-namespace'],
      subscribeBeforePublish: subscribeFirst,
      rustSubscriber,
      browser: browserSession ? options.browser : null,
      browserVersion: browserSession ? browserSession.browser.version() : null,
      certTrust,
      subscriptionEnd,
    }
  } finally {
    for (const cleanup of cleanups.reverse()) {
      await cleanup().catch(error => log('driver', `cleanup failed: ${error.message}`))
    }
  }

  log('driver', `RESULT ${JSON.stringify(result)}`)
  if (!result.match) throw new Error('received payload differs from the sent payload')
}

main().catch(error => {
  log('driver', `FAILED ${error.message}`)
  process.exitCode = 1
})
