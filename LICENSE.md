# Licence

This repository is available under **any one of** the licences below, at your choice.
The copyright holder is Joel Parker Henderson.

The SPDX expression used in the manifests of the server and the UI is:

```
MIT OR Apache-2.0 OR BSD-3-Clause OR GPL-2.0-only OR GPL-3.0-only
```

The Loco front-end declares the narrower `MIT OR Apache-2.0`.

| Licence | SPDX identifier | Text |
|---|---|---|
| MIT | `MIT` | [`LICENSE/MIT.txt`](LICENSE/MIT.txt) |
| Apache License 2.0 | `Apache-2.0` | [`LICENSE/Apache-2.0.txt`](LICENSE/Apache-2.0.txt) |
| BSD 3-Clause | `BSD-3-Clause` | [`LICENSE/BSD-3-Clause.txt`](LICENSE/BSD-3-Clause.txt) |
| GNU GPL v2.0 | `GPL-2.0-only` | [`LICENSE/GPL-2.0-only.txt`](LICENSE/GPL-2.0-only.txt) |
| GNU GPL v3.0 | `GPL-3.0-only` | [`LICENSE/GPL-3.0-only.txt`](LICENSE/GPL-3.0-only.txt) |

The permissive licences suit organizations that embed or adapt the code; Apache-2.0 adds an explicit patent grant. The GNU licences suit those who want copyleft.

A test (`patient-administration-system-with-rust/tests/licence_files.rs`) fails if a manifest names a licence that has no text in `LICENSE/`.

Third-party dependencies keep their own licences; `cargo deny check` enforces an allow-list (`deny.toml`).
