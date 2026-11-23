// The accessibility audit (XCUIApplication.performAccessibilityAudit) on the appointments screen, at the default text size and
// the largest accessibility size (NFR-36). No issue is ignored; if one must be, list it here with its reason.
import XCTest

final class AccessibilityAuditTests: XCTestCase {
    @MainActor private func launch(textSize: String? = nil, language: String? = nil) -> XCUIApplication {
        let app = XCUIApplication()
        if let textSize { app.launchArguments += ["-UIPreferredContentSizeCategoryName", textSize] }
        if let language { app.launchArguments += ["-AppleLanguages", "(\(language))", "-AppleLocale", "\(language)_GB"] }
        app.launch()
        return app
    }

    @MainActor func testAppointmentsScreenPassesTheAuditAtDefaultSize() throws {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Upcoming"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'These are example appointments'"))
            .firstMatch.exists, "the demo says it is a demo (FR-229)")
        try app.performAccessibilityAudit()
    }

    @MainActor func testAppointmentsScreenPassesTheAuditAtTheLargestTextSize() throws {
        let app = launch(textSize: "UICTContentSizeCategoryAccessibilityXXXL")
        XCTAssertTrue(app.navigationBars.firstMatch.waitForExistence(timeout: 10))
        try app.performAccessibilityAudit()
    }

    @MainActor func testWelshShowsTheWelshCatalogue() throws {
        let app = launch(language: "cy")
        XCTAssertTrue(app.staticTexts["I ddod"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Dydd'")).firstMatch.exists,
                      "dates are in Welsh too")
        try app.performAccessibilityAudit()
    }
}
