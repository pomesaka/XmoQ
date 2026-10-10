// Probe driver: starts one of the probe servers of this directory, opens a page
// in Chromium and prints what the page and the server see.
//
//   node spikes/wt-library-comparison/probe.mjs --library wtq|wtq-default|wt|wt-map|wt-map-default [--port 44400] [--rounds 20]
//
// `wt-map` is the wtransport server with stream codes mapped by hand. The page
// probes for close, reset, stop, observe and protocol follow probe.mjs of the
// wt-error-codes spike, so the two records can be compared.
//
// Uses the Playwright install of spikes/moqtail.
import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

import { chromium } from '../moqtail/browser/node_modules/playwright/index.mjs'
import { certFile, ensureCertificate, keyFile } from './cert.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const SERVERS = {
  wtq: { binary: 'wtq-server', args: [] },
  'wtq-default': { binary: 'wtq-server', args: ['--default-transport'] },
  wt: { binary: 'wt-server', args: [] },
  'wt-map': { binary: 'wt-server', args: ['--map'] },
  // Without the limits of src/common.rs, as the wt-error-codes probe ran. The
  // streams and window probes then show the library defaults.
  'wt-map-default': { binary: 'wt-server', args: ['--map', '--default-transport'] },
}
const { values: options } = parseArgs({
  options: {
    library: { type: 'string', default: 'wtq' },
    port: { type: 'string', default: '44400' },
    // How often the stop and observe probes are repeated.
    rounds: { type: 'string', default: '20' },
  },
})
const serverSpec = SERVERS[options.library]
if (!serverSpec) throw new Error(`unknown --library ${options.library}`)

const certificate = ensureCertificate()
const base = `https://127.0.0.1:${options.port}`

// Runs in the page. Returns plain data only.
async function probe([base, certHash, rounds]) {
  const pin = { serverCertificateHashes: [{ algorithm: 'sha-256', value: new Uint8Array(certHash) }] }
  const open = (path, extra = {}) => new WebTransport(base + path, { ...pin, ...extra })
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
  const describe = error => ({
    name: error?.name,
    message: error?.message,
    source: error?.source,
    streamErrorCode: error?.streamErrorCode,
  })
  const readToEnd = async readable => {
    const reader = readable.getReader()
    let text = ''
    for (;;) {
      const { done, value } = await reader.read()
      if (done) return text
      text += new TextDecoder().decode(value)
    }
  }
  // Counts equal outcomes, so a repeated probe reports how often each one occurred.
  const tally = outcomes => {
    const counts = {}
    for (const outcome of outcomes) {
      const key = JSON.stringify(outcome)
      counts[key] = (counts[key] ?? 0) + 1
    }
    return Object.entries(counts).map(([outcome, count]) => ({ count, outcome: JSON.parse(outcome) }))
  }
  const results = {}

  {
    const wt = open('/close')
    await wt.ready
    results.close = await wt.closed.then(
      info => ({ resolved: { closeCode: info.closeCode, reason: info.reason } }),
      error => ({ rejected: describe(error) }),
    )
  }
  {
    const wt = open('/reset')
    await wt.ready
    const streams = wt.incomingUnidirectionalStreams.getReader()
    const { value: stream } = await streams.read()
    const reader = stream.getReader()
    try {
      for (;;) {
        const { done } = await reader.read()
        if (done) {
          results.reset = { finished: true }
          break
        }
      }
    } catch (error) {
      results.reset = { rejected: describe(error) }
    }
    wt.close()
  }
  {
    const outcomes = []
    for (let round = 0; round < rounds; round++) {
      const wt = open('/stop')
      await wt.ready
      const stream = await wt.createUnidirectionalStream()
      const writer = stream.getWriter()
      try {
        for (let i = 0; i < 200; i++) {
          await writer.write(new Uint8Array(1024))
          await sleep(10)
        }
        outcomes.push({ neverFailed: true })
      } catch (error) {
        outcomes.push({ rejected: describe(error) })
      }
      wt.close()
    }
    results.stop = tally(outcomes)
  }
  // What the server sees when the page resets a stream and closes with a code.
  for (let round = 0; round < rounds; round++) {
    const wt = open('/observe')
    await wt.ready
    const stream = await wt.createUnidirectionalStream()
    const writer = stream.getWriter()
    await writer.write(new Uint8Array([1]))
    await writer.abort(new WebTransportError({ message: 'abort', streamErrorCode: 0x12 }))
    await sleep(300)
    wt.close({ closeCode: 3, reason: 'bye' })
    await wt.closed.catch(() => {})
  }
  results.observe = 'see the server lines'
  {
    const wt = open('/protocol', { protocols: ['moqt-18'] })
    await wt.ready
    results.protocol = { protocol: wt.protocol }
    const stream = await wt.createUnidirectionalStream()
    await stream.getWriter().close()
    wt.close()
  }
  {
    const wt = open('/bidi')
    await wt.ready
    const opened = await wt.createBidirectionalStream()
    const writer = opened.writable.getWriter()
    await writer.write(new TextEncoder().encode('ping'))
    await writer.close()
    const reply = await readToEnd(opened.readable)

    const incoming = wt.incomingBidirectionalStreams.getReader()
    const { value: accepted } = await incoming.read()
    const fromServer = await readToEnd(accepted.readable)
    const ack = accepted.writable.getWriter()
    await ack.write(new TextEncoder().encode('ack'))
    await ack.close()
    await sleep(300)
    results.bidi = { replyOnOpenedStream: reply, readOnAcceptedStream: fromServer }
    wt.close()
  }
  {
    const wt = open('/streams')
    await wt.ready
    let opened = 0
    for (let i = 0; i < 20; i++) {
      wt.createUnidirectionalStream()
        .then(async stream => {
          await stream.getWriter().write(new Uint8Array([1]))
          opened++
        })
        .catch(() => {})
    }
    await sleep(1000)
    results.streams = { attempted: 20, openedWithin1s: opened }
    await sleep(1200)
    wt.close()
  }
  {
    const wt = open('/window')
    await wt.ready
    const stream = await wt.createUnidirectionalStream()
    const writer = stream.getWriter()
    let written = 0
    ;(async () => {
      for (let i = 0; i < 64; i++) {
        await writer.write(new Uint8Array(16384))
        written += 16384
      }
    })().catch(() => {})
    await sleep(1000)
    results.window = { offeredBytes: 64 * 16384, writesResolvedWithin1s: written }
    await sleep(800)
    wt.close()
  }
  return results
}

const server = spawn(
  join(here, 'target', 'debug', serverSpec.binary),
  [options.port, certFile, keyFile, ...serverSpec.args],
  { stdio: ['ignore', 'pipe', 'inherit'] },
)
let listening
const ready = new Promise(resolve => (listening = resolve))
server.stdout.on('data', chunk => {
  for (const line of chunk.toString().split('\n').filter(Boolean)) {
    console.log(`[server] ${line}`)
    if (line.startsWith('LISTENING')) listening()
  }
})

// WebTransport needs a secure context; http://localhost counts as one.
const pageServer = createServer((_, response) => {
  response.setHeader('content-type', 'text/html')
  response.end('<!doctype html><title>probe</title>')
})
await new Promise(resolve => pageServer.listen(0, '127.0.0.1', resolve))

let browser
try {
  await ready
  browser = await chromium.launch({ headless: true, channel: 'chromium' })
  console.log(`[driver] chromium ${browser.version()} library=${options.library} rounds=${options.rounds}`)
  const page = await browser.newPage()
  page.on('console', message => console.log(`[page] ${message.text()}`))
  await page.goto(`http://localhost:${pageServer.address().port}/`)
  const results = await page.evaluate(probe, [base, certificate.sha256Bytes, Number(options.rounds)])
  console.log(`[driver] RESULT ${JSON.stringify(results)}`)
  await new Promise(resolve => setTimeout(resolve, 500))
} finally {
  await browser?.close()
  await new Promise(resolve => pageServer.close(resolve))
  server.kill('SIGTERM')
}
