# Multi-factor authentication plan (DTAC-5)

**Rule:** a staff member or an administrator signs in with a password **and** a one-time code. A patient signs in with a password alone.

## How it is enforced

The `pas` realm (`podman/keycloak/pas-realm.json`) uses its own browser flow, `browser-mfa`:

1. an existing session cookie is enough (so a person is not asked again on every page);
2. otherwise the username and password form, and then a **conditional** step: *if the user holds the `staff` role, require the one-time-code form*. Administrators hold `staff` as well as `admin`, so one rule covers both.

A staff account that has no authenticator yet is walked through enrolling one at its first sign-in (Keycloak's own "configure authenticator" step), so a new member of staff cannot skip it. Nothing in the PAS or the UI can switch the rule off: it is decided by the identity provider before a token exists.

## What the test proves

`sign-in.spec.ts` ("staff and administrators must give a one-time code as well as a password; patients are not asked"):

- an administrator who gives only a password is stopped at the code step, and a wrong code is refused with no session on the UI;
- the right code signs in;
- a patient signs in by password alone and is never shown a code field.

The seeded administrator has a development authenticator secret (`JBSWY3DPEHPK3PXP`), and the browser tests generate its code (`tests/e2e/support/totp.ts`, checked against the RFC 6238 test vectors).

## What must be different in production

The file is a **development** realm. A production realm must:

- not carry the seeded secret or the seeded users at all: staff enrol their own authenticator;
- leave `otpPolicyCodeReusable` **false** (the development realm sets it true only so that many browser tests can sign in within one 30-second step);
- decide which authenticators are acceptable (an authenticator app, or a hardware key through WebAuthn as a stronger second factor) and how a lost authenticator is recovered (an identity check by a second administrator, recorded), because "reset my second factor" is the usual way in for an attacker;
- keep at least two administrators able to recover each other.

## Not covered

- Patients are not asked for a second factor. Their portal shows the secure messages and appointments of one person and is gated by identity proofing (DTAC-9) instead; if a deploying organisation wants more, NHS login provides it.
- A service account (the UI's server to the API) uses a signed token, not a password: see `operations/deployment.md`.
