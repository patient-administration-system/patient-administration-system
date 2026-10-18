# Security policy

## Reporting a vulnerability

Please report a suspected vulnerability **privately**, not in a public issue.

- Use GitHub's private vulnerability reporting on the repository (**Security → Report a vulnerability**).
- Say what you found, how to reproduce it, which version or commit, and what you think the impact is. A proof of concept helps; please do not use it against anyone's live system.
- If the report involves personal data, describe it without attaching real personal data.

There is one maintainer (see [MAINTAINERS.md](MAINTAINERS.md)). The intent is to acknowledge a report within **5 business days** and to say what will be done within **15 business days**; these are targets, not guarantees. A fix is released as a patch version and noted in the changelog, crediting you if you wish. No bug bounty is offered.

## Supported versions

Only the latest release and the `main` branch receive security fixes. The project is pre-1.0.

## What the software does today

- Sign-in uses OpenID Connect (code flow with PKCE); tokens stay on the server; sessions are httpOnly, `__Host-` cookies with a hard and an idle limit.
- Every UI page and form action checks the user's role on the server; a patient can read only their own record.
- The UI sends a Content-Security-Policy and the standard security headers, tested in a browser.
- Production code contains no `unsafe` Rust. Dependencies are checked with `cargo deny` against an allow-list of licences and known advisories.

## What it does not do yet

Be aware before you deploy ([`governance/production-readiness.md`](governance/production-readiness.md)):

- The API authenticates callers with one shared token, or not at all if `API_TOKEN` is unset. It has no per-user authentication, and the audit log records the user id the caller supplies.
- The shipped identity realm does not require multi-factor authentication.
- Nothing proves a patient is the person their account says (the account-to-patient link is set by hand).
- No external penetration test has been done.
- Six advisories are accepted with reasons in [`deny.toml`](deny.toml); they come from the Loco framework version and are tracked as DEP-2 in [`tasks.md`](tasks.md).
