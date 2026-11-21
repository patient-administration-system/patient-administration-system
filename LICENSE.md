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
| MIT | `MIT` | [`LICENSES/mit/index.md`](LICENSES/mit/index.md) |
| Apache License 2.0 | `Apache-2.0` | [`LICENSES/apache-2.0/index.md`](LICENSES/apache-2.0/index.md) |
| BSD 3-Clause | `BSD-3-Clause` | [`LICENSES/bsd-3-clause/index.md`](LICENSES/bsd-3-clause/index.md) |
| GNU GPL v2.0 | `GPL-2.0-only` | [`LICENSES/gpl-2.0-only/index.md`](LICENSES/gpl-2.0-only/index.md) |
| GNU GPL v3.0 | `GPL-3.0-only` | [`LICENSES/gpl-3.0-only/index.md`](LICENSES/gpl-3.0-only/index.md) |
| UK Open Government Licence v3.0 | `OGL-UK-3.0` | [`LICENSES/ogl-uk-3.0/index.md`](LICENSES/ogl-uk-3.0/index.md) |
| Creative Commons Attribution Non Commercial Share Alike 4.0 International | `CC-BY-NC-SA-4.0` | [`LICENSES/cc-by-nc-sa-4.0/index.md`](LICENSES/cc-by-nc-sa-4.0/index.md) |

The permissive licences suit organizations that embed or adapt the code; Apache-2.0 adds an explicit patent grant. The GNU licences suit those who want copyleft. The UK Open Government Licence v3.0 suits public-sector reuse in the UK; CC BY-NC-SA 4.0 suits non-commercial reuse shared alike (it forbids commercial use, so it is not an open-source licence).

A test (`patient-administration-system-with-rust/tests/licence_files.rs`) fails if a manifest names a licence that has no `LICENSES/<lowercase-spdx-id>/index.md`, if `LICENSE.md` does not link each text, or if a `LICENSE/` directory exists. All licence texts live in `LICENSES/`, one `index.md` per licence, and nowhere else.

Third-party dependencies keep their own licences; `cargo deny check` enforces an allow-list (`deny.toml`).

## Full license texts

The full text of each license is in its own directory:

- [MIT](LICENSES/mit/index.md)
- [Apache-2.0](LICENSES/apache-2.0/index.md)
- [BSD-3-Clause](LICENSES/bsd-3-clause/index.md)
- [GPL-2.0-only](LICENSES/gpl-2.0-only/index.md)
- [GPL-3.0-only](LICENSES/gpl-3.0-only/index.md)
- [OGL-UK-3.0](LICENSES/ogl-uk-3.0/index.md)
- [CC-BY-NC-SA-4.0](LICENSES/cc-by-nc-sa-4.0/index.md)
