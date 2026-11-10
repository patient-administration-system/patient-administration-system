# One entry point per CI job (QUAL-41, plan.md §16).
#
# .github/workflows/ci.yml calls these recipes, so a laptop, GitHub, and any second CI run the same commands.
# Installing a toolchain or caching stays in the workflow; every project command belongs here.
#
#   just             list the recipes
#   just ci          every check that needs no image build (needs DATABASE_URL, see `rust-db`)
#   just quick       the fast subset, under two minutes (QUAL-45)
#
# A recipe that needs PostgreSQL takes DATABASE_URL from the environment and refuses to start without it. On this
# project's development stack that is postgres://pas:pas_dev_password@127.0.0.1:5442/pas (see podman/compose.yml).

set shell := ["bash", "-euo", "pipefail", "-c"]

ui_dir := "patient-administration-system-ui-with-svelte"
# Docker where a runner provides it (`docker compose`, GitHub); podman otherwise.
container_cmd := env_var_or_default("CONTAINER_CMD", `command -v podman >/dev/null && echo podman || echo docker`)

[private]
default:
    @just --list

# Everything that needs no image build. Stops at the first failure.
ci: rust rust-db deny ui docs

# Format, generated files, links, UI lint and type check. Under two minutes (QUAL-45).
quick: fmt docs-scripts ui-quick

# Install `quick` as the pre-push hook. Opt-in; `git push --no-verify` still skips it (QUAL-45).
hooks:
    git config core.hooksPath .githooks
    @echo "pre-push hook on. To turn it off: git config --unset core.hooksPath"

# The `rust` job: format, lint, and the tests that need no database.
rust: fmt clippy test

fmt:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

[private]
need-database:
    @test -n "${DATABASE_URL:-}" || { echo "DATABASE_URL is not set" >&2; exit 1; }

migrate: need-database
    cargo run -p patient-administration-system-migrations --bin pas-migrate -- up

# The `rust-db` job without coverage: the tests marked #[ignore], which fail (not skip) without a database.
rust-db: migrate
    cargo test --workspace -- --ignored --test-threads=1

# The `rust-db` job as CI runs it: database and unit tests under coverage, written to lcov.info (QUAL-17).
rust-db-coverage: migrate
    cargo llvm-cov --workspace --no-report -- --ignored --test-threads=1
    cargo llvm-cov --workspace --no-report
    cargo llvm-cov report --lcov --output-path lcov.info

# The line-coverage summary of the last `rust-db-coverage`.
coverage-summary:
    @cargo llvm-cov report --summary-only

# The `fhir-validation` job: real R4 responses against the official validator. Needs java and the network.
fhir: migrate
    java -version
    scripts/validate-fhir.sh

# The `deny` job: advisories, licences and sources, then unused dependencies.
deny:
    cargo deny --all-features check
    cargo machete

# The `ui` job.
[working-directory("patient-administration-system-ui-with-svelte")]
ui: ui-install
    pnpm audit --prod --audit-level high
    pnpm run types
    git diff --exit-code -- src/lib/server/api-types.ts
    pnpm run check
    pnpm run lint
    pnpm run test:coverage
    pnpm run build
    pnpm run bundle:check

[private]
[working-directory("patient-administration-system-ui-with-svelte")]
ui-install:
    pnpm install --frozen-lockfile

[private]
[working-directory("patient-administration-system-ui-with-svelte")]
ui-quick:
    pnpm run check
    pnpm run lint

# The binaries the browser suites run next to the UI.
e2e-build:
    cargo build --locked --bin pas-token-issuer --bin patient-administration-system --bin pas-migrate

# PostgreSQL and Keycloak. On a laptop, `podman ps` first and pass that stack's port variables (AGENTS.md).
e2e-up:
    {{ container_cmd }} compose -f podman/compose.yml up -d

# Wait for Keycloak's realm, for up to three minutes.
e2e-wait:
    #!/usr/bin/env bash
    set -euo pipefail
    url="${KEYCLOAK_URL:-http://localhost:8180}/realms/pas/.well-known/openid-configuration"
    for _ in $(seq 1 60); do
      if curl -sf "$url" -o /dev/null; then exit 0; fi
      sleep 3
    done
    {{ container_cmd }} compose -f podman/compose.yml logs keycloak
    echo "Keycloak did not become ready" >&2
    exit 1

# The `ui-e2e` job's two suites: mocked API, then the real API (QUAL-20). Needs the browsers and `e2e-up`.
e2e: e2e-suite e2e-real-api

[private]
[working-directory("patient-administration-system-ui-with-svelte")]
e2e-suite:
    pnpm exec playwright test --project=chromium --project=mobile

[private]
[working-directory("patient-administration-system-ui-with-svelte")]
e2e-real-api:
    pnpm exec playwright test -c playwright.real-api.config.ts

# The `docs` job. CI also runs lychee through its pinned action; this runs lychee when it is installed.
docs: docs-scripts script-tests
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v lychee >/dev/null; then
      lychee --offline --config lychee.toml .
    else
      echo "lychee is not installed: the offline link check was NOT run (CI runs it)" >&2
    fi

[private]
docs-scripts:
    python3 scripts/check-links.py
    scripts/generate-status.sh --check
    python3 scripts/generate-llms.py --check
    python3 scripts/generate-schema-map.py --check
    python3 scripts/check-hazard-tests.py

# Tests of the local gate and the hook; no database or build needed.
script-tests:
    scripts/ci-local-test.sh
    python3 scripts/check-hazard-tests.py --self-test

# The whole local gate on one commit, in its own worktree, with its own database and build directory (QUAL-42).
gate *args:
    scripts/ci-local.sh {{ args }}

# Rewrite every generated file. Run before committing a change to docs or tests.
generate:
    scripts/generate-status.sh
    python3 scripts/generate-llms.py
    python3 scripts/generate-schema-map.py

# The `workflows` job.
workflows:
    {{ container_cmd }} run --rm -v "$PWD:/repo" --workdir /repo docker.io/rhysd/actionlint:1.7.12 -color
    pipx run zizmor==1.31.1 --offline --min-severity low .github/workflows

# The `restore` job's drill: needs a migrated database in the container named by PG_CONTAINER.
restore: need-pg-container
    {{ container_cmd }} exec -i "$PG_CONTAINER" psql -U pas -d pas -v ON_ERROR_STOP=1 -q < scripts/restore-seed.sql
    scripts/restore-test.sh
    @if RESTORE_DRILL_TAMPER=1 scripts/restore-test.sh; then echo "the drill passed although a restored row was altered" >&2; exit 1; fi

[private]
need-pg-container:
    @test -n "${PG_CONTAINER:-}" || { echo "PG_CONTAINER is not set" >&2; exit 1; }

# Start a throwaway PostgreSQL for `restore`, as CI does. Refuses if the name is taken; a laptop can pass its own.
restore-up name="pg" port="5432":
    #!/usr/bin/env bash
    set -euo pipefail
    podman run -d --name "{{ name }}" -e POSTGRES_USER=pas -e POSTGRES_PASSWORD=pas_dev_password \
      -e POSTGRES_DB=pas -p "{{ port }}:5432" docker.io/library/postgres:18
    for _ in $(seq 1 30); do
      if podman exec "{{ name }}" pg_isready -U pas; then exit 0; fi
      sleep 2
    done
    echo "PostgreSQL did not become ready" >&2
    exit 1

# The `container` job for one image: `just container server` or `just container ui`.
container image:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ image }}" in
      server) file=patient-administration-system-with-rust/Containerfile; context=. ;;
      ui) file={{ ui_dir }}/Containerfile; context={{ ui_dir }} ;;
      *) echo "unknown image: {{ image }} (server or ui)" >&2; exit 1 ;;
    esac
    podman build --format docker -f "$file" -t "pas-{{ image }}" "$context"

# The `fresh-clone` job's checks and builds, from the tree as it stands.
fresh-clone: lockfile-checks
    cargo build --workspace --locked
    cd {{ ui_dir }} && pnpm install --frozen-lockfile && pnpm run build

# The toolchain pin and the one lockfile rule (QUAL-3). RUST_TOOLCHAIN is checked only when it is set, as in CI.
[private]
lockfile-checks:
    #!/usr/bin/env bash
    set -euo pipefail
    pinned="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
    if [[ -n "${RUST_TOOLCHAIN:-}" && "$pinned" != "$RUST_TOOLCHAIN" ]]; then
      echo "rust-toolchain.toml ($pinned) != CI ($RUST_TOOLCHAIN)" >&2
      exit 1
    fi
    extra="$(git ls-files '*/Cargo.lock')"
    test -z "$extra" || { echo "crate-level lockfiles: $extra" >&2; exit 1; }
    grep -q -- '--locked' patient-administration-system-with-rust/Containerfile
