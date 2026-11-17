# Security policy

This is the security policy of the Patient Administration System (PAS) project: how to report a
vulnerability, what the project commits to do with a report, how a fix is disclosed, and what the
project does to keep the software secure. It applies to this repository. It is not the security
policy of any organisation that runs the software; see [For deploying organisations](#for-deploying-organisations).

| | |
|---|---|
| Owner | The maintainer ([MAINTAINERS.md](MAINTAINERS.md)) |
| Version | 2.0 |
| Last reviewed | 2026-11-17 |
| Next review | At the next minor release, and no later than 2027-11-17 |

## 1. Scope

**In scope:**

- The source in this repository: the API server (`patient-administration-system-with-rust`), its
  migrations, the operator front-end (`patient-administration-system-frontend`), the web UI
  (`patient-administration-system-ui-with-svelte`), the scripts, and the shipped configuration (the
  development identity realm, the container files, the compose stack).
- The release artifacts the project publishes: tagged source, container images built by
  [`release.yml`](.github/workflows/release.yml), and their SBOMs and provenance.
- The project's CI workflows, where a flaw could let someone change what is built or released.

**Out of scope** (report these elsewhere):

- A system that some organisation runs: report it to that organisation. This project operates no
  production service and holds no patient data.
- A vulnerability in a dependency: report it to that project first. Tell us as well if the PAS is
  exposed by it, and we will treat it as a report here.
- The development defaults that are documented as unsafe for production, such as the published
  development passwords in `podman/compose.yml` or running the API without `PAS_REQUIRE_AUTH=true`.
  A way to run those defaults in production *without a warning or refusal* is in scope.
- Findings from automated scanners with no demonstrated effect, missing headers on pages that
  serve no personal data, and denial of service by sheer volume.

## 2. How to report a vulnerability

Report **privately**. Do not open a public issue, pull request or discussion about it.

1. **Preferred:** GitHub private vulnerability reporting on this repository (**Security → Report a
   vulnerability**). The report is visible only to you and the maintainer, and becomes the draft
   advisory.
2. **If that is unavailable:** open a public issue titled "Security contact request" that contains
   **no** details of the problem. The maintainer will reply with a private channel.

Please include:

- what you found and where (file, route, page, version or commit);
- how to reproduce it, as short as you can make it;
- what you believe an attacker could do, and what they need first (an account, a role, network
  position);
- whether you believe it is being exploited;
- how you would like to be credited, or that you would not.

**Never send real personal data.** If the problem exposes patient or staff data, describe the kind
of data and show it with made-up records.

## 3. What the project commits to

There is one maintainer. These are the targets the project holds itself to; when one will be
missed, the maintainer says so to the reporter with a new date.

| Step | Target |
|---|---|
| Acknowledge the report | 5 business days |
| Triage: confirm or reject it, give a severity, and say what will be done | 15 business days |
| Fix released, critical | 30 days from triage |
| Fix released, high | 60 days from triage |
| Fix released, medium | 90 days from triage |
| Fix released, low | the next planned release |

The reporter is kept informed at each step and is asked to review the fix where that helps.

## 4. Severity

Severity is the [CVSS v4.0](https://www.first.org/cvss/v4.0/) base score, then raised where the
data makes it worse. Because the PAS holds identifying and administrative health data, a finding is
at least **high**, whatever its score, when it lets someone:

- read or change a person's record they are not allowed to (for example, a patient reaching another
  patient's record, against NFR-3);
- make a change that is not audited, or that is audited under another person's identity (NFR-1,
  NFR-16, NFR-25);
- obtain a secret, a session or a token belonging to someone else (NFR-14);
- change what the CI builds or what a release contains.

A finding that could cause patient harm (a wrong waiting time, a lost referral, a booking shown to
the wrong person) is also raised with the deploying organisation's Clinical Safety Officer through
the hazard log ([`governance/clinical-safety.md`](governance/clinical-safety.md)), once disclosed.

## 5. Disclosure

The project practises coordinated disclosure.

- A vulnerability is made public **when its fix is released**, or **90 days after the report**,
  whichever comes first. The reporter and the maintainer may agree a different date. If the
  vulnerability is being exploited, it may be published sooner, with whatever mitigation exists.
- The disclosure is a GitHub security advisory on this repository, with a CVE identifier requested
  through GitHub for anything rated medium or above, and an entry in [CHANGELOG.md](CHANGELOG.md).
  The advisory names the affected and fixed versions, the severity and its vector, a mitigation for
  those who cannot upgrade at once, and the reporter's credit if they want it.
- Until disclosure, the fix is developed in a private fork of the advisory, and the commit message
  that lands it does not describe the vulnerability.
- Reporters are asked not to disclose before the agreed date. The project will not ask for silence
  beyond 90 days without agreement.

No bug bounty is offered.

## 6. Good-faith research

The project welcomes security research on this software. Research done in good faith under this
policy is authorised by the project, and the project will not ask anyone to take legal action
against the researcher over it. Good faith means you:

- test against **your own** copy (the compose stack in `podman/` runs the whole system locally),
  never a system someone else runs without that organisation's permission;
- use only made-up data, and stop and report at once if you come across real personal data;
- do no more than you need to show the problem (no data kept, no persistence, no lateral movement);
- do not degrade a service, and do not use social engineering or physical access;
- report privately, as in section 2, and give the project the time in section 5.

This covers only the project's own interests. It cannot give you permission on behalf of any
organisation that runs the software.

## 7. Supported versions

The project is pre-1.0. Only the **latest release** and the **`main` branch** receive security
fixes. A fix is released as a patch version. There is no back-porting to older releases.

## 8. How the project keeps the software secure

Each of these runs on every push to `main` and on every pull request unless it says otherwise. Each
is named in `spec/07-non-functional-requirements.md` or in the workflow that runs it.

- **Memory safety.** Production code forbids `unsafe` (`#![forbid(unsafe_code)]`, NFR-8).
- **Dependencies.** `cargo deny` refuses any known advisory, any yanked crate, any licence not on the
  allow-list, and any source other than crates.io ([`deny.toml`](deny.toml), NFR-9); no advisory is
  currently accepted. `pnpm audit` fails the UI build on a high or critical advisory. Dependabot
  proposes updates weekly for Cargo, npm, GitHub Actions and the development images. The UI's package
  manager takes no version less than a day old (`minimumReleaseAge`), and Rust updates wait the
  same by practice, unless the maintainer approves an exception.
- **Static analysis.** CodeQL on every push and pull request and weekly. Clippy with warnings as
  errors. `actionlint` and `zizmor` check the workflows; every third-party action is pinned to a
  full commit SHA.
- **Testing aimed at security.** Browser tests of the Content-Security-Policy and security headers
  with zero violations (NFR-7); tests that a patient can reach only their own record (NFR-3), that
  the audit identity is the verified token's (NFR-16), and that secrets and personal data stay out
  of logs and error responses (NFR-14, `tests/hub5_no_personal_data_leaks_test.rs`). Weekly fuzzing
  of the HL7 v2 message and batch parsers, the log redactor, text-message replies and identifier
  check digits ([`fuzz.yml`](.github/workflows/fuzz.yml)) and weekly
  mutation testing of the modules where a surviving mutant would hurt
  ([`mutation.yml`](.github/workflows/mutation.yml)).
- **Releases.** A release is built only from a tag whose version and changelog agree. It ships
  CycloneDX SBOMs for the Rust workspace and the UI, and its container images carry build-provenance
  attestations.
- **Secrets.** Secrets come only from the environment (the operator front-end may also read its
  database address from a file, `DB_URL_FILE`); the configuration's debug output redacts them. The repository holds no secret except the development defaults that
  are labelled as such.

## 9. What the software does today

- Sign-in uses OpenID Connect (code flow with PKCE); tokens stay on the server; sessions are
  httpOnly, `__Host-` cookies with a hard and an idle limit.
- Every UI page and form action checks the user's role on the server; a patient can read only their
  own record.
- The UI sends a Content-Security-Policy and the standard security headers, tested in a browser.
- HL7 v2 over MLLP can run over TLS, with client certificates, a connection limit and an idle
  timeout.

## 10. What it does not do yet

Be aware of these before you deploy. The whole list, with an estimate for each item, is the
[path to production](governance/path-to-production.md).

- Per-user authentication at the API is opt-in (`PAS_REQUIRE_AUTH=true`). Without it the API takes
  one shared token, or none if `API_TOKEN` is unset, and the audit log records the user id the
  caller supplies.
- The operator dashboard (`patient-administration-system-frontend`) has no sign-in (AXA-3).
- The shipped identity realm requires a one-time code for staff; a deploying organisation's own
  identity provider must do the same.
- A patient's account is linked only after a named staff member records proof of identity, through
  the API: there is no staff screen for it, and no digital identity scheme (a national login, a bank
  identity, an eID) is integrated.
- No external penetration test has been done (DTAC-10).

## For deploying organisations

An organisation that runs the PAS is the data controller and is responsible for the security of
its deployment: its own vulnerability-reporting contact (publish one at
`/.well-known/security.txt`, [RFC 9116](https://www.rfc-editor.org/rfc/rfc9116)), its incident
response, its identity provider, network, backups and key management, and its own penetration test.
[`operations/deployment.md`](operations/deployment.md) lists the settings that must be changed from
the development defaults, and [`operations/runbook.md`](operations/runbook.md) says who to tell
when something goes wrong.

To hear about fixes, watch this repository's releases and security advisories.

## Changes to this policy

Changes go through a pull request like any other change and are recorded in the table at the top.
Version 1 (2026-10) was a short note on reporting; version 2 (2026-11-17) is this policy.
