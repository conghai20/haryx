import Foundation

/// What a `haryx login` run printed on its pseudo-terminal, cleaned for display, plus what the
/// sign-in window needs from it: the sign-in link and whether the run waits for input.
///
/// Terminal output arrives in arbitrary chunks (an escape sequence can be split across two), so the
/// raw bytes are kept and the whole transcript is cleaned again on every append. Login output is a
/// few lines, so that costs nothing.
public struct LoginTranscript: Sendable, Equatable {
    public private(set) var raw = ""
    public private(set) var text = ""

    public init() {}

    public mutating func append(_ chunk: String) {
        raw += chunk
        text = Self.clean(raw)
    }

    /// The last sign-in link printed (`claude auth login` prints one when the browser may not open).
    public var url: URL? {
        let matches = text.matches(of: /https:\/\/[^\s<>"']+/)
        return matches.last.flatMap { URL(string: String($0.output)) }
    }

    /// Claude Code asks for the code the sign-in page shows when it cannot receive it by itself.
    public var awaitingCode: Bool { lastLine.localizedCaseInsensitiveContains("paste code") }

    /// A yes/no question (haryx asks before replacing an occupied slot).
    public var awaitingAnswer: Bool { lastLine.contains("[y/N]") }

    public var lastLine: String {
        text.split(separator: "\n").last.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
    }

    /// The line to show when the run failed: haryx's `Error:` line, else the last line printed.
    public var failureMessage: String {
        let lines = text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        if let error = lines.last(where: { $0.hasPrefix("Error:") }) {
            return String(error.dropFirst("Error:".count)).trimmingCharacters(in: .whitespaces)
        }
        return lines.last ?? "Sign-in did not finish."
    }

    /// The line haryx printed on success ("Signed in Account 3: …" / "Added Account 4: …").
    public var successMessage: String? {
        text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
            .last { $0.hasPrefix("Signed in") || $0.hasPrefix("Added") || $0.hasPrefix("Updated credentials") }
    }

    /// Terminal output → plain text: escape sequences dropped (OSC hyperlinks keep their visible
    /// text), CRLF → LF, a bare CR overwrites its line, other control characters dropped.
    public static func clean(_ raw: String) -> String {
        var out = ""
        var it = raw.unicodeScalars.makeIterator()
        var pending: Unicode.Scalar? = nil
        func next() -> Unicode.Scalar? {
            if let p = pending { pending = nil; return p }
            return it.next()
        }
        while let c = next() {
            switch c {
            case "\u{1B}":
                skipEscape(&it)
            case "\r":
                // A terminal sends "\r\r\n" (output post-processing on "\r\n"): runs of CR are one.
                var following = it.next()
                while following == "\r" { following = it.next() }
                guard let following else { break }
                if following == "\n" {
                    out.unicodeScalars.append("\n")
                } else {
                    // A bare CR returns to the line start: what follows overwrites the line.
                    if let lineStart = out.lastIndex(of: "\n") {
                        out = String(out[...lineStart])
                    } else {
                        out = ""
                    }
                    pending = following
                }
            case "\n", "\t":
                out.unicodeScalars.append(c)
            default:
                if c.value >= 0x20 && c.value != 0x7F { out.unicodeScalars.append(c) }
            }
        }
        return out
    }

    /// Skip one escape sequence after ESC: CSI (`ESC [ … final`), OSC (`ESC ] … BEL|ESC \`), or a
    /// two-character escape.
    private static func skipEscape(_ it: inout String.UnicodeScalarView.Iterator) {
        guard let kind = it.next() else { return }
        switch kind {
        case "[":
            while let c = it.next() {
                if (0x40...0x7E).contains(c.value) { return }
            }
        case "]":
            while let c = it.next() {
                if c == "\u{07}" { return }
                if c == "\u{1B}" { _ = it.next(); return }
            }
        default:
            return
        }
    }
}
