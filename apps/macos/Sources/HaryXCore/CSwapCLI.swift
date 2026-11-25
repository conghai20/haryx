import Foundation
import Synchronization

public enum CLIError: LocalizedError, Sendable {
    case notFound
    case failed(command: String, message: String)
    case decode(command: String, message: String)

    public var errorDescription: String? {
        switch self {
        case .notFound: "haryx not found (cswap works too). Install it with `brew install haryx` or `cargo install haryx`, or set its path in Preferences."
        case .failed(_, let message): message
        case .decode(let command, let message): "Unexpected output from `\(CSwapCLI.selectedFlavor.rawValue) \(command)`: \(message)"
        }
    }
}

public struct CLIResult: Sendable {
    public let status: Int32
    public let stdout: Data
    public let stderr: String
    public var text: String { String(decoding: stdout, as: UTF8.self) }
}

/// Which command-line backend the app drives. `haryx` is the drop-in Rust port of `cswap`
/// (same commands, flags and JSON), but each keeps its own store.
public enum CLIFlavor: String, Sendable, CaseIterable {
    case haryx, cswap

    /// The backend's data folder: haryx honours `HARYX_HOME`, else `~/Library/Application Support/HaryX`.
    public var storeRoot: String {
        switch self {
        case .haryx:
            if let custom = ProcessInfo.processInfo.environment["HARYX_HOME"], !custom.isEmpty {
                return (custom as NSString).expandingTildeInPath
            }
            return CSwapCLI.home + "/Library/Application Support/HaryX"
        case .cswap:
            return CSwapCLI.home + "/.claude-swap-backup"
        }
    }

    public var logFileName: String { self == .haryx ? "haryx.log" : "claude-swap.log" }

    /// Identify a backend by its executable name (`cswap*` is cswap; anything else is haryx).
    public static func of(_ executable: URL) -> CLIFlavor {
        executable.lastPathComponent.lowercased().hasPrefix("cswap") ? .cswap : .haryx
    }
}

/// Thin async wrapper over the `haryx` (or legacy `cswap`) executable. Arguments go straight to
/// `Process` (never through a shell), and secrets are passed on stdin.
public struct CSwapCLI: Sendable {
    public let executable: URL

    public init(executable: URL) {
        self.executable = executable
        Self.selected.withLock { $0 = CLIFlavor.of(executable) }
    }

    public var flavor: CLIFlavor { CLIFlavor.of(executable) }

    public static let home = FileManager.default.homeDirectoryForCurrentUser.path

    private static let selected = Mutex<CLIFlavor>(.haryx)

    /// The flavor of the most recently located backend (haryx until one is found).
    public static var selectedFlavor: CLIFlavor { selected.withLock { $0 } }

    /// The selected backend's data folder (sequence.json, mappings.json, sessions/, the log).
    public static var backupRoot: String { selectedFlavor.storeRoot }

    /// GUI apps get a bare PATH, so look where uv/pipx/brew/cargo install tools, and finally in
    /// the app bundle (`Contents/Resources/bin`, the DMG build's bundled haryx) so a user-installed
    /// copy always wins and stays upgradable.
    public static let searchPaths = [
        home + "/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", home + "/.cargo/bin", "/usr/bin",
    ] + (Bundle.main.resourceURL.map { [$0.appendingPathComponent("bin").path] } ?? [])

    /// The override when given; else `haryx` on any search path, then `cswap` as a fallback.
    public static func locate(override: String?, searchPaths: [String] = searchPaths) -> URL? {
        let fm = FileManager.default
        if let override, !override.isEmpty {
            let path = (override as NSString).expandingTildeInPath
            return fm.isExecutableFile(atPath: path) ? URL(fileURLWithPath: path) : nil
        }
        let candidates = CLIFlavor.allCases.flatMap { flavor in searchPaths.map { $0 + "/" + flavor.rawValue } }
        return candidates.first(where: fm.isExecutableFile(atPath:)).map(URL.init(fileURLWithPath:))
    }

    public static var environment: [String: String] {
        var env = ProcessInfo.processInfo.environment
        let existing = env["PATH"].map { [$0] } ?? []
        let shell = loginShellPath.map { [$0] } ?? []
        env["PATH"] = (searchPaths + shell + existing).joined(separator: ":")
        env["NO_COLOR"] = "1"
        env["TERM"] = "dumb"
        return env
    }

    /// Runs the CLI to completion. `stdin` answers y/N prompts or carries a token; when nil,
    /// stdin is /dev/null so a prompt can never hang the app.
    public func run(_ args: [String], stdin: String? = nil, env extra: [String: String] = [:]) async throws -> CLIResult {
        let process = Process()
        process.executableURL = executable
        process.arguments = args
        process.environment = Self.environment.merging(extra) { _, new in new }
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
                    // The CLI may exit before reading (e.g. refuses up front): EPIPE must not crash us.
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

    /// Runs and throws on non-zero exit, with the CLI's own error line as the message.
    @discardableResult
    public func checked(_ args: [String], stdin: String? = nil, env: [String: String] = [:]) async throws -> String {
        let result = try await run(args, stdin: stdin, env: env)
        guard result.status == 0 else {
            throw CLIError.failed(command: args.joined(separator: " "), message: Self.errorMessage(result))
        }
        return result.text
    }

    public func json<T: Decodable>(_ type: T.Type, _ args: [String], decoder: JSONDecoder = CSwapJSON.decoder,
                                   env: [String: String] = [:]) async throws -> T {
        let result = try await run(args, env: env)
        do {
            return try decoder.decode(T.self, from: result.stdout)
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
        return error.map { $0.replacingOccurrences(of: "Error: ", with: "") } ?? "\(CSwapCLI.selectedFlavor.rawValue) exited with status \(r.status)"
    }
}

private final class DataBox: @unchecked Sendable {
    var data = Data()
}
