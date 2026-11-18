import Foundation

struct NotesCalendarEvent: Codable, Equatable {
    let title: String
    var start: String? = nil
    var end: String? = nil
    var timeZone: String? = nil
    var location: String? = nil
}

struct NotesDateCandidate: Codable, Sendable {
    let id: Int
    let start: String
    let end: String?
    let timeZone: String
    let display: String
}

enum NotesCalendar {
    /// Dates come from the OS text detector, never from invented model fields.
    /// Require an explicit year and time before offering a calendar import.
    static func candidates(in evidence: String) -> [NotesDateCandidate] {
        let text = evidence.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        guard let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.date.rawValue) else { return [] }
        let value = text as NSString
        let clock = ISO8601DateFormatter()
        var dates: [NotesDateCandidate] = []
        for match in detector.matches(in: text, range: NSRange(location: 0, length: value.length)) {
            guard dates.count < 8, let date = match.date else { continue }
            let raw = value.substring(with: match.range)
            guard raw.range(of: #"\b20\d{2}\b"#, options: .regularExpression) != nil,
                  raw.range(of: #"(?i)(\d{1,2}:\d{2}|\d{1,2}\s*[ap]\.?m\.?)"#, options: .regularExpression) != nil else { continue }
            let zone = match.timeZone ?? .current
            let formatter = DateFormatter(); formatter.timeZone = zone; formatter.dateStyle = .medium; formatter.timeStyle = .short
            dates.append(NotesDateCandidate(id: dates.count, start: clock.string(from: date),
                end: match.duration > 0 ? clock.string(from: date.addingTimeInterval(match.duration)) : nil,
                timeZone: zone.identifier, display: formatter.string(from: date)))
        }
        return dates
    }

    static func write(_ event: NotesCalendarEvent, id: UUID, directory: URL, details: String = "") throws -> URL? {
        let parser = ISO8601DateFormatter()
        guard !event.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let start = event.start, let date = parser.date(from: start) else { return nil }
        let end = event.end.flatMap(parser.date)
        if event.end != nil && (end == nil || end! <= date) { return nil }
        let stamp = DateFormatter(); stamp.locale = Locale(identifier: "en_US_POSIX")
        stamp.timeZone = TimeZone(secondsFromGMT: 0); stamp.dateFormat = "yyyyMMdd'T'HHmmss'Z'"
        func escape(_ text: String) -> String {
            text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\r\n", with: "\n")
                .replacingOccurrences(of: "\r", with: "\n").replacingOccurrences(of: "\n", with: "\\n")
                .replacingOccurrences(of: ";", with: "\\;").replacingOccurrences(of: ",", with: "\\,")
        }
        var lines = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Lorca//Notes//EN", "CALSCALE:GREGORIAN", "BEGIN:VEVENT",
            "UID:\(id.uuidString)@lorca.local", "DTSTAMP:\(stamp.string(from: Date()))", "DTSTART:\(stamp.string(from: date))", "SUMMARY:\(escape(event.title))"]
        if let end { lines.append("DTEND:\(stamp.string(from: end))") }
        if let location = event.location, !location.isEmpty { lines.append("LOCATION:\(escape(location))") }
        if !details.isEmpty { lines.append("DESCRIPTION:\(escape(String(details.prefix(6000))))") }
        lines += ["END:VEVENT", "END:VCALENDAR"]
        // RFC 5545 folds at 75 UTF-8 octets without splitting a scalar.
        func fold(_ line: String) -> String {
            var output = "", current = ""
            for scalar in line.unicodeScalars {
                let part = String(scalar)
                if current.utf8.count + part.utf8.count > 75 { output += current + "\r\n"; current = " " }
                current += part
            }
            return output + current
        }
        let data = Data((lines.map(fold).joined(separator: "\r\n") + "\r\n").utf8)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent(id.uuidString + ".ics")
        try data.write(to: url, options: .atomic)
        return url
    }
}
