import Foundation

/// The app's words. Keys are shared with the web UI's catalogue where the meaning is the same
/// (`patient-administration-system-ui-with-svelte/src/lib/messages/`); app-only keys start `mobile.`.
public enum Messages {
    /// The locales with a catalogue. English is the source; the others are first drafts awaiting fluent review.
    public static let locales = ["en", "cy", "ar", "de", "es", "fr", "hi", "zh"]

    /// The text for `key` in the person's preferred language that has a catalogue, falling back to English, then to the key.
    /// The locale is chosen here, not by the app bundle, so a host app need not list the package's languages.
    public static func text(_ key: String) -> String {
        text(key, locale: current) ?? text(key, locale: "en") ?? key
    }

    /// The catalogue locale for `preferences` (BCP 47 tags, most preferred first): the first whose language has a catalogue.
    public static func locale(for preferences: [String]) -> String {
        for tag in preferences {
            let language = Locale(identifier: tag).language.languageCode?.identifier ?? tag
            if locales.contains(language) { return language }
        }
        return "en"
    }

    static let current = locale(for: Locale.preferredLanguages)

    /// The locale dates are written in: the catalogue's language, so the words around a date and the date agree.
    public static var dateLocale: Locale { Locale(identifier: "\(current)_GB") }

    /// The text for `key` in one locale's catalogue, or nil when that catalogue lacks it.
    public static func text(_ key: String, locale: String) -> String? {
        catalogue(locale)?[key]
    }

    /// One locale's whole catalogue.
    public static func catalogue(_ locale: String) -> [String: String]? {
        guard let url = Bundle.module.url(forResource: "Localizable", withExtension: "strings", subdirectory: nil,
                                          localization: locale)
        else { return nil }
        return NSDictionary(contentsOf: url) as? [String: String]
    }
}
