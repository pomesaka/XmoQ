// Interop driver: pairs web-transport-quinn with the wtransport-based peers
// XmoQ already runs against.
//
//   node spikes/wt-library-comparison/interop.mjs [--port 44430]
//
//   1. wtq-client -> moqtail relay 0.15.0, up to the MOQT SETUP exchange
//   2. wtq-client with a wrong certificate hash -> moqtail relay (must fail)
//   3. wtq-client probe -> wt-server (what each library sees of the other's codes)
//   4. wtq-client probe -> wtq-server
//   5. moqtail Rust client, XmoQ's own client and the moqtail-ts page -> wtq-server
//
// Needs the relay, the Rust client and the browser install of spikes/moqtail,
// and `cargo build --workspace --locked` for XmoQ's own client.
import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

import { build } from '../moqtail/browser/node_modules/esbuild/lib/main.js'
import { chromium } from '../moqtail/browser/node_modules/playwright/index.mjs'
import { certFile, ensureCertificate, keyFile } from './cert.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const moqtailSpike = join(here, '..', 'moqtail')
const bin = name => join(here, 'target', 'debug', name)
const relayBin = join(moqtailSpike, '.tools', 'bin', 'relay')
const moqtailClient = join(moqtailSpike, 'rust-client', 'target', 'debug', 'xmoq-moqtail-spike')
const ownClient = join(here, '..', '..', 'target', 'debug', 'xmoq-moqt-client')

const { values: options } = parseArgs({ options: { port: { type: 'string', default: '44430' } } })
const basePort = Number(options.port)
const certificate = ensureCertificate()
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))

/** Spawns a child, echoes its lines under a label and resolves `started` on a matching line. */
function start(label, command, args, startedPattern) {
  const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'] })
  let markStarted
  const started = new Promise(resolve => (markStarted = resolve))
  for (const stream of [child.stdout, child.stderr]) {
    let buffered = ''
    stream.setEncoding('utf8')
    stream.on('data', chunk => {
      buffered += chunk
      const lines = buffered.split('\n')
      buffered = lines.pop() ?? ''
      for (const line of lines.filter(Boolean)) {
        const clean = line.replace(/\u001b\[[0-9;]*m/g, '')
        console.log(`[${label}] ${clean}`)
        if (startedPattern?.test(clean)) markStarted()
      }
    })
  }
  const exited = new Promise(resolve => child.once('exit', code => resolve(code)))
  return { child, started, exited }
}

/** Runs a client to its end, or kills it after 15 seconds. */
async function run(label, command, args) {
  const client = start(label, command, args)
  const code = await Promise.race([client.exited, sleep(15_000).then(() => 'timeout')])
  if (code === 'timeout') client.child.kill('SIGTERM')
  console.log(`[driver] ${label} exit=${code}`)
}

async function stop(server, signal = 'SIGTERM') {
  server.child.kill(signal)
  await server.exited
}

const url = port => `https://127.0.0.1:${port}`
const moqtArgs = port => [
  '--mode', 'subscribe', '--url', url(port), '--cert-sha256', certificate.sha256Hex,
  '--namespace', 'xmoq/phase0', '--track', 'object',
]

{
  const port = basePort
  const relay = start(
    'relay',
    relayBin,
    ['--port', String(port), '--host', '127.0.0.1', '--cert-file', certFile, '--key-file', keyFile,
      '--log-folder', join(here, '.logs', 'relay')],
    /is running on/,
  )
  await relay.started
  await run('1 wtq-client->relay', bin('wtq-client'), ['setup', url(port), certificate.sha256Hex])
  await run('2 wtq-client wrong hash', bin('wtq-client'), ['setup', url(port), '00'.repeat(32)])
  await stop(relay, 'SIGINT')
}
{
  const port = basePort + 1
  const server = start('wt-server', bin('wt-server'), [String(port), certFile, keyFile], /^LISTENING/)
  await server.started
  await run('3 wtq-client->wt-server', bin('wtq-client'), ['probe', url(port), certificate.sha256Hex])
  await stop(server)
}
{
  const port = basePort + 2
  const server = start('wtq-server', bin('wtq-server'), [String(port), certFile, keyFile], /^LISTENING/)
  await server.started
  await run('4 wtq-client->wtq-server', bin('wtq-client'), ['probe', url(port), certificate.sha256Hex])
  await run('5a moqtail-rs->wtq-server', moqtailClient, moqtArgs(port))
  await run('5b own-client->wtq-server', ownClient, moqtArgs(port))

  // The moqtail-ts page of the moqtail spike, unchanged.
  const outfile = join(here, '.logs', 'page.js')
  await build({
    entryPoints: [join(moqtailSpike, 'browser', 'src', 'page.ts')],
    bundle: true, format: 'esm', target: 'es2022', keepNames: true, outfile, logLevel: 'warning',
  })
  const files = {
    '/': { type: 'text/html', body: readFileSync(join(moqtailSpike, 'browser', 'index.html')) },
    '/page.js': { type: 'text/javascript', body: readFileSync(outfile) },
  }
  const pageServer = createServer((request, response) => {
    const file = files[request.url ?? '']
    if (!file) return response.writeHead(404).end()
    response.writeHead(200, { 'content-type': file.type }).end(file.body)
  })
  await new Promise(resolve => pageServer.listen(0, '127.0.0.1', resolve))
  const browser = await chromium.launch({ headless: true, channel: 'chromium' })
  try {
    const page = await browser.newPage()
    page.on('console', message => console.log(`[5c moqtail-ts] ${message.text()}`))
    page.on('pageerror', error => console.log(`[5c moqtail-ts] pageerror: ${error.message}`))
    await page.goto(`http://localhost:${pageServer.address().port}/`)
    await page.waitForFunction(() => typeof window.xmoq === 'object')
    const outcome = await Promise.race([
      page
        .evaluate(([relayUrl, hash]) => window.xmoq.connect(relayUrl, hash), [url(port), certificate.sha256Hex])
        .then(() => 'connect() resolved', error => `connect() rejected: ${error.message}`),
      sleep(15_000).then(() => 'timeout'),
    ])
    console.log(`[driver] 5c moqtail-ts->wtq-server chromium ${browser.version()}: ${outcome}`)
    await sleep(3_000)
  } finally {
    await browser.close()
    await new Promise(resolve => pageServer.close(resolve))
    await stop(server)
  }
}
