# Patient Administration System: Android app (Kotlin)

The patient's appointments on Android, in Kotlin and Jetpack Compose, drawn with [Lily Design System for Kotlin](https://github.com/LilyDesignSystem/lily-design-system). Specification: [`../spec/mobile-apps/index.md`](../spec/mobile-apps/index.md). Tasks: `MOB-*` in [`../tasks.md`](../tasks.md).

**It shows example appointments only,** and does not ask for the network permission. The app may not call a real PAS until the API itself limits a patient to their own record (NFR-35). Sign-in is not built.

## Layout

| Path | What |
|---|---|
| `core/` | Kotlin on the JVM, no Android: `Appointment` and `PasDecoding`, `AppointmentList`, `AppointmentFormat`, `AppointmentSource` (the HTTP client and the demo source), `AppointmentsModel`, `Messages` |
| `app/` | The Android app: `MainActivity`, `AppointmentsScreen` (Lily `LilyCard`, `SummaryList`, `Tag`, `InsetText`, `ErrorSummary` on Compose foundation, no Material), `res/values*/strings.xml` |
| `app/src/test/` | Robolectric screen tests, the catalogue tests (against the iOS app and the web UI) and the manifest test |

## Build and test

Lily for Kotlin is not on Maven Central yet, so it is an included build from a local checkout of the Lily monorepo: `-PlilyKotlinPath=...`, `LILY_KOTLIN_PATH`, else `../../../lilydesignsystem/lily-design-system/lily-design-system-kotlin`. Needs a JDK 17 or later (`JAVA_HOME`) and the Android SDK with platform 37 (`ANDROID_HOME`).

```sh
./gradlew test assembleDebug lintDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Minimum Android 8 (API 26).

## Words

`app/src/main/res/values/strings.xml` is the source; `values-cy` is a **first draft** awaiting fluent review. A resource name is the shared catalogue key with `.` as `_`; `CatalogueTest` checks the words match the iOS app's and, for shared keys, the web UI's. Map a new key in `Strings.kt`.
