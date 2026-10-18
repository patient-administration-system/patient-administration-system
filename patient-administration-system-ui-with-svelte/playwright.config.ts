import { defineConfig, devices } from "@playwright/test";

// The browser suite runs the production build against a real Keycloak (started with
// `podman compose -f ../podman/compose.yml up -d`; set KEYCLOAK_URL if it is not on 8180)
// and a mock PAS API. No Rust service is needed.
const PORT = Number(process.env.PW_PORT ?? 4173);
const KEYCLOAK = process.env.KEYCLOAK_URL ?? "http://localhost:8180";
const MOCK_PORT = Number(process.env.MOCK_PORT ?? 4181);

export default defineConfig({
  testDir: "tests/e2e",
  timeout: 30_000,
  fullyParallel: false,
  // A real Keycloak login occasionally stalls; one retry keeps CI honest without hiding a real break.
  retries: process.env.CI ? 1 : 0,
  // One worker: the specs share one Keycloak and one mock API whose request log a test reads, so
  // they cannot run side by side.
  workers: 1,
  use: { baseURL: `http://localhost:${PORT}`, trace: "on-first-retry" },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] }, testIgnore: /mobile\.spec\.ts/ },
    // A phone-width audit (390x844, about an iPhone 12 mini), run in isolation with
    // `pnpm exec playwright test --project=mobile`.
    {
      name: "mobile",
      testMatch: /mobile\.spec\.ts/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 390, height: 844 } },
    },
  ],
  webServer: [
    {
      command: "node tests/e2e/support/pas-mock.mjs",
      url: `http://127.0.0.1:${MOCK_PORT}/__health`,
      env: { MOCK_PORT: String(MOCK_PORT) },
      reuseExistingServer: !process.env.CI,
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
        PAS_API_URL: `http://127.0.0.1:${MOCK_PORT}`,
        OIDC_ISSUER: `${KEYCLOAK}/realms/pas`,
        OIDC_CLIENT_ID: "pas-frontend",
        OIDC_CLIENT_SECRET: "pas_dev_client_secret",
        OIDC_END_SESSION: "true",
      },
    },
  ],
});
