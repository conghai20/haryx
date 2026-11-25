import Foundation

/// `haryx ai doctor --json`: whether Claude Code is installed and signed in, and how to fix it.
public struct ClaudeSetup: Decodable, Sendable, Equatable {
    public struct Problem: Decodable, Sendable, Equatable {
        public enum Kind: Sendable, Equatable { case claudeMissing, notSignedIn, other }

        public let code: String
        public let message: String
        public let fix: String

        public var kind: Kind {
            switch code {
            case "claude_missing": .claudeMissing
            case "not_signed_in": .notSignedIn
            default: .other
            }
        }
    }

    public let ready: Bool
    public let claude: String?
    public let email: String?
    public let installCommand: String
    public let problems: [Problem]

    /// The problem to fix first (signing in needs Claude Code installed).
    public var next: Problem? { problems.first }
}
