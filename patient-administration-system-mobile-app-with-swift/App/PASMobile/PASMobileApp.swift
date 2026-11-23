// The iOS and iPadOS patient app (spec/mobile-apps/index.md). It shows the demo source only: the app may not call a real PAS
// until NFR-35 holds (the API must itself refuse a patient another person's record).
import PASMobileCore
import PASMobileUI
import SwiftUI

@main
struct PASMobileApp: App {
    var body: some Scene {
        WindowGroup {
            AppointmentsScreen(model: AppointmentsModel(source: DemoAppointmentSource(), isDemo: true))
        }
    }
}
