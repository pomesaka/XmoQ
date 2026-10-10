// What a page sees when the own relay closes its session, and when the relay
// resets or stops one of its streams.
//
//   cargo build -p xmoq-relay
//   node spikes/wt-library-comparison/relay-close.mjs [--port 44460] [--rounds 5]
//
// The page connects with the subprotocol `moqt-18` and does two things, each
// `--rounds` times on a session of its own:
//
// * `violation`: opens a bidirectional stream that begins with a message type
//   no draft-18 message has (0x3f). The relay closes the session with
//   PROTOCOL_VIOLATION (0x3) and a reason; the page reads `WebTransport.closed`.
// * `early`: opens a unidirectional subgroup stream whose Track Alias no
//   PUBLISH explains. After two seconds the relay stops it with CANCELLED
//   (0x1); the page reads `streamErrorCode` from its failing write.
//
// Uses the Playwright install of spikes/moqtail and the certificate of this
// directory.
import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

import { chromium } from '../moqtail/browser/node_modules/playwright/index.mjs'
import { certFile, ensureCertificate, keyFile } from './cert.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const { values: options } = parseArgs({
  options: {
    port: { type: 'string', default: '44460' },
    rounds: { type: 'string', default: '5' },
  },
})
const certificate = ensureCertificate()
const base = `https://127.0.0.1:${options.port}`

// Runs in the page. Returns plain data only.
async function probe([base, certHash, rounds]) {
  const open = () =>
    new WebTransport(base, {
      serverCertificateHashes: [{ algorithm: 'sha-256', value: new Uint8Array(certHash) }],
      protocols: ['moqt-18'],
    })
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
  const describe = error => ({
    name: error?.name,
    message: error?.message,
    source: error?.source,
    streamErrorCode: error?.streamErrorCode,
  })
  const closed = wt =>
    wt.closed.then(
      info => ({ resolved: { closeCode: info.closeCode, reason: info.reason } }),
      error => ({ rejected: describe(error) }),
    )
  const tally = outcomes => {
    const counts = {}
    for (const outcome of outcomes) {
      const key = JSON.stringify(outcome)
      counts[key] = (counts[key] ?? 0) + 1
    }
    return Object.entries(counts).map(([outcome, count]) => ({ count, outcome: JSON.parse(outcome) }))
  }
  // The page sends no SETUP: the relay does not need one for these probes.
  const results = { protocol: null, violation: [], early: [] }

  for (let round = 0; round < rounds; round++) {
    const wt = open()
    await wt.ready
    results.protocol = wt.protocol
    const stream = await wt.createBidirectionalStream()
    // Message type 0x3f with an empty payload.
    await stream.writable.getWriter().write(new Uint8Array([0x3f, 0x00, 0x00]))
    results.violation.push(await closed(wt))
  }

  for (let round = 0; round < rounds; round++) {
    const wt = open()
    await wt.ready
    const stream = await wt.createUnidirectionalStream()
    const writer = stream.getWriter()
    // SUBGROUP_HEADER: type 0x10 (Subgroup ID 0, no properties, priority
    // present), Track Alias 9, Group 0, Publisher Priority 0.
    await writer.write(new Uint8Array([0x10, 0x09, 0x00, 0x00]))
    let outcome = { neverFailed: true }
    for (let attempt = 0; attempt < 40; attempt++) {
      await sleep(100)
      try {
        // Object 0 with a payload of one byte.
        await writer.write(new Uint8Array([0x00, 0x01, 0x61]))
      } catch (error) {
        outcome = { rejected: describe(error) }
        break
      }
    }
    results.early.push(outcome)
    wt.close({ closeCode: 0, reason: 'done' })
    await closed(wt)
  }
  return { protocol: results.protocol, violation: tally(results.violation), early: tally(results.early) }
}

const relay = spawn(
  join(here, '..', '..', 'target', 'debug', 'xmoq-relay'),
  ['--port', options.port, '--cert-file', certFile, '--key-file', keyFile],
  { stdio: ['ignore', 'pipe', 'inherit'] },
)
let listening
const ready = new Promise(resolve => (listening = resolve))
relay.stdout.on('data', chunk => {
  for (const line of chunk.toString().split('\n').filter(Boolean)) {
    console.log(`[relay] ${line}`)
    if (line.startsWith('LISTENING')) listening()
  }
})

// WebTransport needs a secure context; http://localhost counts as one.
const pageServer = createServer((_, response) => {
  response.setHeader('content-type', 'text/html')
  response.end('<!doctype html><title>relay close probe</title>')
})
await new Promise(resolve => pageServer.listen(0, '127.0.0.1', resolve))

let browser
try {
  await ready
  browser = await chromium.launch({ headless: true, channel: 'chromium' })
  console.log(`[driver] browser=chromium version=${browser.version()}`)
  const page = await browser.newPage()
  page.on('console', message => console.log(`[page] ${message.text()}`))
  await page.goto(`http://localhost:${pageServer.address().port}/`)
  const results = await page.evaluate(probe, [base, certificate.sha256Bytes, Number(options.rounds)])
  console.log(`[driver] RESULT ${JSON.stringify(results)}`)
} finally {
  await browser?.close()
  pageServer.close()
  relay.kill('SIGTERM')
}
