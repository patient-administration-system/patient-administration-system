#!/bin/sh
# Development only: set up per-user authentication end to end (DTAC-4) in one command.
#
#   sh scripts/dev-auth.sh          # generates .dev-auth/ once, then runs the token issuer
#
# It generates a throwaway Ed25519 key, publishes the public keys at http://127.0.0.1:8195/keys.json and
# keeps .dev-auth/service.token fresh. Start the API and the UI with the variables it prints.
# For a real deployment keep the private key in your secret store: see operations/deployment.md.
set -eu
cd "$(dirname "$0")/.."
DIR=.dev-auth
BIN="cargo run --quiet --bin pas-token-issuer --"
if [ ! -f "$DIR/private.key" ]; then
  $BIN keygen --dir "$DIR" --kid dev-1
fi
cat >&2 <<MSG

Start the API with:
  PAS_PASETO_KEYS_URL=http://127.0.0.1:8195/keys.json PAS_REQUIRE_AUTH=true cargo run -p patient-administration-system
Start the UI with:
  PAS_API_TOKEN_FILE=$PWD/$DIR/service.token pnpm run dev

MSG
exec $BIN serve --key "$DIR/private.key" --kid dev-1 --sub pas-ui --ttl 300 \
  --token-file "$DIR/service.token" --bind 127.0.0.1:8195
