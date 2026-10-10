// Scenarios against XmoQ's own relay that the one-Object transfer of run.mjs does
// not reach: a publisher that leaves first, a subscriber that leaves first, and
// a subscription to a track nobody published. Uses XmoQ's own Rust client on
// both sides; it prints what the relay did to each of them.
//
// Usage: node relay-scenarios.mjs [--port N]
//
// Needs `cargo build --workspace --locked` and the certificate that run.mjs
// creates under ../cert.

import { spawn } from 'node:child_process'
import { X509Certificate, createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const here = dirname(fileURLToPath(import.meta.url))
const spikeRoot = join(here, '..')
const targetDir = join(spikeRoot, '..', '..', 'target', 'debug')
const certFile = join(spikeRoot, 'cert', 'cert.pem')
const keyFile = join(spikeRoot, 'cert', 'key.pem')
const STEP_TIMEOUT_MS = 20_000

const { values: options } = parseArgs({ options: { port: { type: 'string', default: '44420' } } })
const certSha256 = createHash('sha256').update(new X509Certificate(readFileSync(certFile)).raw).digest('hex')
const relayUrl = `https://127.0.0.1:${options.port}`
const payloadHex = Buffer.from('xmoq-relay-scenario').toString('hex')

function log(source, line) {
  console.log(`[${source}] ${line}`)
}

/** Spawns a child, echoes its lines, and lets callers wait for one that matches. */
function spawnLogged(source, command, args) {
  const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'pipe'] })
  const lines = []
  const waiters = []
  const exited = new Promise(resolve => child.once('exit', code => resolve(code)))
  const onLine = line => {
    lines.push(line)
    log(source, line)
    for (const waiter of [...waiters]) {
      const match = line.match(waiter.pattern)
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
      const seen = lines.map(line => line.match(pattern)).find(Boolean)
      if (seen) return Promise.resolve(seen)
      let timer
      const timeout = new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`timed out waiting for ${source} to print ${pattern}`)), STEP_TIMEOUT_MS)
      })
      const matched = new Promise(resolve => waiters.push({ pattern, resolve }))
      return Promise.race([matched, timeout]).finally(() => clearTimeout(timer))
    },
  }
}

// Every process this script starts, so that none outlives it.
const children = []

function client(label, mode, extraArgs = []) {
  const spawned = spawnLogged(label, join(targetDir, 'xmoq-moqt-client'), [
    '--mode', mode, '--url', relayUrl, '--cert-sha256', certSha256,
    '--namespace', 'xmoq/scenario', '--track', 'object', ...extraArgs,
  ])
  children.push(spawned)
  return spawned
}

async function publisher(label) {
  const pub = client(label, 'publish', ['--payload-hex', payloadHex])
  await pub.waitForLine(/^PUBLISHED /)
  return {
    ...pub,
    async send() {
      pub.child.stdin.write('send\n')
      await pub.waitForLine(/^SENT /)
    },
  }
}

async function subscriber(label, objects) {
  const sub = client(label, 'subscribe', ['--objects', String(objects)])
  await sub.waitForLine(/^SUBSCRIBED /)
  return sub
}

const scenarios = {
  // The publisher is killed, without PUBLISH_DONE. The relay ends the track for
  // the subscriber, which is still waiting for a second Object.
  async 'publisher disconnects first'() {
    const pub = await publisher('pub')
    const sub = await subscriber('sub', 2)
    await pub.send()
    await sub.waitForLine(/^OBJECT group=0 /)
    pub.child.kill('SIGKILL')
    const [, done] = await sub.waitForLine(/^PUBLISH_DONE (.*)$/)
    const [, ended] = await sub.waitForLine(/^SUBSCRIPTION_ENDED (.*)$/)
    return { publishDone: done, subscriptionEnded: ended, subscriberExit: await sub.exited }
  },

  // One of two subscribers leaves after the first Object. The publisher and
  // the other subscriber go on: a second Object still arrives.
  async 'subscriber disconnects first'() {
    const pub = await publisher('pub')
    const leaver = await subscriber('sub-leaves', 1)
    const stayer = await subscriber('sub-stays', 2)
    await pub.send()
    await leaver.waitForLine(/^OBJECT group=0 /)
    await stayer.waitForLine(/^OBJECT group=0 /)
    const leaverExit = await leaver.exited
    await pub.send()
    await stayer.waitForLine(/^OBJECT group=1 /)
    const stayerExit = await stayer.exited
    pub.child.stdin.end()
    return { leaverExit, stayerExit, publisherExit: await pub.exited }
  },

  // The same, with the subscriber killed instead of leaving in order.
  async 'subscriber is killed'() {
    const pub = await publisher('pub')
    const victim = await subscriber('sub-killed', 5)
    const stayer = await subscriber('sub-stays', 2)
    await pub.send()
    await victim.waitForLine(/^OBJECT group=0 /)
    await stayer.waitForLine(/^OBJECT group=0 /)
    victim.child.kill('SIGKILL')
    await victim.exited
    await pub.send()
    await stayer.waitForLine(/^OBJECT group=1 /)
    const stayerExit = await stayer.exited
    pub.child.stdin.end()
    return { stayerExit, publisherExit: await pub.exited }
  },

  // Nobody published the track: the relay rejects the SUBSCRIBE.
  async 'subscribe to a track nobody published'() {
    const sub = client('sub', 'subscribe')
    const [, error] = await sub.waitForLine(/SUBSCRIBE rejected: (.*)$/)
    return { rejected: error, subscriberExit: await sub.exited }
  },
}

const relay = spawnLogged('relay', join(targetDir, 'xmoq-relay'), [
  '--port', options.port, '--cert-file', certFile, '--key-file', keyFile,
])
children.push(relay)
let failed = false
try {
  await relay.waitForLine(/^LISTENING /)
  for (const [name, run] of Object.entries(scenarios)) {
    log('driver', `SCENARIO ${name}`)
    try {
      log('driver', `RESULT ${JSON.stringify({ scenario: name, ...(await run()) })}`)
    } catch (error) {
      failed = true
      log('driver', `FAILED ${name}: ${error.message}`)
    }
    // Lets the relay finish with the sessions of this scenario before the next.
    await new Promise(resolve => setTimeout(resolve, 500))
  }
} finally {
  for (const { child } of children) child.kill('SIGKILL')
  await Promise.all(children.map(({ exited }) => exited))
}
if (failed) process.exitCode = 1
