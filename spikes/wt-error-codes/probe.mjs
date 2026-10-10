// Probe driver: starts the wtransport server of this directory, opens a page in
// Chromium and prints what the page sees for a session close, a stream reset
// and a STOP_SENDING that carry error codes.
//
//   node spikes/wt-error-codes/probe.mjs [--map] [--port 44390]
//
// Uses the certificate and the Playwright install of spikes/moqtail.
import { spawn } from 'node:child_process'
import { createHash, X509Certificate } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

import { chromium } from '../moqtail/browser/node_modules/playwright/index.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const certDir = join(here, '..', 'moqtail', 'cert')
const { values: options } = parseArgs({
  options: { map: { type: 'boolean', default: false }, port: { type: 'string', default: '44390' } },
})

const certificate = new X509Certificate(readFileSync(join(certDir, 'cert.pem')))
const certHash = [...createHash('sha256').update(certificate.raw).digest()]
const base = `https://127.0.0.1:${options.port}`

// Runs in the page. Returns plain data only.
async function probe([base, certHash]) {
  const open = path =>
    new WebTransport(base + path, {
      serverCertificateHashes: [{ algorithm: 'sha-256', value: new Uint8Array(certHash) }],
    })
  const describe = error => ({
    name: error?.name,
    message: error?.message,
    source: error?.source,
    streamErrorCode: error?.streamErrorCode,
  })
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
    const wt = open('/stop')
    await wt.ready
    const stream = await wt.createUnidirectionalStream()
    const writer = stream.getWriter()
    try {
      for (let i = 0; i < 200; i++) {
        await writer.write(new Uint8Array(1024))
        await new Promise(resolve => setTimeout(resolve, 10))
      }
      results.stop = { neverFailed: true }
    } catch (error) {
      results.stop = { rejected: describe(error) }
    }
    wt.close()
  }
  {
    // What the server sees when the page resets a stream and closes with a code.
    const wt = open('/observe')
    await wt.ready
    const stream = await wt.createUnidirectionalStream()
    const writer = stream.getWriter()
    await writer.write(new Uint8Array([1]))
    await writer.abort(new WebTransportError({ message: 'abort', streamErrorCode: 0x12 }))
    await new Promise(resolve => setTimeout(resolve, 300))
    wt.close({ closeCode: 3, reason: 'bye' })
    await wt.closed.catch(() => {})
    results.observe = 'see the server lines'
  }
  {
    const wt = new WebTransport(base + '/protocol', {
      serverCertificateHashes: [{ algorithm: 'sha-256', value: new Uint8Array(certHash) }],
      protocols: ['moqt-18'],
    })
    await wt.ready
    results.protocol = { protocol: wt.protocol }
    const stream = await wt.createUnidirectionalStream()
    await stream.getWriter().close()
    wt.close()
  }
  return results
}

const server = spawn(
  join(here, 'target', 'debug', 'xmoq-wt-error-codes-spike'),
  [options.port, join(certDir, 'cert.pem'), join(certDir, 'key.pem'), ...(options.map ? ['--map'] : [])],
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
  console.log(`[driver] chromium ${browser.version()} map=${options.map}`)
  const page = await browser.newPage()
  page.on('console', message => console.log(`[page] ${message.text()}`))
  await page.goto(`http://localhost:${pageServer.address().port}/`)
  const results = await page.evaluate(probe, [base, certHash])
  console.log(`[driver] RESULT ${JSON.stringify(results)}`)
  await new Promise(resolve => setTimeout(resolve, 500))
} finally {
  await browser?.close()
  await new Promise(resolve => pageServer.close(resolve))
  server.kill('SIGTERM')
}
