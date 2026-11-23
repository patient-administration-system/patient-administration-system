# Mobile apps

The patient's appointments on a phone: an iOS and iPadOS app in Swift and SwiftUI, and an Android app in Kotlin and Jetpack Compose, both drawn with Lily Design System. Requirements FR-227 to FR-229 and NFR-35 to NFR-37; tasks `MOB-*` in [`../../tasks.md`](../../tasks.md).

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

## Managed and personal devices, phones and tablets

The apps MUST run on all four of these, from one build (NFR-37):

| | Managed (the organisation's device) | BYOD (the person's own device) |
|---|---|---|
| **Phone** | iPhone under an MDM; Android with a work profile or a fully managed device | iPhone or Android phone with no management at all |
| **Tablet** | iPad under an MDM (Shared iPad is out of scope); Android tablet, managed | the person's own iPad or Android tablet |

- **Managed.** The organisation sets the server address and default language through the platform's app configuration (Apple Managed App Configuration, read from `UserDefaults` key `com.apple.configuration.managed`; Android Enterprise `RestrictionsManager`), and may block backup, copy-out and screenshots. The app reads the configuration at every launch and when it changes, ignores a value outside what it accepts (and says so, in its own words), and obeys the blocks: no iCloud or Android backup of its data, no `UIPasteboard` or `ClipboardManager` writes of record text when copy-out is blocked, `FLAG_SECURE` and the iOS screen-capture check when screenshots are blocked. Its data lives only in its own sandbox, so removing the app or wiping the work profile leaves nothing behind.
- **BYOD.** Nothing is required of the device: no enrolment, no management agent, no device-administrator rights. The app reads nothing outside itself: no other apps, no device inventory, no advertising, hardware or serial identifier. Its sign-in is held only in the Keychain or the Android Keystore, and signing out removes it. A person on a BYOD device gets the same app and the same screens as a managed one, without the organisation's settings.
- **Phones and tablets.** Every screen is laid out for the smallest phone width and for a tablet, in portrait and landscape, in split view, slide over and multi-window, at the largest text size. On a wide screen the list uses a readable column (or list and detail side by side once there is a detail), never a stretched phone layout. The iOS app already targets iPhone and iPad; the Android manifest sets no phone-only restriction, but no tablet layout has been tried.
- **Not decided here:** which MDM products to certify, and whether a managed device may sign in with the organisation's identity rather than a patient credential (MOB-3's open question 20).

**Not built yet.** None of this is implemented or tested (MOB-9); the apps read no managed configuration, and their layouts have been checked on a phone only.

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
| NFR-37 | **Not yet tested** (MOB-9): an iPad simulator and an Android tablet emulator, in portrait, landscape and split view; managed-configuration tests with a stub `RestrictionsManager` and `UserDefaults` | as Swift |
| NFR-36 | `App/PASMobileUITests/AccessibilityAuditTests.swift`: `performAccessibilityAudit` with no ignored issue, at the default and the largest accessibility text size, and in Welsh | **Not yet tested**: no on-device Compose accessibility check (MOB-6) |
