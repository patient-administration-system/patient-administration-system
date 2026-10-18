# Install and run (development)

Needs: Rust (the version in `rust-toolchain.toml`), Node 26 with pnpm, Podman.

```sh
# 1. PostgreSQL 18 and Keycloak 26
podman compose -f podman/compose.yml up -d
#   ports busy?  PAS_PG_PORT=5442 PAS_KEYCLOAK_PORT=8190 podman compose -f podman/compose.yml up -d

# 2. Database schema
export DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas
cargo run -p patient-administration-system-migrations --bin pas-migrate -- up

# 3. Per-user authentication (recommended): a token issuer for the API, in its own terminal
sh scripts/dev-auth.sh
#    ...then the PAS API (http://localhost:8080, Swagger UI at /swagger-ui/) requiring those tokens
PAS_PASETO_KEYS_URL=http://127.0.0.1:8195/keys.json PAS_REQUIRE_AUTH=true \
  cargo run -p patient-administration-system
#    Without the two variables the API runs in trusted-caller mode: fine on a laptop, never in a pilot.

# 4. The UI (http://localhost:5173)
cd patient-administration-system-ui-with-svelte
cp .env.example .env     # set PAS_API_URL, OIDC_*, and PAS_API_TOKEN_FILE=<repo>/.dev-auth/service.token
pnpm install && pnpm run dev
```

Development users (Keycloak realm `pas`): `patient` / `patient`, `admin` / `admin`. These are for development only.

Tests: see [CONTRIBUTING.md](CONTRIBUTING.md). Running it for real: [operations/deployment.md](operations/deployment.md), after reading [governance/production-readiness.md](governance/production-readiness.md).
