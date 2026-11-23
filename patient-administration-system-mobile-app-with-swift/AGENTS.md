# AGENTS: iOS app

Read [`../AGENTS.md`](../AGENTS.md) and [`../spec/mobile-apps/index.md`](../spec/mobile-apps/index.md) first.

- Logic goes in `PASMobileCore` with a Swift Testing test; `PASMobileUI` and `App/` stay thin.
- No server text on screen: map a failure to a `LoadProblem` and show its catalogue key (FR-228).
- Do not wire `HTTPAppointmentSource` into the app, or add sign-in, until NFR-35 is lifted (MOB-3).
- Every word is a catalogue key in `en.lproj` and `cy.lproj`, and the same key in the Android app. Translations are written by hand, serially; mark new Welsh as a first draft.
- Run `swift test` and the simulator audit (`App/`, see README) before saying a change is done. Do not add an ignored issue to the audit without a written reason.
- Dates: clinic time (`AppointmentFormat.clinicTimeZone`), and assert on structure, not digits.
