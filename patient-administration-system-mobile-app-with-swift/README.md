# Patient Administration System: iOS app (Swift)

The patient's appointments on iPhone and iPad, in Swift 6 and SwiftUI, drawn with [Lily Design System for Swift](https://github.com/LilyDesignSystem/lily-design-system). Specification: [`../spec/mobile-apps/index.md`](../spec/mobile-apps/index.md). Tasks: `MOB-*` in [`../tasks.md`](../tasks.md).

**It shows example appointments only.** The app may not call a real PAS until the API itself limits a patient to their own record (NFR-35). Sign-in is not built.

## Layout

| Path | What |
|---|---|
| `Package.swift` | The `PASMobile` package: `PASMobileCore` (no UI) and `PASMobileUI` (SwiftUI) |
| `Sources/PASMobileCore/` | `Appointment` and its decoding from the PAS envelope, `AppointmentList` (upcoming and past), `AppointmentFormat` (clinic-time dates), `AppointmentSource` (the HTTP client and the demo source), `AppointmentsModel` (the screen's state), `Messages` and the catalogues in `Resources/*.lproj` |
| `Sources/PASMobileUI/AppointmentsScreen.swift` | The screen: Lily `Card`, `SummaryList`, `Tag`, `InsetText`, `ErrorSummary`, themed by `LilyThemes` |
| `Tests/PASMobileCoreTests/` | Swift Testing: decoding, ordering, failures, the model, dates, catalogues |
| `App/` | The Xcode app (`PASMobile.xcodeproj`), which hosts `AppointmentsScreen`, and its UI test, the accessibility audit |

## Build and test

Lily for Swift is not published yet, so the package resolves it from a local checkout of the Lily monorepo: `LILY_SWIFT_PATH`, else `~/git/lilydesignsystem/lily-design-system/lily-design-system-swift` (relative: `../../../lilydesignsystem/...`). The Xcode project uses this package, so the same applies.

```sh
swift test                                              # the logic, on macOS; no simulator
cd App && xcodebuild -project PASMobile.xcodeproj -scheme PASMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17' test   # the accessibility audit on a simulator
```

Needs Xcode 27 (Swift 6.4, the iOS 27 SDK); the app runs on iOS 18 and later.

## Words

`Resources/en.lproj/Localizable.strings` is the source; `cy.lproj` is a **first draft** awaiting fluent review. Keys are the web UI's where the meaning is the same and keep its wording (a test compares them); app-only keys start `mobile.`. The Android app has the same words (its test compares them). Add a key to both apps and both catalogues in one change.
