import Foundation

public enum CLIError: LocalizedError, Sendable {
    case notFound
    case failed(command: String, message: String)
    case decode(command: String, message: String)

    public var errorDescription: String? {
        switch self {
        case .notFound: "cswap not found. Install it with `uv tool install claude-swap`, or set its path in Preferences."
        case .failed(_, let message): message
        case .decode(let command, let message): "Unexpected output from `cswap \(command)`: \(message)"
        }
    }
}

public struct CLIResult: Sendable {
    public let status: Int32
    public let stdout: Data
    public let stderr: String
    public var text: String { String(decoding: stdout, as: UTF8.self) }
}

/// Thin async wrapper over the `cswap` executable. Arguments go straight to `Process`
/// (never through a shell), and secrets are passed on stdin.
public struct CSwapCLI: Sendable {
    public let executable: URL

    public init(executable: URL) { self.executable = executable }

    public static let home = FileManager.default.homeDirectoryForCurrentUser.path
    public static let backupRoot = home + "/.claude-swap-backup"

    /// GUI apps get a bare PATH, so look where uv/pipx/brew install tools.
    public static let searchPaths = [
        home + "/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", home + "/.cargo/bin", "/usr/bin",
    ]

    public static func locate(override: String?) -> URL? {
        let fm = FileManager.default
        if let override, !override.isEmpty {
            let path = (override as NSString).expandingTildeInPath
            return fm.isExecutableFile(atPath: path) ? URL(fileURLWithPath: path) : nil
        }
        return searchPaths.map { $0 + "/cswap" }.first(where: fm.isExecutableFile(atPath:)).map(URL.init(fileURLWithPath:))
    }

    public static var environment: [String: String] {
        var env = ProcessInfo.processInfo.environment
        let existing = env["PATH"].map { [$0] } ?? []
        env["PATH"] = (searchPaths + existing).joined(separator: ":")
        env["NO_COLOR"] = "1"
        env["TERM"] = "dumb"
        return env
    }

    /// Runs cswap to completion. `stdin` answers y/N prompts or carries a token; when nil,
    /// stdin is /dev/null so a prompt can never hang the app.
    public func run(_ args: [String], stdin: String? = nil) async throws -> CLIResult {
        let process = Process()
        process.executableURL = executable
        process.arguments = args
        process.environment = Self.environment
        let out = Pipe(), err = Pipe()
        process.standardOutput = out
        process.standardError = err
        let input = stdin.map { _ in Pipe() }
        process.standardInput = input ?? FileHandle.nullDevice

        return try await withCheckedThrowingContinuation { cont in
            DispatchQueue.global(qos: .userInitiated).async {
                do {
                    try process.run()
                } catch {
                    cont.resume(throwing: CLIError.failed(command: args.joined(separator: " "), message: error.localizedDescription))
                    return
                }
                if let input, let stdin {
                    // cswap may exit before reading (e.g. refuses up front): EPIPE must not crash us.
                    try? input.fileHandleForWriting.write(contentsOf: Data(stdin.utf8))
                    try? input.fileHandleForWriting.close()
                }
                // Drain stderr concurrently so a full pipe can't deadlock the child.
                let errBox = DataBox()
                let group = DispatchGroup()
                group.enter()
                DispatchQueue.global().async {
                    errBox.data = err.fileHandleForReading.readDataToEndOfFile()
                    group.leave()
                }
                let outData = out.fileHandleForReading.readDataToEndOfFile()
                group.wait()
                process.waitUntilExit()
                cont.resume(returning: CLIResult(status: process.terminationStatus, stdout: outData,
                                                 stderr: String(decoding: errBox.data, as: UTF8.self)))
            }
        }
    }

    /// Runs and throws on non-zero exit, with cswap's own error line as the message.
    @discardableResult
    public func checked(_ args: [String], stdin: String? = nil) async throws -> String {
        let result = try await run(args, stdin: stdin)
        guard result.status == 0 else {
            throw CLIError.failed(command: args.joined(separator: " "), message: Self.errorMessage(result))
        }
        return result.text
    }

    public func json<T: Decodable>(_ type: T.Type, _ args: [String]) async throws -> T {
        let result = try await run(args)
        do {
            return try CSwapJSON.decoder.decode(T.self, from: result.stdout)
        } catch {
            if result.status != 0 { throw CLIError.failed(command: args.joined(separator: " "), message: Self.errorMessage(result)) }
            throw CLIError.decode(command: args.joined(separator: " "), message: String(describing: error))
        }
    }

    /// Prefers the JSON error envelope, then the last `Error:` line on stderr.
    static func errorMessage(_ r: CLIResult) -> String {
        if let obj = try? JSONSerialization.jsonObject(with: r.stdout) as? [String: Any],
           let err = obj["error"] as? [String: Any], let msg = err["message"] as? String {
            return msg
        }
        let lines = (r.stderr + "\n" + r.text).split(separator: "\n").map(String.init)
        let error = lines.last(where: { $0.hasPrefix("Error:") }) ?? lines.last(where: { !$0.isEmpty })
        return error.map { $0.replacingOccurrences(of: "Error: ", with: "") } ?? "cswap exited with status \(r.status)"
    }
}

private final class DataBox: @unchecked Sendable {
    var data = Data()
}
