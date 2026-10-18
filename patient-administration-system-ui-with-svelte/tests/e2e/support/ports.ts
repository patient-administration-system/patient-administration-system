// Ports the browser suite uses. Another project on the same machine may hold the defaults, so each
// can be overridden: `MOCK_PORT=4191 PW_PORT=4183 pnpm exec playwright test`.
// (The Keycloak realm registers redirect URIs for 3000, 4173 and 5173 only; if you change PW_PORT,
// pick one of those.)
export const MOCK_PORT = Number(process.env.MOCK_PORT ?? 4181);
export const MOCK_URL = `http://127.0.0.1:${MOCK_PORT}`;
