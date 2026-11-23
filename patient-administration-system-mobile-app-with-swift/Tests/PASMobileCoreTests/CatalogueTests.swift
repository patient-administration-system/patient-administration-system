import Foundation
@testable import PASMobileCore
import Testing

/// Every locale has exactly the English keys and every value is filled (the rule the web UI's i18n test keeps).
@Test func everyLocaleHasExactlyTheEnglishKeys() throws {
    let english = try #require(Messages.catalogue("en"))
    #expect(english.count > 20)
    for locale in Messages.locales {
        let catalogue = try #require(Messages.catalogue(locale), "no catalogue for \(locale)")
        #expect(Set(catalogue.keys) == Set(english.keys), "\(locale) differs from English")
        #expect(catalogue.values.allSatisfy { !$0.trimmingCharacters(in: .whitespaces).isEmpty })
    }
}

/// Every key the code names is in the catalogue.
@Test func everyKeyTheCodeUsesExists() throws {
    let english = try #require(Messages.catalogue("en"))
    let keys = [AppointmentStatus.proposed, .booked, .arrived, .fulfilled, .cancelled, .noShow, .other("x")].map(\.messageKey)
        + [Modality.inPerson, .video, .phone, .other("x")].map(\.messageKey)
        + [LoadProblem.unreachable, .signedOut, .forbidden, .notFound, .unavailable].map(\.messageKey)
        + ["myAppointments.title", "myAppointments.empty", "mobile.upcoming", "mobile.past", "mobile.when", "mobile.how",
           "mobile.status", "mobile.loading", "mobile.retry", "mobile.demoNotice", "mobile.error.title"]
    for key in keys { #expect(english[key] != nil, "missing \(key)") }
}

/// Keys shared with the web UI keep the web UI's English wording, so the two say the same thing.
@Test func sharedKeysMatchTheWebCatalogue() throws {
    let here = URL(fileURLWithPath: #filePath)
    let web = here.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent()
        .appending(path: "patient-administration-system-ui-with-svelte/src/lib/messages")
    for (mine, theirs) in [("en", "en-001"), ("cy", "cy-001")] {
        let data = try Data(contentsOf: web.appending(path: "\(theirs).json"))
        let webCatalogue = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let catalogue = try #require(Messages.catalogue(mine))
        for (key, value) in catalogue where !key.hasPrefix("mobile.") {
            if let webValue = webCatalogue[key] as? String {
                #expect(webValue == value, "\(mine) \(key)")
            }
        }
    }
}

/// The catalogue language follows the person's preferences, and a language with no catalogue falls back to English.
@Test func localeFollowsPreferences() {
    #expect(Messages.locale(for: ["cy-GB", "en-GB"]) == "cy")
    #expect(Messages.locale(for: ["fr-FR", "cy"]) == "cy")
    #expect(Messages.locale(for: ["fr-FR"]) == "en")
    #expect(Messages.locale(for: []) == "en")
}
