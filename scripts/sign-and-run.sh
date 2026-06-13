#!/usr/bin/env bash
# Cargo `runner` hook: sign the freshly-built binary with a stable code-signing
# identity + fixed identifier, then exec it.
#
# Why: an ad-hoc signature's identity is its cdhash, which changes on every
# rebuild, so macOS revokes the Accessibility (TCC) grant each time. Signing
# with a real identity + stable identifier keeps the grant across rebuilds —
# grant Accessibility once and you're done (plan §5/§14).
#
# Gracefully no-ops (just runs the binary) when no identity is available, so it
# never breaks `cargo run`/`cargo test` on a machine without a cert.
set -u

BIN="${1:-}"
[ -n "$BIN" ] && shift

IDENTIFIER="com.emulstick.desktop"

# Pick the signing identity: explicit override, else the first Apple Development
# cert in the keychain.
ID="${EMULSTICK_SIGN_ID:-}"
if [ -z "$ID" ]; then
  ID=$(security find-identity -v -p codesigning 2>/dev/null \
        | grep "Apple Development" | head -1 | awk '{print $2}')
fi

if [ -n "$BIN" ] && [ -n "$ID" ]; then
  codesign --force --sign "$ID" --identifier "$IDENTIFIER" "$BIN" >/dev/null 2>&1 || true
fi

exec "$BIN" "$@"
