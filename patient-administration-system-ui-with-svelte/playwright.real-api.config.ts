import { defineConfig, devices } from "@playwright/test";

// The browser tests against the REAL PAS API on a real PostgreSQL database (QUAL-20), not the mock the main
// suite uses: `pnpm exec playwright test -c playwright.real-api.config.ts`. It needs Keycloak (as the main
// suite) and a PostgreSQL database it may write to (REAL_API_DATABASE_URL); it migrates the database, starts
// the API from the workspace with `cargo run` (or PAS_MIGRATE_BIN and PAS_SERVER_BIN), and the specs seed
// what they read. Only
// person-service, a separate service, is a stand-in.
const PORT = Number(process.env.PW_PORT ?? 4173);
const KEYCLOAK = process.env.KEYCLOAK_URL ?? "http://localhost:8180";
const API_PORT = Number(process.env.REAL_API_PORT ?? 4196);
const PERSON_PORT = Number(process.env.PERSON_SERVICE_PORT ?? 4197);
const ISSUER_PORT = Number(process.env.ISSUER_PORT ?? 4195);
const ISSUER_DIR = `${process.cwd()}/test-results/issuer-real`;
export const DATABASE_URL = process.env.REAL_API_DATABASE_URL ?? "postgres://pas:pas_dev_password@localhost:5432/pas";
const API = process.env.PAS_SERVER_BIN ?? "cargo run --quiet --bin patient-administration-system";
const MIGRATE =
  process.env.PAS_MIGRATE_BIN ?? "cargo run --quiet -p patient-administration-system-migrations --bin pas-migrate --";

export default defineConfig({
  testDir: "tests/e2e-real",
  timeout: 60_000,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  use: { baseURL: `http://localhost:${PORT}`, trace: "on-first-retry", ...devices["Desktop Chrome"] },
  webServer: [
    {
      command: "node tests/e2e/support/issuer.mjs",
      url: `http://127.0.0.1:${ISSUER_PORT}/keys.json`,
      env: { ISSUER_PORT: String(ISSUER_PORT), ISSUER_DIR },
      reuseExistingServer: !process.env.CI,
      timeout: 240_000,
    },
    {
      command: "node tests/e2e-real/support/person-service.mjs",
      url: `http://127.0.0.1:${PERSON_PORT}/api/health`,
      env: { PERSON_SERVICE_PORT: String(PERSON_PORT) },
      reuseExistingServer: !process.env.CI,
    },
    {
      // Migrations first, then the API itself, in trusted-caller mode (no per-user keys) on its own port.
      command: `${MIGRATE} up && ${API}`,
      cwd: "..",
      url: `http://127.0.0.1:${API_PORT}/api/health`,
      reuseExistingServer: !process.env.CI,
      timeout: 600_000,
      env: {
        DATABASE_URL,
        SERVER_HOST: "127.0.0.1",
        SERVER_PORT: String(API_PORT),
        PERSON_SERVICE_URL: `http://127.0.0.1:${PERSON_PORT}`,
        PAS_QUIET_HOURS: "off",
      },
    },
    {
      command: "pnpm run build && node container-server.mjs",
      url: `http://localhost:${PORT}/signin`,
      reuseExistingServer: !process.env.CI,
      timeout: 120_000,
      env: {
        PORT: String(PORT),
        HOST: "127.0.0.1",
        PROTOCOL_HEADER: "x-forwarded-proto",
        PAS_API_URL: `http://127.0.0.1:${API_PORT}`,
        PAS_API_TOKEN_FILE: `${ISSUER_DIR}/service.token`,
        OIDC_ISSUER: `${KEYCLOAK}/realms/pas`,
        OIDC_CLIENT_ID: "pas-frontend",
        OIDC_CLIENT_SECRET: "pas_dev_client_secret",
        OIDC_END_SESSION: "true",
      },
    },
  ],
});
