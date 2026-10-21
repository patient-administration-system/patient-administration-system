// @vitest-environment node
// The OpenID Connect client (QUAL-19). The ID token is the one thing the BFF trusts from the provider, so
// every refusal is tested with a real RSA key: wrong algorithm, wrong key, a changed claim, wrong issuer,
// audience, nonce or tenant, expired, not yet valid. Discovery, keys and the token endpoint fail closed.
import { createSign, generateKeyPairSync } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import {
  OidcError,
  authorizationUrl,
  discover,
  exchangeCode,
  fetchJwks,
  pkcePair,
  randomToken,
  refreshTokens,
  safeReturnPath,
  verifyIdToken,
  type Jwk,
  type OidcConfig,
  type ProviderMetadata,
} from "#lib/server/oidc.js";
import { createHash } from "node:crypto";

const ISSUER = "https://idp.test/realms/pas";
const CLIENT = "pas-ui";
const NOW = 1_800_000_000;

function rsa(kid: string) {
  const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const jwk = { ...(publicKey.export({ format: "jwk" }) as Jwk), kid, use: "sig" };
  return { privateKey, jwk };
}
const KEY = rsa("k1");
const OTHER = rsa("k2");

const b64 = (o: unknown) => Buffer.from(JSON.stringify(o)).toString("base64url");

function token(claims: Record<string, unknown>, opts: { alg?: string; kid?: string; key?: typeof KEY } = {}): string {
  const head = b64({ alg: opts.alg ?? "RS256", kid: opts.kid ?? "k1", typ: "JWT" });
  const body = b64({ iss: ISSUER, aud: CLIENT, sub: "kc-1", exp: NOW + 300, nonce: "n-1", ...claims });
  const sig = createSign("RSA-SHA256")
    .update(`${head}.${body}`)
    .sign((opts.key ?? KEY).privateKey)
    .toString("base64url");
  return `${head}.${body}.${sig}`;
}

const EXPECT = { issuer: ISSUER, audience: CLIENT, nonce: "n-1", now: NOW };
const refusal = (f: () => unknown): string => {
  try {
    f();
  } catch (e) {
    expect(e).toBeInstanceOf(OidcError);
    return (e as Error).message;
  }
  throw new Error("accepted");
};

describe("verifyIdToken", () => {
  it("accepts a good token and returns its claims", () => {
    const c = verifyIdToken(token({ roles: ["staff"] }), [KEY.jwk, OTHER.jwk], EXPECT);
    expect(c.sub).toBe("kc-1");
    expect(c.roles).toEqual(["staff"]);
  });

  it("accepts an audience list that includes us", () => {
    expect(verifyIdToken(token({ aud: ["other", CLIENT] }), [KEY.jwk], EXPECT).sub).toBe("kc-1");
  });

  it("refuses every algorithm but RS256, including none and HMAC", () => {
    for (const alg of ["none", "HS256", "RS512", "ES256"]) {
      const [, body] = token({}).split(".");
      expect(refusal(() => verifyIdToken(`${b64({ alg, kid: "k1" })}.${body}.`, [KEY.jwk], EXPECT))).toMatch(/RS256/);
    }
  });

  it("refuses a token signed by another key, or with a changed claim", () => {
    expect(refusal(() => verifyIdToken(token({}, { key: OTHER }), [KEY.jwk, OTHER.jwk], EXPECT))).toMatch(/signature/);
    const [h, , s] = token({}).split(".");
    const forged = `${h}.${b64({ iss: ISSUER, aud: CLIENT, sub: "kc-admin", exp: NOW + 300, nonce: "n-1" })}.${s}`;
    expect(refusal(() => verifyIdToken(forged, [KEY.jwk], EXPECT))).toMatch(/signature/);
  });

  it("refuses an unknown kid, and a missing kid when the choice is ambiguous", () => {
    expect(refusal(() => verifyIdToken(token({}, { kid: "nope" }), [KEY.jwk], EXPECT))).toMatch(/no matching/);
    const noKid = (() => {
      const head = b64({ alg: "RS256" });
      const body = b64({ iss: ISSUER, aud: CLIENT, sub: "kc-1", exp: NOW + 300, nonce: "n-1" });
      return `${head}.${body}.${createSign("RSA-SHA256").update(`${head}.${body}`).sign(KEY.privateKey).toString("base64url")}`;
    })();
    expect(verifyIdToken(noKid, [KEY.jwk], EXPECT).sub).toBe("kc-1");
    expect(refusal(() => verifyIdToken(noKid, [KEY.jwk, OTHER.jwk], EXPECT))).toMatch(/no matching/);
    expect(refusal(() => verifyIdToken(token({}), [{ ...KEY.jwk, use: "enc" }], EXPECT))).toMatch(/no matching/);
  });

  it("refuses the wrong issuer, audience, nonce, tenant or an empty subject", () => {
    expect(refusal(() => verifyIdToken(token({ iss: "https://evil.test" }), [KEY.jwk], EXPECT))).toMatch(/issuer/);
    expect(refusal(() => verifyIdToken(token({ aud: "someone-else" }), [KEY.jwk], EXPECT))).toMatch(/audience/);
    expect(refusal(() => verifyIdToken(token({ nonce: "n-2" }), [KEY.jwk], EXPECT))).toMatch(/nonce/);
    expect(refusal(() => verifyIdToken(token({ nonce: undefined }), [KEY.jwk], EXPECT))).toMatch(/nonce/);
    expect(refusal(() => verifyIdToken(token({ sub: "" }), [KEY.jwk], EXPECT))).toMatch(/subject/);
    expect(refusal(() => verifyIdToken(token({ tid: "t-2" }), [KEY.jwk], { ...EXPECT, tenantId: "t-1" }))).toMatch(
      /tenant/,
    );
    expect(verifyIdToken(token({ tid: "t-1" }), [KEY.jwk], { ...EXPECT, tenantId: "t-1" }).tid).toBe("t-1");
  });

  it("allows a minute of clock skew and no more", () => {
    expect(verifyIdToken(token({ exp: NOW - 59 }), [KEY.jwk], EXPECT).sub).toBe("kc-1");
    expect(refusal(() => verifyIdToken(token({ exp: NOW - 61 }), [KEY.jwk], EXPECT))).toMatch(/expired/);
    expect(verifyIdToken(token({ nbf: NOW + 59 }), [KEY.jwk], EXPECT).sub).toBe("kc-1");
    expect(refusal(() => verifyIdToken(token({ nbf: NOW + 61 }), [KEY.jwk], EXPECT))).toMatch(/not yet/);
  });

  it("refuses what is not a compact JWS", () => {
    expect(refusal(() => verifyIdToken("a.b", [KEY.jwk], EXPECT))).toMatch(/compact/);
    expect(refusal(() => verifyIdToken("!!.??.xx", [KEY.jwk], EXPECT))).toMatch(/malformed/);
  });
});

const META: ProviderMetadata = {
  issuer: ISSUER,
  authorization_endpoint: `${ISSUER}/protocol/openid-connect/auth`,
  token_endpoint: `${ISSUER}/protocol/openid-connect/token`,
  jwks_uri: `${ISSUER}/protocol/openid-connect/certs`,
};
const CFG: OidcConfig = { issuer: ISSUER, clientId: CLIENT, clientSecret: "secret", scopes: "profile email openid" };

const answering = (status: number, body: unknown) => {
  const f = vi.fn(async (_u: RequestInfo | URL, _i?: RequestInit) => new Response(JSON.stringify(body), { status }));
  return f;
};
const unreachable = vi.fn(async () => {
  throw new TypeError("fetch failed");
}) as unknown as typeof fetch;

describe("discover", () => {
  it("reads the metadata of the configured issuer", async () => {
    const f = answering(200, META);
    await expect(discover(f as unknown as typeof fetch, `${ISSUER}/`)).resolves.toEqual(META);
    expect(String(f.mock.calls[0]![0])).toBe(`${ISSUER}/.well-known/openid-configuration`);
  });

  it("refuses another issuer, a templated issuer, missing endpoints, errors and an unreachable provider", async () => {
    const cases: [typeof fetch, string][] = [
      [answering(200, { ...META, issuer: "https://evil.test" }) as unknown as typeof fetch, "issuer_mismatch"],
      [
        answering(200, {
          ...META,
          issuer: "https://login.microsoftonline.com/{tenantid}/v2.0",
        }) as unknown as typeof fetch,
        "issuer_mismatch",
      ],
      [answering(200, { ...META, jwks_uri: undefined }) as unknown as typeof fetch, "metadata"],
      [answering(500, {}) as unknown as typeof fetch, "metadata"],
      [unreachable, "metadata"],
    ];
    for (const [f, code] of cases) await expect(discover(f, ISSUER)).rejects.toMatchObject({ code });
  });
});

describe("fetchJwks", () => {
  it("returns the keys, and fails closed otherwise", async () => {
    await expect(fetchJwks(answering(200, { keys: [KEY.jwk] }) as unknown as typeof fetch, META)).resolves.toHaveLength(
      1,
    );
    for (const f of [answering(200, {}), answering(404, {}), unreachable]) {
      await expect(fetchJwks(f as unknown as typeof fetch, META)).rejects.toMatchObject({ code: "jwks" });
    }
  });
});

describe("the token endpoint", () => {
  it("exchanges a code with the PKCE verifier and the client secret", async () => {
    const f = answering(200, { access_token: "at", id_token: "it", expires_in: 300 });
    await expect(
      exchangeCode(f as unknown as typeof fetch, META, CFG, { code: "c", verifier: "v", redirectUri: "https://ui/cb" }),
    ).resolves.toMatchObject({ access_token: "at" });
    const body = new URLSearchParams(String(f.mock.calls[0]![1]!.body));
    expect(Object.fromEntries(body)).toEqual({
      grant_type: "authorization_code",
      code: "c",
      redirect_uri: "https://ui/cb",
      code_verifier: "v",
      client_id: CLIENT,
      client_secret: "secret",
    });
  });

  it("refreshes without a secret for a public client", async () => {
    const f = answering(200, { access_token: "at2" });
    await refreshTokens(f as unknown as typeof fetch, META, { ...CFG, clientSecret: undefined }, "rt");
    const body = new URLSearchParams(String(f.mock.calls[0]![1]!.body));
    expect(body.get("grant_type")).toBe("refresh_token");
    expect(body.get("refresh_token")).toBe("rt");
    expect(body.has("client_secret")).toBe(false);
  });

  it("fails closed on an error, a reply with no access token, or no answer", async () => {
    for (const f of [answering(400, { error: "invalid_grant" }), answering(200, { id_token: "x" }), unreachable]) {
      await expect(refreshTokens(f as unknown as typeof fetch, META, CFG, "rt")).rejects.toMatchObject({
        code: "token_endpoint",
      });
    }
  });
});

describe("authorizationUrl and PKCE", () => {
  it("asks for the code flow with S256, openid once, and the hint when configured", () => {
    const u = new URL(
      authorizationUrl(
        META,
        { ...CFG, idpHint: "entra" },
        { state: "s", nonce: "n", challenge: "c", redirectUri: "https://ui/cb" },
      ),
    );
    expect(u.origin + u.pathname).toBe(META.authorization_endpoint);
    const q = Object.fromEntries(u.searchParams);
    expect(q).toMatchObject({
      response_type: "code",
      client_id: CLIENT,
      code_challenge_method: "S256",
      kc_idp_hint: "entra",
    });
    expect(q.scope!.split(" ").filter((s) => s === "openid")).toHaveLength(1);
  });

  it("keeps a query the endpoint already has", () => {
    const u = authorizationUrl({ ...META, authorization_endpoint: "https://idp.test/auth?p=1" }, CFG, {
      state: "s",
      nonce: "n",
      challenge: "c",
      redirectUri: "r",
    });
    expect(u.startsWith("https://idp.test/auth?p=1&")).toBe(true);
  });

  it("makes a verifier whose S256 challenge matches, and tokens that do not repeat", () => {
    const { verifier, challenge } = pkcePair();
    expect(verifier.length).toBeGreaterThanOrEqual(43);
    expect(challenge).toBe(createHash("sha256").update(verifier).digest("base64url"));
    expect(randomToken()).not.toBe(randomToken());
    expect(randomToken()).toMatch(/^[A-Za-z0-9_-]{43}$/);
  });
});

describe("safeReturnPath", () => {
  it("keeps a same-site path and turns everything else into /", () => {
    expect(safeReturnPath("/cy-001/waitlist?x=1#y")).toBe("/cy-001/waitlist?x=1#y");
    for (const bad of [
      null,
      undefined,
      "",
      "https://evil.test/",
      "//evil.test",
      "/\\evil.test",
      "/a\\b",
      "/a\nb",
      "/a\u0000",
      "javascript:alert(1)",
    ]) {
      expect(safeReturnPath(bad), String(bad)).toBe("/");
    }
  });
});
