import { describe, expect, it } from "vitest";
import { totp } from "../e2e/support/totp";

// RFC 6238, appendix B: the SHA-1 secret "12345678901234567890" (base32 GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ).
describe("the test one-time-code generator", () => {
  const secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
  it("matches the RFC 6238 test vectors (the last six digits of the eight-digit values)", () => {
    expect(totp(secret, 59_000)).toBe("287082");
    expect(totp(secret, 1_111_111_109_000)).toBe("081804");
    expect(totp(secret, 1_234_567_890_000)).toBe("005924");
    expect(totp(secret, 2_000_000_000_000)).toBe("279037");
  });
  it("changes every thirty seconds and not within one", () => {
    expect(totp(secret, 60_000)).toBe(totp(secret, 89_999));
    expect(totp(secret, 60_000)).not.toBe(totp(secret, 90_000));
  });
});
