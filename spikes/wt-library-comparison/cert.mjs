// The self-signed certificate the probe servers run on, created on first use.
import { execFileSync } from 'node:child_process'
import { X509Certificate, createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const certDir = join(dirname(fileURLToPath(import.meta.url)), 'cert')
export const certFile = join(certDir, 'cert.pem')
export const keyFile = join(certDir, 'key.pem')

/**
 * Chromium accepts a certificate through `serverCertificateHashes` only if it is
 * ECDSA P-256 and valid for less than two weeks, so the certificate is regenerated
 * whenever it is missing or about to expire.
 */
export function ensureCertificate() {
  const oneDayMs = 24 * 60 * 60 * 1000
  const usable =
    existsSync(certFile) &&
    existsSync(keyFile) &&
    new Date(new X509Certificate(readFileSync(certFile)).validTo).getTime() - Date.now() > oneDayMs
  if (!usable) {
    mkdirSync(certDir, { recursive: true })
    // WHY two steps instead of `req -newkey ec`: with LibreSSL 3.3.6 (macOS
    // /usr/bin/openssl) the one-step form wrote a key that wtransport 0.7.2
    // rejected (recorded in spikes/moqtail/browser/run.mjs).
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
  }
  const digest = createHash('sha256').update(new X509Certificate(readFileSync(certFile)).raw).digest()
  return { sha256Hex: digest.toString('hex'), sha256Bytes: [...digest] }
}
