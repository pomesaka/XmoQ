#!/usr/bin/env python3
"""Makes a token of the relay's experiment in authorization (crates/relay/src/auth.rs), as a
room would: the Token structure of draft-ietf-moq-transport-18 §10.2.2, in hex, for
`xmoq-moqt-client --auth-token-hex`.

    mint.py <key, hex> <namespace, a/b> <pub|sub> <seconds it is good for>
"""
import hashlib
import hmac
import sys
import time

key, namespace, role, seconds = sys.argv[1:5]
signed = f"v1|{namespace}|{role}|{int(time.time()) + int(seconds)}"
mac = hmac.new(bytes.fromhex(key), signed.encode(), hashlib.sha256).hexdigest()
# Alias Type USE_VALUE (0x3), Token Type 0, and the value.
print((b"\x03\x00" + f"{signed}|{mac}".encode()).hex())
