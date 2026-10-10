// Phase 0 spike driver: starts the moqtail relay, a real Chromium and the Rust
// client, moves one MOQT Object through the relay and compares the received
// payload with the sent one.
//
// Usage: node run.mjs [--direction browser-to-rust|rust-to-browser|browser-to-browser]
//                     [--browser chromium|headless-shell|chrome]
//                     [--cert-trust hash|spki-flag|none] [--port N] [--verbose]

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
const rustClientBin = join(spikeRoot, 'rust-client', 'target', 'debug', 'xmoq-moqtail-spike')
const certDir = join(spikeRoot, 'cert')
const certFile = join(certDir, 'cert.pem')
const keyFile = join(certDir, 'key.pem')

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
}

// `none` is the negative control: the self-signed certificate is not trusted at all.
const CERT_TRUST_MODES = ['hash', 'spki-flag', 'none']

const { values: options } = parseArgs({
  options: {
    direction: { type: 'string', default: 'browser-to-rust' },
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
  for (const binary of [relayBin, rustClientBin]) {
    if (!existsSync(binary)) throw new Error(`${binary} is missing; see README.md for the build steps`)
  }

  const certificate = ensureCertificate()
  log('driver', `certificate sha256=${certificate.sha256Hex} validTo=${certificate.validTo}`)
  const relayAuthority = `127.0.0.1:${options.port}`
  const relayUrl = `https://${relayAuthority}`
  const payload = Buffer.from(`xmoq-phase0 ${new Date().toISOString()} ${randomBytes(8).toString('hex')}`)
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

  const cleanups = []
  let result
  try {
    const relay = spawnLogged(
      'relay',
      relayBin,
      [
        '--port', options.port, '--host', '127.0.0.1', '--cert-file', certFile, '--key-file', keyFile,
        '--log-folder', join(spikeRoot, '.logs'),
      ],
      options.verbose ? { RUST_LOG: 'debug' } : {},
    )
    cleanups.push(async () => {
      relay.child.kill('SIGINT')
      await relay.exited
    })
    const [, relayVersion] = await relay.waitForLine(/(moqtail\/\S+) is running on/)

    const { server, url: pageUrl } = await servePage()
    cleanups.push(() => new Promise(resolve => server.close(resolve)))

    const browser = await chromium.launch({ headless: true, args: browserArgs, ...launchOptions })
    cleanups.push(() => browser.close())
    log('driver', `browser=${options.browser} version=${browser.version()} args=${JSON.stringify(browserArgs)}`)

    const openConnectedPage = async label => {
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
    const spawnRustClient = (label, mode, extraArgs = []) => {
      const client = spawnLogged(label, rustClientBin, [
        '--mode', mode, '--url', relayUrl, '--cert-sha256', certificate.sha256Hex,
        '--namespace', NAMESPACE, '--track', TRACK, ...extraArgs,
      ])
      cleanups.push(async () => {
        client.child.stdin.end()
        client.child.kill('SIGTERM')
        await client.exited
      })
      return client
    }

    // The publisher offers the track first: the relay rejects a SUBSCRIBE for a
    // track nobody has published.
    let sendObject
    if (direction.publisherInBrowser) {
      const page = await openConnectedPage('browser-pub')
      await withTimeout(
        page.evaluate(([ns, track]) => window.xmoq.publishTrack(ns, track), [NAMESPACE, TRACK]),
        'browser-pub to publish the track',
      )
      sendObject = () => page.evaluate(hex => window.xmoq.sendObject(hex), payloadHex)
    } else {
      const client = spawnRustClient('rust-pub', 'publish', ['--payload-hex', payloadHex])
      await client.waitForLine(/^PUBLISHED /)
      sendObject = async () => {
        client.child.stdin.write('send\n')
        await client.waitForLine(/^SENT /)
      }
    }

    // The subscriber is live before the Object is sent: the subscription uses the
    // LatestObject filter, which does not replay what was published earlier.
    let receiveHex
    if (direction.subscriberInBrowser) {
      const page = await openConnectedPage('browser-sub')
      await withTimeout(
        page.evaluate(([ns, track]) => window.xmoq.subscribe(ns, track), [NAMESPACE, TRACK]),
        'browser-sub to subscribe',
      )
      receiveHex = async () => {
        const object = await withTimeout(page.evaluate(() => window.xmoq.firstObject()), 'browser-sub to receive')
        return object.payloadHex
      }
    } else {
      const client = spawnRustClient('rust-sub', 'subscribe')
      await client.waitForLine(/^SUBSCRIBED /)
      receiveHex = async () => {
        const [, hex] = await client.waitForLine(/^OBJECT .*payload_hex=([0-9a-f]*)$/)
        return hex
      }
    }

    await sendObject()
    const receivedHex = await receiveHex()

    result = {
      direction: options.direction,
      match: receivedHex === payloadHex,
      sentText: payload.toString(),
      receivedText: Buffer.from(receivedHex, 'hex').toString(),
      sentHex: payloadHex,
      receivedHex,
      relayVersion,
      browser: options.browser,
      browserVersion: browser.version(),
      certTrust,
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
