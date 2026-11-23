import Foundation

/// The PAS success envelope, `{"data": ...}` (`ApiResponse` in spec/openapi.json). A failure is an RFC 9457 problem and is
/// never decoded for display: its text is the server's, and the app shows only its own words (FR-228).
struct Envelope<T: Decodable>: Decodable {
    let data: T
}

public enum PASDecoding {
    /// A decoder for PAS JSON: RFC 3339 date-times with or without fractional seconds, as chrono writes them.
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { d in
            let text = try d.singleValueContainer().decode(String.self)
            if let date = parse(text) { return date }
            throw DecodingError.dataCorrupted(.init(codingPath: d.codingPath, debugDescription: "not an RFC 3339 date-time"))
        }
        return decoder
    }

    static func parse(_ text: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: text) { return date }
        let whole = ISO8601DateFormatter()
        whole.formatOptions = [.withInternetDateTime]
        return whole.date(from: text)
    }

    /// The appointments inside an envelope.
    public static func appointments(from data: Data) throws -> [Appointment] {
        try decoder().decode(Envelope<[Appointment]>.self, from: data).data
    }
}
