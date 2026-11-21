# Licence

This repository is available under **any one of** the licences below, at your choice.
The copyright holder is Joel Parker Henderson.

The SPDX expression used in the manifests of the server and the UI is:

```
MIT OR Apache-2.0 OR BSD-3-Clause OR GPL-2.0-only OR GPL-3.0-only OR OGL-UK-3.0 OR CC-BY-NC-SA-4.0
```

The front-end crate declares the narrower `MIT OR Apache-2.0`.

| Licence | SPDX identifier | Text |
|---|---|---|
| MIT | `MIT` | [`LICENSE/MIT.txt`](LICENSE/MIT.txt) |
| Apache License 2.0 | `Apache-2.0` | [`LICENSE/Apache-2.0.txt`](LICENSE/Apache-2.0.txt) |
| BSD 3-Clause | `BSD-3-Clause` | [`LICENSE/BSD-3-Clause.txt`](LICENSE/BSD-3-Clause.txt) |
| GNU GPL v2.0 | `GPL-2.0-only` | [`LICENSE/GPL-2.0-only.txt`](LICENSE/GPL-2.0-only.txt) |
| GNU GPL v3.0 | `GPL-3.0-only` | [`LICENSE/GPL-3.0-only.txt`](LICENSE/GPL-3.0-only.txt) |
| UK Open Government Licence v3.0 | `OGL-UK-3.0` | [`LICENSE/OGL-UK-3.0.txt`](LICENSE/OGL-UK-3.0.txt) |
| Creative Commons Attribution Non Commercial Share Alike 4.0 International | `CC-BY-NC-SA-4.0` | [`LICENSE/CC-BY-NC-SA-4.0.txt`](LICENSE/CC-BY-NC-SA-4.0.txt) |

The permissive licences suit organizations that embed or adapt the code; Apache-2.0 adds an explicit patent grant. The GNU licences suit those who want copyleft. The UK Open Government Licence v3.0 suits public-sector reuse in the UK; CC BY-NC-SA 4.0 suits non-commercial reuse shared alike (it forbids commercial use, so it is not an open-source licence).

A test (`patient-administration-system-with-rust/tests/licence_files.rs`) fails if a manifest names a licence that has no text in `LICENSE/`.

Third-party dependencies keep their own licences; `cargo deny check` enforces an allow-list (`deny.toml`).
