# AGENTS: Android app

Read [`../AGENTS.md`](../AGENTS.md) and [`../spec/mobile-apps/index.md`](../spec/mobile-apps/index.md) first.

- Logic goes in `core` (plain Kotlin, JUnit); `app` stays thin. Keep `core` the same shape as the iOS app's `PASMobileCore`.
- Compose foundation and Lily only; no Material library.
- No server text on screen (FR-228). No `INTERNET` permission and no sign-in until NFR-35 is lifted (MOB-3); `ManifestTest` enforces the first.
- Every word is a string resource in `values` and `values-cy`, mapped in `Strings.kt`, with the same key and words as the iOS app (`CatalogueTest`). Translations are by hand, serially; mark new Welsh as a first draft.
- Run `./gradlew test assembleDebug lintDebug` before saying a change is done.
