#!/usr/bin/env bash
# Create the local code-signing identity dev builds are signed with
# (DEV.md "Sign dev builds"): a self-signed "oxplow dev" certificate and its
# key in the login keychain, trusted for code signing. Run it once per Mac.
#
# `bun run build:dev` signs the debug daemon with it, so the keychain sees
# every rebuild as the same app: after one Always Allow, oxplow stops asking
# for the AI keys. macOS asks for your password once, to trust it.
#
#   sh scripts/dev-signing-identity.sh     # OXPLOW_DEV_SIGNING_IDENTITY names another
set -euo pipefail

name="${OXPLOW_DEV_SIGNING_IDENTITY:-oxplow dev}"
keychain="$HOME/Library/Keychains/login.keychain-db"

if [ "$(uname)" != Darwin ]; then
  echo "dev-signing-identity: only macOS signs dev builds" >&2
  exit 2
fi
if security find-identity -p codesigning "$keychain" | grep -qF "\"$name\""; then
  echo "\"$name\" is already in your login keychain: nothing to do."
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat >"$work/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -config "$work/cert.cnf" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
# A throwaway password: the file lives only for the import.
openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" \
  -name "$name" -out "$work/identity.p12" -passout pass:oxplow-dev
security import "$work/identity.p12" -k "$keychain" -P oxplow-dev -T /usr/bin/codesign
security add-trusted-cert -p codeSign -k "$keychain" "$work/cert.pem"

echo "Created \"$name\". Quit oxplow and run \`bun run run:dev\`; at the next"
echo "keychain prompt choose Always Allow, and later builds won't ask."
