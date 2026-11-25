import Foundation

/// The PATH your terminal has. An app opened from Finder gets a bare one, so a `claude` installed
/// through npm with nvm, volta or bun (set up in the shell's rc files) is invisible to the CLI
/// unless the app asks the shell.
extension CSwapCLI {
    private static let pathStart = "__HARYX_PATH__"
    private static let pathEnd = "__HARYX_END__"
    /// An rc file that waits for input must not hang the app.
    private static let shellTimeout: TimeInterval = 3

    /// Read once per launch: an interactive login shell, so both profile and rc files run.
    public static let loginShellPath: String? = readLoginShellPath()

    /// The PATH between the markers; rc files may print anything around it.
    static func extractShellPath(_ output: String) -> String? {
        guard let start = output.range(of: pathStart),
              let end = output.range(of: pathEnd, range: start.upperBound..<output.endIndex) else { return nil }
        let path = String(output[start.upperBound..<end.lowerBound])
        return path.isEmpty ? nil : path
    }

    private static func readLoginShellPath() -> String? {
        let shell = ProcessInfo.processInfo.environment["SHELL"].flatMap { $0.isEmpty ? nil : $0 } ?? "/bin/zsh"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: shell)
        process.arguments = ["-ilc", "printf '\(pathStart)%s\(pathEnd)' \"$PATH\""]
        let out = Pipe()
        process.standardOutput = out
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        guard (try? process.run()) != nil else { return nil }
        let deadline = Date().addingTimeInterval(shellTimeout)
        while process.isRunning, Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
        if process.isRunning {
            process.terminate()
            return nil
        }
        return extractShellPath(String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self))
    }
}
