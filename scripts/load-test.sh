#!/usr/bin/env bash
# Start a release server on a spare port with the rate limit off, run pas-load against it, stop the server
# (PERF-1, NFR-29). Every argument goes to pas-load, so `scripts/load-test.sh --profile load --report r.json`.
#
# Needs DATABASE_URL (a migrated database). LOAD_PORT picks the port (default 8095). The server's log is
# written to target/load-server.log, and the run's report to target/load-report.json unless --report is given.
# To load-test a server you started yourself, run `cargo run --release -p patient-administration-system-load
# -- --url URL` instead.
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL is required: the load test makes its rows in the database the server uses}"
port="${LOAD_PORT:-8095}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    echo "port $port is in use: set LOAD_PORT to a free one" >&2
    exit 2
fi

cargo build --release -p patient-administration-system --bin patient-administration-system \
    -p patient-administration-system-load --bin pas-load

# The rate limit exists to stop one client sending thousands of requests a minute, which is what a load
# test is; with it on, the numbers would measure the limit. Logging at warn keeps the log from being the
# bottleneck.
SERVER_HOST=127.0.0.1 SERVER_PORT="$port" PAS_RATE_LIMIT_RPM=0 RUST_LOG="${LOAD_RUST_LOG:-warn}" \
    target/release/patient-administration-system >target/load-server.log 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true' EXIT

for _ in $(seq 1 60); do
    if curl -fsS "http://127.0.0.1:$port/api/health" >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "$server" 2>/dev/null; then
        echo "the server stopped while starting; its log:" >&2
        cat target/load-server.log >&2
        exit 2
    fi
    sleep 1
done

args=("$@")
if [[ " ${args[*]-} " != *" --report "* ]]; then
    args+=(--report target/load-report.json)
fi
target/release/pas-load --url "http://127.0.0.1:$port" "${args[@]}"
