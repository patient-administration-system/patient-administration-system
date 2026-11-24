# Mobile apps

The patient's appointments on a phone: an iOS and iPadOS app in Swift and SwiftUI, and an Android app in Kotlin and Jetpack Compose, both drawn with Lily Design System. Requirements FR-227 to FR-229 and NFR-35 to NFR-36; tasks `MOB-*` in [`../../tasks.md`](../../tasks.md).

| Subproject | Stack | Lily | Tests |
|---|---|---|---|
| [`patient-administration-system-mobile-app-with-swift/`](../../patient-administration-system-mobile-app-with-swift/README.md) | Swift 6, SwiftUI, iOS 18, a Swift package (`PASMobileCore`, `PASMobileUI`) and a thin Xcode app (`App/`) | `LilyUI`, `LilyThemes` (Lily Design System for Swift) | `swift test` (Swift Testing); `xcodebuild test` runs the accessibility audit on a simulator |
| [`patient-administration-system-mobile-app-with-kotlin/`](../../patient-administration-system-mobile-app-with-kotlin/README.md) | Kotlin 2.4, Jetpack Compose (foundation, no Material), Android 8 (API 26) and later; a JVM `core` module and an Android `app` | `lily-compose`, `lily-compose-themes` (Lily Design System for Kotlin) | `./gradlew test` (JUnit on the JVM, Robolectric for the screen) |

## What the apps do

One screen, **My appointments**, the same as the web UI's `/my/appointments`:

- Upcoming appointments, earliest first, then past ones, most recent first. An appointment is upcoming until it ends (FR-227).
- For each: when (day, date and start time in clinic time, Europe/London, in the catalogue's language), how it is held (in person, video, phone), and its status as a Lily tag. A status or modality the app does not know shows as "Other"; a field the app does not know is ignored, so a server change cannot break the app (FR-227).
- A failed load shows the app's own words for one of five problems (unreachable, signed out, forbidden, unavailable; a person the PAS does not hold has no appointments) and a retry. Nothing the server says reaches the screen (FR-228), as on the web (AGENTS.md).
- English and Welsh. Keys and wording are the web UI's where the meaning is the same (`status.*`, `error.*`, `myAppointments.*`); app-only keys start `mobile.`. The Welsh is a first draft awaiting fluent review, like the web UI's. The other eight web locales are not yet in the apps.
- Light and dark follow the device (`LilyTokens.light` and `.dark`).

## Why the apps show example data only

The PAS API does not itself limit a patient to their own record: any reader may `GET /api/patients/{id}/appointments` for any id. The web UI is safe because its server takes the person from the verified ID token and never from the browser (NFR-3). A phone app is a public client: whatever credential it holds, a person can lift it and ask for any id. So **until the API itself refuses a patient's credential every record but the token's own `person_id`, the apps MUST NOT call the API with a patient's credential** (NFR-35). The Android app does not even ask for the network permission; the iOS app builds only the demo source.

The HTTP client is built and tested against a stub (`HTTPAppointmentSource`, `HttpAppointmentSource`): the request, the bearer header, the envelope, and the mapping of every failure. Sign-in is not built. The way to lift NFR-35 is open (open question 20): patient-scoped tokens checked by the API, or a mobile backend-for-frontend that does what the web UI's server does.

## Lily Design System

Neither Lily library is published yet (no Swift export repository, nothing on Maven Central), so both apps resolve Lily from a local checkout of the Lily monorepo: `LILY_SWIFT_PATH` and `-PlilyKotlinPath` / `LILY_KOTLIN_PATH`, defaulting to `../../../lilydesignsystem/lily-design-system/lily-design-system-{swift,kotlin}` from the subproject. For the same reason **CI does not build the apps yet** (MOB-5).

## Acceptance

| Requirement | Swift | Kotlin |
|---|---|---|
| FR-227 | `AppointmentsTests.swift`: `decodesTheApiEnvelopeAndToleratesNewValues`, `splitsUpcomingEarliestFirstAndPastMostRecentFirst`, `equalStartsHaveAStableOrder`, `whenIsInClinicTimeAndTheLocalesWords` | `AppointmentsTest`: the same four; `AppointmentsScreenTest.demoShowsUpcomingAndPastInEnglish`, `demoShowsWelsh` |
| FR-228 | `failuresBecomeTheAppsOwnProblems`, `modelStates` | `failuresBecomeTheAppsOwnProblems`, `modelStates`, `AppointmentsScreenTest.failureShowsTheAppsWordsAndRetries` |
| FR-229 | `demoAlwaysHasUpcomingAndPast`; the UI test checks the notice | `demoAlwaysHasUpcomingAndPast`, `demoShowsUpcomingAndPastInEnglish` |
| Catalogues | `CatalogueTests.swift`: every locale has the English keys, every code key exists, shared keys match the web UI | `CatalogueTest`: the same, and Android matches iOS word for word |
| NFR-35 | Read: `PASMobileApp.swift` builds only `DemoAppointmentSource` (no automated test) | `ManifestTest.theAppAsksForNoNetworkPermission` |
| NFR-36 | `App/PASMobileUITests/AccessibilityAuditTests.swift`: `performAccessibilityAudit` with no ignored issue, at the default and the largest accessibility text size, in Welsh and Arabic, and in dark mode; `app/src/androidTest/.../AccessibilityTest.kt`: the Accessibility Test Framework checks on an emulator in light and dark, run at font scale 1.0 and 2.0 | **Partly tested:** Android in Welsh and Arabic, and a physical device, are not yet covered (MOB-6) |
