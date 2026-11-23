# Mobile apps

The patient's appointments on a phone: an iOS and iPadOS app in Swift and SwiftUI, and an Android app in Kotlin and Jetpack Compose, both drawn with Lily Design System. Requirements FR-227 to FR-229 (built, on demo data) and FR-230 to FR-238 (planned), and NFR-35 to NFR-38; tasks `MOB-*` in [`../../tasks.md`](../../tasks.md).

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

## Planned features

The apps are to cover what a patient expects of an NHS app. Only appointments are built, and only on example data (FR-227). Each feature below is a requirement, not a promise that it is built, and **each says where its data comes from, because most of it is not the PAS's** (spec/02-scope.md: clinical records, orders, results and prescribing are out of scope).

| Feature | Requirement | Source | Task |
|---|---|---|---|
| **Appointments:** view, book, cancel; hospital and GP | FR-227, FR-230 | PAS (hospital); GP system (GP) | MOB-8, MOB-10 |
| **Referrals and waiting lists:** stage, place, time waited | FR-231 | PAS | MOB-10 |
| **Messages** from GP practices and hospitals | FR-232 | PAS portal | MOB-8 |
| **Prescriptions:** see and order repeats | FR-233 | GP system | MOB-11 |
| **Health record:** test results, allergies, medicines | FR-234 | GP system | MOB-12 |
| **NHS 111 guidance** | FR-235 | NHS 111 (Wales), handed over | MOB-13 |
| **Organ donation decision** | FR-236 | national register | MOB-13 |
| **Register to give blood** | FR-237 | Welsh Blood Service, handed over | MOB-13 |

Rules for all of them (FR-238): a feature whose source is not connected says it is not available; one that acts for the patient asks for confirmation, is one POST, and shows its outcome in the app's own words; the app never gives clinical advice and says to call 999 in an emergency; and every piece of record text is held under NFR-38 (memory only, never copied out). A medicine, an allergy or a test result is among the most sensitive text the app can show, so these features also wait for NFR-35 and MOB-3 (a patient credential reads only its own record), and for the clinical safety case (MOB-7). Open question 21 decides where the non-PAS data comes from.

## Managed and personal devices, phones and tablets

The apps MUST run on all four of these, from one build (NFR-37):

| | Managed (the organisation's device) | BYOD (the person's own device) |
|---|---|---|
| **Phone** | iPhone under an MDM; Android with a work profile or a fully managed device | iPhone or Android phone with no management at all |
| **Tablet** | iPad under an MDM (Shared iPad is out of scope); Android tablet, managed | the person's own iPad or Android tablet |

- **Managed.** The organisation sets the server address and default language through the platform's app configuration (Apple Managed App Configuration, read from `UserDefaults` key `com.apple.configuration.managed`; Android Enterprise `RestrictionsManager`), and may block backup, copy-out and screenshots. The app reads the configuration at every launch and when it changes, ignores a value outside what it accepts (and says so, in its own words), and obeys the blocks: no iCloud or Android backup of its data, no `UIPasteboard` or `ClipboardManager` writes of record text when copy-out is blocked, `FLAG_SECURE` and the iOS screen-capture check when screenshots are blocked. Its data lives only in its own sandbox, so removing the app or wiping the work profile leaves nothing behind.
- **BYOD.** Nothing is required of the device: no enrolment, no management agent, no device-administrator rights. The app reads nothing outside itself: no other apps, no device inventory, no advertising, hardware or serial identifier. Its sign-in is held only in the Keychain or the Android Keystore, and signing out removes it. A person on a BYOD device gets the same app and the same screens as a managed one, without the organisation's settings.
- **Record text is never copied onto a personal device (NFR-38).** On BYOD it is held in memory only, always, with no setting to turn that off:
  - *No copy-out:* record text is not selectable; no clipboard, drag and drop, share sheet, print, export, "open in", file or URL carries it.
  - *No capture:* iOS blanks the window for screenshots, recording, mirroring and the app-switcher snapshot (`UIScreen.isCaptured`, a covering view on `willResignActive`); Android sets `FLAG_SECURE` and `setRecentsScreenshotEnabled(false)`. A photograph of the screen cannot be blocked, so the app says so at sign-in and the clinical safety case carries it.
  - *No storage:* no database, cache, preference, log, crash report or analytics event holds it; backup and device transfer are off (`allowBackup="false"`, `dataExtractionRules`, `isExcludedFromBackup`); no notification, widget, Spotlight, Siri, Assistant or app-search entry shows it; text fields that take it turn off keyboard suggestions and autofill.
  - *Memory:* cleared when the app leaves the foreground past the idle limit and on sign-out.
  - On a managed device the same holds unless the organisation's management says otherwise.
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
| NFR-38 | **Not yet tested** (MOB-9): a test that every Compose or SwiftUI text carrying record text is not selectable; a manifest and Info.plist test for backup and `FLAG_SECURE`; a test that a marker record text appears in no file, preference, log or pasteboard after a run (as NFR-33 does for server logs) | as Swift |
| NFR-37 | **Not yet tested** (MOB-9): an iPad simulator and an Android tablet emulator, in portrait, landscape and split view; managed-configuration tests with a stub `RestrictionsManager` and `UserDefaults` | as Swift |
| NFR-36 | `App/PASMobileUITests/AccessibilityAuditTests.swift`: `performAccessibilityAudit` with no ignored issue, at the default and the largest accessibility text size, and in Welsh | **Not yet tested**: no on-device Compose accessibility check (MOB-6) |
| FR-230 to FR-238 | **Not yet built.** Each feature gets a decode test against a recorded fixture of its source, a confirm-then-act test for each write, a "not available" test with the source unconnected, and the NFR-38 marker test | as Swift |
