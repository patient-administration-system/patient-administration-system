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

Be aware before you deploy (the whole list, with an estimate for each item, is the [path to production](governance/path-to-production.md)):

- Per-user authentication at the API is opt-in (`PAS_REQUIRE_AUTH=true`). Without it the API takes one shared token, or none if `API_TOKEN` is unset, and the audit log records the user id the caller supplies.
- The operator dashboard (`patient-administration-system-frontend`) has no sign-in (AXA-3).
- The shipped identity realm requires a one-time code for staff; a deploying organisation's own identity provider must do the same.
- A patient's account is linked only after a named staff member records proof of identity, through the API: there is no staff screen for it, and NHS login is not integrated.
- No external penetration test has been done.
- [`deny.toml`](deny.toml) accepts no advisories.
