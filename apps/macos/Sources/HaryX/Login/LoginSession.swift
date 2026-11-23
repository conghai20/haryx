import Darwin
import Foundation
import HaryXCore
import Observation

/// One `haryx ai login` run in the background on a pseudo-terminal the app owns.
///
/// Claude Code's sign-in expects a terminal (it reads a pasted code from it), so the CLI gets a
/// pty instead of pipes: its output is shown in the sign-in window, and whatever the person types
/// or pastes there is written back to it.
@MainActor @Observable
final class LoginSession: Identifiable {
    enum State: Equatable {
        case running
        case succeeded(String)
        case failed(String)
        case cancelled

        var isRunning: Bool { self == .running }
    }

    /// Wide enough that the sign-in link is never wrapped across lines.
    private static let columns: UInt16 = 1000
    private static let rows: UInt16 = 50

    let id = UUID()
    let title: String
    /// The account being signed in again (`Account.identityKey`); `nil` for a new account.
    let identityKey: String?
    private(set) var transcript = LoginTranscript()
    private(set) var state = State.running
    var onFinish: ((LoginSession) -> Void)?

    @ObservationIgnored private var process: Process?
    @ObservationIgnored private var masterFD: Int32 = -1
    @ObservationIgnored private var reader: DispatchSourceRead?

    init(title: String, identityKey: String?) {
        self.title = title
        self.identityKey = identityKey
    }

    func start(executable: URL, arguments: [String]) throws {
        var master: Int32 = -1, slave: Int32 = -1
        var size = winsize(ws_row: Self.rows, ws_col: Self.columns, ws_xpixel: 0, ws_ypixel: 0)
        guard openpty(&master, &slave, nil, nil, &size) == 0 else {
            throw CLIError.failed(command: "login", message: "Could not open a terminal for the sign-in: \(String(cString: strerror(errno)))")
        }
        let slaveHandle = FileHandle(fileDescriptor: slave, closeOnDealloc: true)
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        var env = CSwapCLI.environment
        env["TERM"] = "xterm-256color"
        process.environment = env
        process.standardInput = slaveHandle
        process.standardOutput = slaveHandle
        process.standardError = slaveHandle
        process.terminationHandler = { [weak self] proc in
            let status = proc.terminationStatus
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.processExited(status) } }
        }
        do {
            try process.run()
        } catch {
            close(master)
            throw CLIError.failed(command: "login", message: error.localizedDescription)
        }
        try? slaveHandle.close() // the child holds its own copy; EOF on the master then means it exited
        self.process = process
        masterFD = master
        startReading(master)
    }

    /// Type a line into the run (a pasted code, or a y/n answer), as Return would send it.
    func send(_ line: String) {
        guard state.isRunning, masterFD >= 0 else { return }
        let bytes = Array((line.trimmingCharacters(in: .newlines) + "\r").utf8)
        _ = bytes.withUnsafeBufferPointer { write(masterFD, $0.baseAddress, $0.count) }
    }

    func cancel() {
        guard state.isRunning else { return }
        state = .cancelled
        process?.terminate()
    }

    private func startReading(_ fd: Int32) {
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: .main)
        source.setEventHandler { [weak self] in
            var buffer = [UInt8](repeating: 0, count: 4096)
            let n = read(fd, &buffer, buffer.count)
            guard n > 0 else { source.cancel(); return } // EOF/EIO: the child closed the terminal
            let chunk = String(decoding: buffer[..<n], as: UTF8.self)
            MainActor.assumeIsolated { self?.transcript.append(chunk) }
        }
        source.setCancelHandler { close(fd) }
        source.resume()
        reader = source
    }

    private func processExited(_ status: Int32) {
        // Let the last output drain before judging it.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
            MainActor.assumeIsolated { self?.finish(status) }
        }
    }

    private func finish(_ status: Int32) {
        reader?.cancel()
        reader = nil
        masterFD = -1
        if state.isRunning {
            state = status == 0
                ? .succeeded(transcript.successMessage ?? "Nothing was changed.")
                : .failed(transcript.failureMessage)
        }
        onFinish?(self)
    }
}
