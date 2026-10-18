# Install and run (development)

Needs: Rust (the version in `rust-toolchain.toml`), Node 26 with pnpm, Podman.

```sh
# 1. PostgreSQL 18 and Keycloak 26
podman compose -f podman/compose.yml up -d
#   ports busy?  PAS_PG_PORT=5442 PAS_KEYCLOAK_PORT=8190 podman compose -f podman/compose.yml up -d

# 2. Database schema
export DATABASE_URL=postgres://pas:pas_dev_password@localhost:5432/pas
cargo run -p patient-administration-system-migrations --bin pas-migrate -- up

# 3. The PAS API (http://localhost:8080, Swagger UI at /swagger-ui/)
cargo run -p patient-administration-system

# 4. The UI (http://localhost:5173)
cd patient-administration-system-ui-with-svelte
cp .env.example .env     # set PAS_API_URL, PAS_API_TOKEN, OIDC_*
pnpm install && pnpm run dev
```

Development users (Keycloak realm `pas`): `patient` / `patient`, `admin` / `admin`. These are for development only.

Tests: see [CONTRIBUTING.md](CONTRIBUTING.md). Running it for real: [operations/deployment.md](operations/deployment.md), after reading [governance/production-readiness.md](governance/production-readiness.md).
